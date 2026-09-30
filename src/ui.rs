//! Drawing the application state into a window.
//!
//! booknook draws in immediate mode, which is how egui is built: every frame
//! reads the current `App` and paints the whole window again, sheet,
//! sidebar, and status bar included. A few functions here take `&mut App`,
//! because they need to clamp a value like the current page once the true
//! page count is known, but nothing here changes application state beyond
//! that kind of bookkeeping. Turning input into state changes is the
//! `events` module's job; `Reader::frame` hands it each frame's keys before
//! drawing anything.
//!
//! The one thing kept from frame to frame is the laid-out document. Wrapping
//! a whole book word by word takes long enough to feel on a keypress, so the
//! result is cached and redone only when something that shapes it changes:
//! the document, the column, the page height, the spacing, or the display's
//! pixel density. A page turn or a theme switch repaints from the cache.

use std::path::PathBuf;

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, Frame, Label, Layout, Margin, Painter, Pos2, Rect, RichText,
    ScrollArea, Stroke, pos2, vec2,
};

use crate::anim::{self, Direction, Turn};
use crate::app::{App, Focus};
use crate::browser::is_readable;
use crate::events;
use crate::text::{Span, Style};
use crate::theme::Theme;
use crate::typeset::{self, CHROME, Faces, Typesetter};
use crate::wrap::{self, Pages, RowKind, Spacing};

/// How wide the sidebar is, in points: room for the file browser, and for
/// the contents list to show most headings on a line or two.
const SIDEBAR_WIDTH: f32 = 300.0;

/// How tall the status bar is.
const STATUS_HEIGHT: f32 = 30.0;

/// How far the sheet stands off the edges of the reading area, so it reads
/// as paper lying on something rather than as the window itself.
const OUTER: f32 = 20.0;

/// How far the shading either side of a spread's spine reaches.
const SPINE_SHADE: f32 = 24.0;

/// The column the sidebar's `›` cursor sits in, ahead of each entry.
const CURSOR_WIDTH: f32 = 14.0;

/// The sidebar's small print: settings and the key legend.
const SMALL: f32 = 13.0;

/// The reader's keys, as the sidebar lists them. Pane-specific keys for the
/// sidebar itself are in the status bar while it has focus.
const LEGEND: [(&str, &str); 9] = [
    ("→  space", "next page"),
    ("←  backspace", "previous page"),
    ("g  G", "first or last page"),
    ("-  +", "column width"),
    ("[  ]", "line spacing"),
    ("{  }", "paragraph spacing"),
    (",  .", "pan wide code"),
    ("Ctrl -  +", "text size"),
    ("t  a  r", "theme, page turn, reload"),
];

/// Open the window and read until it is closed.
pub(crate) fn run(app: App) -> anyhow::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(window_title(&app))
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([480.0, 360.0]),
        ..Default::default()
    };
    eframe::run_native(
        "booknook",
        options,
        Box::new(move |cc| {
            let faces = typeset::install(&cc.egui_ctx);
            cc.egui_ctx.set_zoom_factor(app.zoom);
            Ok(Box::new(Reader::new(app, faces)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("could not open the window: {e}"))
}

/// The window: the app's state, plus what drawing it needs to remember
/// between frames.
pub(crate) struct Reader {
    app: App,
    faces: Faces,
    cache: Option<Cache>,
    turn: Option<Turn>,
    /// The window title and the theme egui's own visuals were last set for,
    /// so each is sent to the window only when it changes.
    title: String,
    visuals_for: Option<&'static str>,
    /// Where each sidebar list's cursor was last frame, so a list scrolls to
    /// its cursor only when the cursor moves and otherwise stays put.
    files_cursor: Option<(PathBuf, usize)>,
    toc_cursor: Option<usize>,
    /// The rect the central panel actually got, so a test can check it
    /// against the one `document_area` predicted.
    #[cfg(test)]
    drawn_area: Option<Rect>,
}

/// The last layout, and what it was made for.
struct Cache {
    key: CacheKey,
    pages: Pages,
}

#[derive(Clone, Copy, PartialEq)]
struct CacheKey {
    generation: u64,
    width: u32,
    height: u32,
    spacing: Spacing,
    pixels_per_point: u32,
}

/// What `prepare` worked out for this frame, for the painting that follows.
struct Prepared {
    geometry: Geometry,
    /// How many pages the document runs to at the current size.
    count: usize,
    /// How far code is panned, in points, and how wide the `‹` that marks a
    /// panned line's cut edge is.
    pan: f32,
    marker: f32,
}

impl eframe::App for Reader {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.frame(ui);
    }

    /// Save the session on the way out, however the window was closed. The
    /// current page is folded in first. A failure to save is ignored: a lost
    /// bookmark is not worth an error at exit.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.app.remember_position();
        let _ = self.app.to_session().save();
    }
}

impl Reader {
    pub(crate) fn new(app: App, faces: Faces) -> Self {
        Reader {
            app,
            faces,
            cache: None,
            turn: None,
            title: String::new(),
            visuals_for: None,
            files_cursor: None,
            toc_cursor: None,
            #[cfg(test)]
            drawn_area: None,
        }
    }

    /// One frame: take the keys, work out the page, then draw.
    fn frame(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|input| input.time);
        let (page_before, generation_before) = (self.app.page, self.app.generation);

        events::handle_input(&ctx, &mut self.app);
        if self.app.quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.app.zoom = ctx.zoom_factor();
        self.sync_window(&ctx);

        // The document is worked out before anything is drawn, on purpose.
        // Laying it out is what reveals how many pages there are and which
        // heading the current page falls under, and the status bar and the
        // sidebar both want those answers. egui wants the side panels added
        // before the central one, so the reading area's rect is predicted
        // from the same numbers the panels are given.
        let ts = Typesetter::new(self.faces, self.app.spacing);
        let sidebar = self.app.sidebar_visible();
        let area = document_area(ui.max_rect(), sidebar);
        let prepared = self.prepare(&ctx, area, &ts);
        self.follow_turn(&ctx, prepared.is_some(), page_before, generation_before, now);

        let theme = self.app.theme();
        egui::Panel::bottom("status")
            .exact_size(STATUS_HEIGHT)
            .show_separator_line(false)
            .frame(Frame::NONE.fill(theme.bg).inner_margin(Margin::symmetric(16, 0)))
            .show(ui, |ui| self.status_bar(ui, &ts, prepared.as_ref()));
        if sidebar {
            egui::Panel::left("sidebar")
                .exact_size(SIDEBAR_WIDTH)
                .resizable(false)
                .show_separator_line(false)
                .frame(Frame::NONE.fill(theme.bg).inner_margin(Margin { left: 12, right: 12, top: 12, bottom: 4 }))
                .show(ui, |ui| self.sidebar(ui, &ts));
        }
        egui::CentralPanel::default()
            .frame(Frame::NONE.fill(theme.bg))
            .show(ui, |ui| self.document(ui, prepared.as_ref(), &ts, now));
    }

    /// Keep the window's title and egui's own colors in step with the
    /// document and the theme.
    fn sync_window(&mut self, ctx: &egui::Context) {
        let title = window_title(&self.app);
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        let theme = self.app.theme();
        if self.visuals_for != Some(theme.name) {
            let mut visuals = if is_dark(theme) { egui::Visuals::dark() } else { egui::Visuals::light() };
            visuals.panel_fill = theme.bg;
            visuals.window_fill = theme.bg;
            visuals.override_text_color = Some(theme.fg);
            ctx.set_visuals(visuals);
            self.visuals_for = Some(theme.name);
        }
    }

    /// Lay the document out if it needs it, then settle this frame's page:
    /// resolve a pending jump, clamp the page to the real count, keep a
    /// spread on an even page, clamp the pan, and work out which heading is
    /// in view. Returns `None` when there is nothing to read.
    fn prepare(&mut self, ctx: &egui::Context, area: Rect, ts: &Typesetter) -> Option<Prepared> {
        if self.app.blocks.is_empty() {
            self.app.spread = false;
            self.app.active_heading = None;
            return None;
        }

        let (char_width, mono_width) = ctx.fonts_mut(|fonts| (ts.char_width(fonts), ts.mono_width(fonts).max(1.0)));
        let geometry = Geometry::new(area, self.app.page_width as f32 * char_width, ts.body_row());
        self.app.spread = geometry.right.is_some();

        let column = geometry.left;
        let key = CacheKey {
            generation: self.app.generation,
            width: column.width().to_bits(),
            height: column.height().to_bits(),
            spacing: self.app.spacing,
            pixels_per_point: ctx.pixels_per_point().to_bits(),
        };
        if self.cache.as_ref().is_none_or(|cache| cache.key != key) {
            let (blocks, headings) = (&self.app.blocks, &self.app.headings);
            let pages = ctx.fonts_mut(|fonts| {
                let mut metrics = ts.metrics(fonts);
                wrap::paginate(wrap::layout(blocks, headings, column.width(), &mut metrics), column.height())
            });
            self.cache = Some(Cache { key, pages });
        }
        let pages = &self.cache.as_ref()?.pages;

        // Sideways pan is clamped to the widest code line's real overflow,
        // the same draw-time clamping `page` gets: the event handler asks for
        // more and the one place that knows the bound trims it. The bound
        // includes the `‹` that takes the left edge once a line is panned, so
        // the last character comes fully into view.
        let overflow = (pages.code_width - column.width()).max(0.0);
        let max_pan = if overflow > 0.0 { overflow + mono_width } else { 0.0 };
        self.app.pan = self.app.pan.min((max_pan / mono_width).ceil() as u16);
        let pan = (self.app.pan as f32 * mono_width).min(max_pan);

        // A jump requested from the contents list becomes a real page here,
        // where the layout that decides it is at hand, and fires once.
        let count = pages.count();
        if let Some(block) = self.app.pending_jump.take() {
            self.app.page = page_number(pages.page_of_block(block));
        }
        self.app.page = self.app.page.min(page_number(count - 1));
        // The left page of a spread is always even, as in a printed book.
        if self.app.spread && self.app.page % 2 == 1 {
            self.app.page -= 1;
        }

        // The active heading is the last one that starts on or before the
        // last page in view. Headings run in document order, so counting the
        // ones already reached is enough.
        let last_visible = self.app.page as usize + usize::from(self.app.spread);
        self.app.active_heading = self
            .app
            .headings
            .iter()
            .take_while(|heading| pages.page_of_block(heading.block) <= last_visible)
            .count()
            .checked_sub(1);

        Some(Prepared { geometry, count, pan, marker: mono_width })
    }

    /// Start a turn animation if a turn key moved the page this frame, and
    /// end one that has finished or no longer makes sense.
    fn follow_turn(&mut self, ctx: &egui::Context, open: bool, from: u16, generation: u64, now: f64) {
        let turned = std::mem::take(&mut self.app.page_turn);
        if turned && self.app.animate && open && self.app.page != from && self.app.generation == generation {
            let dir = if self.app.page > from { Direction::Forward } else { Direction::Back };
            self.turn = Some(Turn::new(from, dir, now, generation));
        }
        // A turn ends when its time is up, and early if the document changed
        // under it, animation was switched off, or the sidebar came back and
        // moved the sheet.
        let stale = self.turn.is_some_and(|turn| {
            turn.done(now) || turn.generation != self.app.generation || !self.app.animate || self.app.sidebar_visible()
        });
        if stale {
            self.turn = None;
        }
        if self.turn.is_some() {
            ctx.request_repaint();
        }
    }

    fn document(&mut self, ui: &mut egui::Ui, prepared: Option<&Prepared>, ts: &Typesetter, now: f64) {
        #[cfg(test)]
        {
            self.drawn_area = Some(ui.max_rect());
        }
        let theme = self.app.theme();
        let painter = ui.painter();
        let (Some(prepared), Some(cache)) = (prepared, &self.cache) else {
            let area = ui.max_rect();
            painter.text(
                pos2(area.center().x, area.top() + area.height() / 3.0),
                Align2::CENTER_CENTER,
                "Select a markdown file to begin reading.",
                ts.chrome(CHROME + 1.0, false, true),
                theme.muted,
            );
            return;
        };

        let sheet = prepared.geometry.sheet;
        let shadow = egui::epaint::Shadow {
            offset: [0, 2],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(if is_dark(theme) { 90 } else { 34 }),
        };
        painter.add(shadow.as_shape(sheet, CornerRadius::same(3)));

        let leaf = Leaf { pages: &cache.pages, prepared, ts, theme };
        match self.turn {
            Some(turn) => {
                // The destination is painted whole, then the outgoing page
                // over the part of the sheet the fold has not yet crossed.
                let progress = turn.progress(now);
                leaf.paint(painter, self.app.page);
                let outgoing = painter.with_clip_rect(turn.old_region(sheet, progress).intersect(painter.clip_rect()));
                leaf.paint(&outgoing, turn.from);
                turn.paint_fold(painter, sheet, progress, theme.page);
            }
            None => leaf.paint(painter, self.app.page),
        }
    }

    /// While reading, the bar says where you are and nothing else. The
    /// settings and the key legend used to be live here, and most of those
    /// numbers change as you type, which puts motion at the bottom edge of
    /// vision for the whole session. Peripheral motion takes attention on its
    /// own, with no help from color. The sidebar shows the full legend, and
    /// it is one Tab away. Pan is the exception that stays: it reports a
    /// state you can otherwise only infer from the code having slid sideways.
    fn status_bar(&self, ui: &mut egui::Ui, ts: &Typesetter, prepared: Option<&Prepared>) {
        let theme = self.app.theme();
        let (text, color) = match &self.app.notice {
            Some(notice) => (notice.clone(), theme.fg),
            None => {
                let text = match self.app.focus {
                    Focus::Files => {
                        let next = if self.app.headings.is_empty() { "reader" } else { "contents" };
                        format!("↑/↓ move · →/l open · ←/h up · Tab {next} · t {} · q quit", theme.name)
                    }
                    Focus::Toc => {
                        format!("↑/↓ move · →/l/Enter jump · ←/h files · Tab reader · t {} · q quit", theme.name)
                    }
                    Focus::Document => {
                        let count = prepared.map_or(1, |p| p.count);
                        let position = if self.app.spread && (self.app.page as usize) + 1 < count {
                            format!("pages {}-{}", self.app.page + 1, self.app.page + 2)
                        } else {
                            format!("page {}", self.app.page + 1)
                        };
                        let pan = if self.app.pan > 0 { format!("   pan {}", self.app.pan) } else { String::new() };
                        format!("{position} / {count}{pan}")
                    }
                };
                (text, theme.muted)
            }
        };
        ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.add(Label::new(RichText::new(text).font(ts.chrome(14.0, false, false)).color(color)).truncate().selectable(false));
        });
    }

    /// The sidebar stacks three boxes: the file browser, the open document's
    /// contents once it has headings, and the reading settings with the key
    /// legend when the window is tall enough to spare the room. The contents
    /// box is sized to what its headings need, capped at two thirds of what
    /// the browser shares with it, and the browser takes the rest.
    fn sidebar(&mut self, ui: &mut egui::Ui, ts: &Typesetter) {
        let height = ui.available_height();
        let width = ui.available_width();
        let gap = 10.0;
        let spacing = ui.spacing().item_spacing.y;
        let list_font = ts.chrome(CHROME, false, false);
        let small_font = ts.chrome(SMALL, false, false);
        let (list_row, small_row) = ui.fonts_mut(|fonts| (fonts.row_height(&list_font), fonts.row_height(&small_font)));
        // A box's own height beyond its contents: frame margins and border,
        // plus the title and the space under it.
        let chrome = 18.0 + list_row + spacing + 4.0;

        let legend_rows = (LEGEND.len() + 2) as f32;
        let legend_h = chrome + legend_rows * (small_row + spacing) + 6.0;
        let show_legend = height - legend_h - gap >= 7.0 * (list_row + spacing) + 2.0 * chrome;
        let shared = height - if show_legend { legend_h + gap } else { 0.0 };

        let toc_h = if self.app.headings.is_empty() {
            0.0
        } else {
            let text_width = width - 22.0 - CURSOR_WIDTH;
            let needed: f32 = ui.fonts_mut(|fonts| {
                self.app
                    .headings
                    .iter()
                    .map(|heading| {
                        let indent = toc_indent(heading.level);
                        let galley = fonts.layout(heading.text.clone(), list_font.clone(), Color32::WHITE, text_width - indent);
                        galley.size().y + spacing
                    })
                    .sum()
            });
            (chrome + needed).min((shared - gap) * 2.0 / 3.0).max(chrome + 3.0 * (list_row + spacing))
        };
        let files_h = (shared - if toc_h > 0.0 { toc_h + gap } else { 0.0 }).max(chrome + list_row);

        let theme = self.app.theme();
        let label = self.app.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.app.dir.display().to_string());
        section(ui, ts, theme, &label, matches!(self.app.focus, Focus::Files), files_h, |ui| self.files(ui, ts));
        if toc_h > 0.0 {
            ui.add_space(gap - spacing);
            section(ui, ts, theme, "Contents", matches!(self.app.focus, Focus::Toc), toc_h, |ui| self.contents(ui, ts));
        }
        if show_legend {
            ui.add_space(gap - spacing);
            section(ui, ts, theme, "Reading", false, legend_h, |ui| self.legend(ui, ts));
        }
    }

    fn files(&mut self, ui: &mut egui::Ui, ts: &Typesetter) {
        let theme = self.app.theme();
        let focused = matches!(self.app.focus, Focus::Files);
        let cursor = (self.app.dir.clone(), self.app.selected);
        let moved = focused && self.files_cursor.as_ref() != Some(&cursor);

        ScrollArea::vertical()
            .id_salt("files")
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                // ".." is a hint, not a selectable row. `h` and Backspace
                // already go up a directory, so it does not need its own
                // slot in `selected`'s index space.
                if self.app.dir.parent().is_some() {
                    list_row(ui, ts, theme, false, RichText::new("..").font(ts.chrome(CHROME, false, false)).color(theme.muted), false, 0.0);
                }
                for (i, entry) in self.app.entries.iter().enumerate() {
                    let selected = focused && i == self.app.selected;
                    let name = if entry.is_dir { format!("{}/", entry.name) } else { entry.name.clone() };
                    let color = if entry.is_dir {
                        theme.link
                    } else if is_readable(&entry.path) {
                        theme.fg
                    } else {
                        theme.muted
                    };
                    let text = RichText::new(name).font(ts.chrome(CHROME, selected, false)).color(color);
                    let response = list_row(ui, ts, theme, selected, text, false, 0.0);
                    if selected && moved {
                        response.scroll_to_me(Some(Align::Center));
                    }
                }
            });
        self.files_cursor = focused.then_some(cursor);
    }

    /// The open document's headings, as a jump list. The entry the reader is
    /// currently under is drawn in the heading color, so the list doubles as
    /// a "you are here" marker, not just a way to navigate. When the pane
    /// has focus, a cursor marks the entry a jump would land on. A heading
    /// too long for the pane wraps under itself rather than being cut off.
    fn contents(&mut self, ui: &mut egui::Ui, ts: &Typesetter) {
        let theme = self.app.theme();
        let focused = matches!(self.app.focus, Focus::Toc);
        // With focus, the list follows the cursor; without it, the list keeps
        // the heading being read in view as the pages turn.
        let target = if focused { Some(self.app.toc_selected) } else { self.app.active_heading };
        let moved = target != self.toc_cursor;

        ScrollArea::vertical()
            .id_salt("contents")
            .auto_shrink([false, false])
            .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
            .show(ui, |ui| {
                for (i, heading) in self.app.headings.iter().enumerate() {
                    let selected = focused && i == self.app.toc_selected;
                    let active = self.app.active_heading == Some(i);
                    let color = if selected {
                        theme.fg
                    } else if active {
                        theme.heading
                    } else {
                        theme.muted
                    };
                    let text = RichText::new(&heading.text).font(ts.chrome(CHROME, selected || active, false)).color(color);
                    let response = list_row(ui, ts, theme, selected, text, true, toc_indent(heading.level));
                    if moved && target == Some(i) {
                        response.scroll_to_me(Some(Align::Center));
                    }
                }
            });
        self.toc_cursor = target;
    }

    /// The settings in force and the reader's keys. This is where they live
    /// now that the status bar keeps quiet while reading.
    fn legend(&self, ui: &mut egui::Ui, ts: &Typesetter) {
        let theme = self.app.theme();
        let small = |text: String, color: Color32| {
            Label::new(RichText::new(text).font(ts.chrome(SMALL, false, false)).color(color)).selectable(false)
        };
        let app = &self.app;
        ui.add(small(format!("{} · size {:.0}%", app.theme().name, app.zoom * 100.0), theme.fg));
        ui.add(small(
            format!(
                "width {} · line {} · para {} · turn {}",
                app.page_width,
                app.spacing.line,
                app.spacing.paragraph,
                if app.animate { "on" } else { "off" }
            ),
            theme.fg,
        ));
        ui.add_space(6.0);
        // Rows as tall as their text. A grid otherwise pads every row to
        // the height of a button, which the sidebar did not budget for.
        let row = ui.fonts_mut(|fonts| fonts.row_height(&ts.chrome(SMALL, false, false)));
        egui::Grid::new("legend").num_columns(2).min_row_height(row).spacing(vec2(12.0, ui.spacing().item_spacing.y)).show(ui, |ui| {
            for (keys, action) in LEGEND {
                ui.add(small(keys.to_string(), theme.muted));
                ui.add(small(action.to_string(), theme.muted));
                ui.end_row();
            }
        });
    }
}

/// How far a contents entry sits in for its heading level, so the list
/// reads as an outline rather than a flat run of titles.
fn toc_indent(level: u8) -> f32 {
    12.0 * level.saturating_sub(1) as f32
}

/// A bordered box in the sidebar with a title, `height` points tall. The
/// border takes the link color while the box has focus.
fn section(
    ui: &mut egui::Ui,
    ts: &Typesetter,
    theme: &Theme,
    title: &str,
    focused: bool,
    height: f32,
    contents: impl FnOnce(&mut egui::Ui),
) {
    let stroke = Stroke::new(1.0, if focused { theme.link } else { theme.muted });
    let width = ui.available_width();
    ui.allocate_ui(vec2(width, height), |ui| {
        Frame::NONE.stroke(stroke).corner_radius(4).inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_height((height - 18.0).max(0.0));
            ui.add(Label::new(RichText::new(title).font(ts.chrome(CHROME, true, false)).color(theme.heading)).truncate().selectable(false));
            ui.add_space(4.0);
            contents(ui);
        });
    });
}

/// One entry in a sidebar list, indented `indent` points, with the `›`
/// cursor in its own column ahead of the text when `cursor` is set. A
/// wrapping entry wraps under its own text, not under the cursor; a
/// non-wrapping one is cut off with an ellipsis.
fn list_row(
    ui: &mut egui::Ui,
    ts: &Typesetter,
    theme: &Theme,
    cursor: bool,
    text: RichText,
    wrap: bool,
    indent: f32,
) -> egui::Response {
    let row = ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(indent + CURSOR_WIDTH);
        let label = Label::new(text).selectable(false);
        ui.add(if wrap { label.wrap() } else { label.truncate() })
    });
    if cursor {
        let at = row.inner.rect.left_top() - vec2(CURSOR_WIDTH, 0.0);
        ui.painter().text(at, Align2::LEFT_TOP, "›", ts.chrome(CHROME, false, false), theme.muted);
    }
    row.inner
}

/// Where everything on the reading side of the window goes.
#[derive(Clone, Copy)]
struct Geometry {
    /// The sheet of paper: one page wide, or two for a spread, with the
    /// spine drawn down its middle rather than a gap between two sheets.
    sheet: Rect,
    /// The text block of the left page, or the only page.
    left: Rect,
    /// The text block of the right page, in a spread.
    right: Option<Rect>,
    /// The side margin, which also bounds how far a line may overhang its
    /// column before it is clipped.
    side: f32,
}

impl Geometry {
    /// Fit a page, or a spread, into `area` around a text column `column`
    /// points wide. Is there room for two pages side by side? Then show a
    /// spread, like an open book. Otherwise one page, like a phone or a
    /// narrow window would, with the column narrowed to fit if the window is
    /// narrower than a page. This is decided afresh every frame, so resizing
    /// switches between them with no setting involved.
    fn new(area: Rect, column: f32, body_row: f32) -> Self {
        // Side margins scale with the measure, the proportion a book keeps,
        // with a floor so a narrow column still stands off the sheet's edge.
        let side = (column * 0.09).max(body_row);
        let page = column + 2.0 * side;
        let outer_y = OUTER.min(area.height() * 0.05);
        let sheet_h = (area.height() - 2.0 * outer_y).max(0.0);
        // A little more margin at the foot than at the head, as in a printed
        // page, capped as fractions of the sheet so a short window still
        // leaves room to read.
        let top = (2.0 * body_row).min(sheet_h * 0.1);
        let bottom = (2.4 * body_row).min(sheet_h * 0.12);
        let text_h = (sheet_h - top - bottom).max(body_row);
        let y = area.top() + outer_y;

        if area.width() >= 2.0 * page + 2.0 * OUTER {
            let sheet = Rect::from_min_size(pos2(area.center().x - page, y), vec2(2.0 * page, sheet_h));
            // Both pages get the same margins on every side, including the
            // edge facing the spine, so the spread is symmetric.
            let left = Rect::from_min_size(pos2(sheet.left() + side, y + top), vec2(column, text_h));
            let right = left.translate(vec2(page, 0.0));
            Geometry { sheet, left, right: Some(right), side }
        } else {
            let outer_x = OUTER.min(area.width() * 0.04);
            let width = page.min(area.width() - 2.0 * outer_x).max(1.0);
            let side = side.min(width * 0.08);
            let sheet = Rect::from_min_size(pos2(area.center().x - width / 2.0, y), vec2(width, sheet_h));
            let left = Rect::from_min_size(pos2(sheet.left() + side, y + top), vec2((width - 2.0 * side).max(1.0), text_h));
            Geometry { sheet, left, right: None, side }
        }
    }
}

/// Everything needed to paint the sheet open at some page.
struct Leaf<'a> {
    pages: &'a Pages,
    prepared: &'a Prepared,
    ts: &'a Typesetter,
    theme: &'static Theme,
}

impl Leaf<'_> {
    /// Paint the sheet open at `page`: the paper, then the page, or both
    /// pages and the spine between them in a spread. The left page of a
    /// spread shows `page` and the right the one after it.
    fn paint(&self, painter: &Painter, page: u16) {
        let geometry = &self.prepared.geometry;
        painter.rect_filled(geometry.sheet, CornerRadius::same(3), self.theme.page);
        let page = page as usize;
        self.page(painter, geometry.left, page);
        if let Some(right) = geometry.right {
            self.spine(painter);
            if page + 1 < self.prepared.count {
                self.page(painter, right, page + 1);
            }
        }
    }

    /// The spine: a rule down the middle of the spread, with the paper
    /// shading slightly toward it on both sides, the way a bound book curves
    /// into its gutter.
    fn spine(&self, painter: &Painter) {
        let sheet = self.prepared.geometry.sheet;
        let x = sheet.center().x;
        let shade = Color32::from_black_alpha(if is_dark(self.theme) { 60 } else { 20 });
        let y = sheet.y_range();
        painter.add(anim::gradient(Rect::from_x_y_ranges(x - SPINE_SHADE..=x, y), Color32::TRANSPARENT, shade));
        painter.add(anim::gradient(Rect::from_x_y_ranges(x..=x + SPINE_SHADE, y), shade, Color32::TRANSPARENT));
        painter.vline(x, y, Stroke::new(1.0, self.theme.muted));
    }

    /// Paint one page's rows into its text block, top to bottom.
    fn page(&self, painter: &Painter, rect: Rect, page: usize) {
        // Nothing crosses into the neighboring page: a line may overhang its
        // column by at most half a margin before it is clipped.
        let bounds = rect.expand2(vec2(self.prepared.geometry.side / 2.0, 0.0));
        let painter = painter.with_clip_rect(bounds.intersect(painter.clip_rect()));
        let mut y = rect.top();
        for row in self.pages.rows_on(page) {
            let line = Rect::from_min_size(pos2(rect.left(), y), vec2(rect.width(), row.height));
            match &row.kind {
                RowKind::Blank => {}
                RowKind::Text { x, spans, rule } => {
                    self.line(&painter, pos2(rect.left() + x, y), row.height, spans);
                    if let Some(rule) = rule {
                        painter.vline(rect.left() + rule, line.y_range(), Stroke::new(2.0, self.theme.muted));
                    }
                }
                RowKind::Verbatim { span, width } => self.code(&painter, line, span, *width),
                RowKind::Cells(cells) => {
                    for cell in cells {
                        let cell_rect = Rect::from_min_size(pos2(rect.left() + cell.x, y), vec2(cell.width, row.height));
                        let clipped = painter.with_clip_rect(cell_rect.intersect(painter.clip_rect()));
                        self.line(&clipped, cell_rect.min, row.height, &cell.spans);
                    }
                }
                RowKind::TableRule { columns, style } => {
                    let stroke = Stroke::new(1.0, typeset::ink(style.ink, self.theme));
                    for (x, width) in columns {
                        let left = rect.left() + x;
                        painter.hline(left..=left + width, line.center().y, stroke);
                    }
                }
            }
            y += row.height;
        }
    }

    /// One line of spans, its left edge at `at.x`, centered in a row
    /// `height` points tall so the leading falls evenly above and below.
    fn line(&self, painter: &Painter, at: Pos2, height: f32, spans: &[Span]) {
        let galley = painter.layout_job(self.ts.job(spans, self.theme));
        let top = at.y + (height - galley.size().y) / 2.0;
        painter.galley(pos2(at.x, top), galley, self.theme.fg);
    }

    /// A line of code: a band of tinted paper across the column, the code
    /// on it shifted by the pan and clipped at the column's edges, and a `‹`
    /// or `›` on each edge that has content cut off past it.
    fn code(&self, painter: &Painter, row: Rect, span: &Span, width: f32) {
        if span.style.tint {
            painter.rect_filled(row, 0.0, self.theme.code_bg);
        }
        let marker = self.prepared.marker;
        let (x, cut_left, cut_right) = code_window(width, row.width(), self.prepared.pan, marker);
        let mut window = row;
        if cut_left {
            window.min.x += marker;
        }
        if cut_right {
            window.max.x -= marker;
        }
        self.line(&painter.with_clip_rect(window.intersect(painter.clip_rect())), pos2(row.left() + x, row.top()), row.height(), std::slice::from_ref(span));

        let font = self.ts.font_id(Style::default().code());
        if cut_left {
            painter.text(pos2(row.left(), row.center().y), Align2::LEFT_CENTER, "‹", font.clone(), self.theme.muted);
        }
        if cut_right {
            painter.text(pos2(row.right(), row.center().y), Align2::RIGHT_CENTER, "›", font, self.theme.muted);
        }
    }
}

/// Where a code line `width` points long sits in a column `column` wide
/// when panned `pan` points: the offset its start is drawn at, and whether
/// each edge is cut. A cut left edge gives up `marker` points to the `‹`
/// that marks it, and the text moves over to make room, so the first
/// visible character is exactly the one `pan` points in.
fn code_window(width: f32, column: f32, pan: f32, marker: f32) -> (f32, bool, bool) {
    let cut_left = pan > 0.0;
    let x = if cut_left { marker - pan } else { 0.0 };
    (x, cut_left, x + width > column + 0.5)
}

/// The rect the reading side of the window gets: everything right of the
/// sidebar, when it is showing, and above the status bar. This has to agree
/// with how `Reader::frame` divides the window into panels, and a test
/// checks that it does.
fn document_area(window: Rect, sidebar: bool) -> Rect {
    let left = if sidebar { SIDEBAR_WIDTH } else { 0.0 };
    Rect::from_min_max(
        pos2((window.left() + left).min(window.right()), window.top()),
        pos2(window.right(), (window.bottom() - STATUS_HEIGHT).max(window.top())),
    )
}

/// A page index as the `u16` the app stores it in.
fn page_number(index: usize) -> u16 {
    index.min(u16::MAX as usize) as u16
}

fn window_title(app: &App) -> String {
    if app.title.is_empty() { "booknook".to_string() } else { format!("{} · booknook", app.title) }
}

/// Whether a theme is a dark one, judged by its paper.
fn is_dark(theme: &Theme) -> bool {
    let page = theme.page;
    (page.r() as u32 + page.g() as u32 + page.b() as u32) < 384
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use eframe::egui::{Galley, RawInput, Shape, Vec2};

    use super::*;
    use crate::events::{KeyCode, handle_key};
    use crate::markdown;

    /// A headless window: a real egui context and a real `Reader`, driven
    /// one frame at a time at a fixed size, with no display involved. This is
    /// the closest thing to launching the app that runs anywhere.
    struct Harness {
        ctx: egui::Context,
        reader: Reader,
        size: Vec2,
        time: f64,
        /// Input for the next frame, as egui would deliver it.
        events: Vec<egui::Event>,
    }

    impl Harness {
        fn new(app: App, width: f32, height: f32) -> Self {
            let ctx = egui::Context::default();
            let faces = typeset::install(&ctx);
            Harness { ctx, reader: Reader::new(app, faces), size: vec2(width, height), time: 0.0, events: Vec::new() }
        }

        fn app(&mut self) -> &mut App {
            &mut self.reader.app
        }

        /// Type `text` during the next frame.
        fn type_text(&mut self, text: &str) {
            self.events.push(egui::Event::Text(text.to_string()));
        }

        /// Run one frame and return every piece of text it painted.
        fn frame(&mut self) -> Vec<Arc<Galley>> {
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                time: Some(self.time),
                events: std::mem::take(&mut self.events),
                ..Default::default()
            };
            let output = self.ctx.run_ui(input, |ui| self.reader.frame(ui));
            let mut galleys = Vec::new();
            for clipped in &output.shapes {
                collect_text(&clipped.shape, &mut galleys);
            }
            galleys
        }

        /// Run one frame and return its text as one string.
        fn screen(&mut self) -> String {
            self.frame().iter().map(|galley| galley.text().to_string()).collect::<Vec<_>>().join("\n")
        }
    }

    fn collect_text(shape: &Shape, out: &mut Vec<Arc<Galley>>) {
        match shape {
            Shape::Text(text) => out.push(text.galley.clone()),
            Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect_text(shape, out)),
            _ => {}
        }
    }

    fn app_with(doc: &str) -> App {
        let mut app = App::new();
        let parsed = markdown::render_markdown(doc);
        app.blocks = parsed.blocks;
        app.headings = parsed.headings;
        app.title = "test.md".into();
        app
    }

    const DOC: &str = "# Chapter One\n\nSome body text here.\n\n## A Section\n\nMore text.\n";

    /// A full frame, exercising the real draw path: the contents box appears
    /// in the sidebar and lists the document's headings, and the page shows
    /// the body.
    #[test]
    fn renders_contents_list_alongside_the_reader() {
        let mut harness = Harness::new(app_with(DOC), 1000.0, 700.0);
        let text = harness.screen();
        assert!(text.contains("Contents"), "sidebar should show the contents box:\n{text}");
        assert!(text.contains("Chapter One"), "the H1 should appear:\n{text}");
        assert!(text.contains("A Section"), "the H2 should appear:\n{text}");
        assert!(text.contains("Some body text"), "the page should show the body:\n{text}");
    }

    /// Selecting a heading in the contents list turns the page to it. The
    /// event handler only records the target block; this confirms the draw
    /// step resolves that block to a real page and clears the request. A
    /// short window forces a short page, so the second heading genuinely
    /// lands past the first.
    #[test]
    fn a_pending_jump_turns_to_the_heading_page() {
        let mut app = app_with(DOC);
        app.pending_jump = Some(app.headings[1].block);
        let mut harness = Harness::new(app, 1000.0, 160.0);
        harness.frame();

        let app = harness.app();
        assert!(app.page >= 1, "the jump should leave the first page, landed on {}", app.page);
        assert_eq!(app.pending_jump, None, "the jump request should be consumed");
        assert_eq!(app.active_heading, Some(1), "the second heading should now be the active one");
    }

    /// The reading area laid out before the panels are drawn must be the rect
    /// the central panel actually receives, or the page would be positioned
    /// for a different window than the one it lands in. Checked with the
    /// sidebar showing and receded.
    #[test]
    fn document_area_matches_the_drawn_layout() {
        let window = Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 700.0));
        for (focus, sidebar) in [(Focus::Files, true), (Focus::Document, false)] {
            let mut app = app_with(DOC);
            app.focus = focus;
            let mut harness = Harness::new(app, window.width(), window.height());
            harness.frame();
            assert_eq!(harness.reader.drawn_area, Some(document_area(window, sidebar)), "sidebar showing: {sidebar}");
        }
    }

    /// While the reader has focus the sidebar recedes entirely: no file
    /// list, no contents box, just the page. Handing focus back to the
    /// sidebar brings it back.
    #[test]
    fn the_sidebar_recedes_while_reading_and_returns_on_focus() {
        let mut app = app_with("# Chapter One\n\nSome body text here.\n");
        app.focus = Focus::Document;
        let mut harness = Harness::new(app, 1000.0, 700.0);

        let reading = harness.screen();
        assert!(!reading.contains("Contents"), "the contents box should recede while reading:\n{reading}");
        assert!(reading.contains("Some body text"), "the page itself must remain:\n{reading}");

        harness.app().focus = Focus::Files;
        let browsing = harness.screen();
        assert!(browsing.contains("Contents"), "the sidebar should return with focus:\n{browsing}");
    }

    /// With nothing open there is nothing to read, so the sidebar holds its
    /// ground even if focus lands on the empty reader; receding would leave
    /// a blank window with no way to see where you are.
    #[test]
    fn an_empty_reader_keeps_the_sidebar() {
        let mut app = App::new();
        app.focus = Focus::Document;
        assert!(app.sidebar_visible(), "no document means the sidebar stays");
    }

    /// Sideways pan is clamped at draw time to the widest code line's real
    /// overflow, so asking for "further" lands at the end of the content and
    /// stays there, and the panned line carries the left-edge marker.
    #[test]
    fn pan_clamps_to_the_widest_code_line() {
        let mut app = app_with(&format!("Intro text.\n\n```\n{}\n```\n", "x".repeat(120)));
        app.focus = Focus::Document;
        app.pan = u16::MAX;
        let mut harness = Harness::new(app, 1000.0, 700.0);
        let text = harness.screen();

        let clamped = harness.app().pan;
        assert!(clamped > 0 && clamped < u16::MAX, "pan should clamp to the overflow, got {clamped}");
        assert!(text.contains('‹'), "a panned line should mark its cut left edge");

        handle_key(harness.app(), KeyCode::Char('.'));
        harness.frame();
        assert_eq!(harness.app().pan, clamped, "panning past the end stays at the end");
    }

    /// A heading too long for the sidebar wraps under itself rather than
    /// being cut off with an ellipsis.
    #[test]
    fn a_long_heading_wraps_instead_of_clipping() {
        let heading = "When intelligence itself becomes the product being shipped";
        let mut harness = Harness::new(app_with(&format!("# {heading}\n\nBody.\n")), 1000.0, 700.0);
        let galleys = harness.frame();
        let entry = galleys.iter().find(|galley| galley.text() == heading).expect("the contents entry");
        assert!(entry.rows.len() > 1, "the entry should wrap onto more than one line");
    }

    /// With animation on, a page turn starts a turn that runs for a moment
    /// and then settles; with it off, the page simply changes.
    #[test]
    fn a_page_turn_animates_only_when_asked_to() {
        let doc: String = (0..40).map(|i| format!("Paragraph {i} with a few words in it.\n\n")).collect();
        let mut app = app_with(&doc);
        app.focus = Focus::Document;
        let mut harness = Harness::new(app, 800.0, 400.0);
        harness.frame();

        harness.type_text(" ");
        harness.frame();
        assert_eq!(harness.app().page, 1);
        assert!(harness.reader.turn.is_none(), "animation is off by default");

        harness.type_text("a ");
        harness.frame();
        assert!(harness.app().animate, "`a` switches the page turn on");
        assert!(harness.reader.turn.is_some_and(|turn| turn.from == 1), "the turn should start from page 1");

        harness.time += 1.0;
        harness.frame();
        assert!(harness.reader.turn.is_none(), "the turn should settle once its time is up");
        assert_eq!(harness.app().page, 2);
    }

    /// A line that fits shows no cut marker; a wider one is marked on the
    /// right until panned, then on both sides, then only on the left once
    /// its end is in view, with the text shifted clear of the left marker.
    #[test]
    fn code_window_follows_the_pan() {
        assert_eq!(code_window(6.0, 8.0, 0.0, 1.0), (0.0, false, false));
        assert_eq!(code_window(16.0, 8.0, 0.0, 1.0), (0.0, false, true));
        assert_eq!(code_window(16.0, 8.0, 4.0, 1.0), (-3.0, true, true));
        assert_eq!(code_window(16.0, 8.0, 9.0, 1.0), (-8.0, true, false));
    }
}
