//! Application state: what booknook currently knows and is showing.
//!
//! `App` is the single source of truth. Nothing outside this module writes
//! to its fields as a side effect of some unrelated operation. State
//! changes that need to keep several fields consistent with each other,
//! such as `dir` and `entries`, or `blocks` and `page`, go through a
//! method here, so that consistency lives in one place.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::browser::{self, Entry};
use crate::epub;
use crate::markdown::{self, Heading, RenderLine};
use crate::session::Session;
use crate::theme::{Theme, THEMES};
use crate::wrap::Spacing;

/// Which pane currently receives keyboard input. All panes are always drawn;
/// this only decides where `j`, `k`, the arrow keys, and Enter go. `Files`
/// and `Toc` both live in the sidebar, one above the other, and focus moves
/// between them and the reader with Tab.
pub(crate) enum Focus {
    Files,
    Toc,
    Document,
}

/// Everything the running app needs to know.
///
/// The sidebar and the document are not mutually exclusive. Both are
/// always on screen, so their state lives side by side in one `App`
/// rather than behind an enum. `focus` is the only thing that still works
/// like a mode switch, because exactly one pane owns the keyboard at a
/// time.
///
/// The reader stores a page number, not a scroll row. An e-ink reader
/// flips whole pages and never scrolls mid-page. Which rows a given page
/// holds is derived at draw time from the current page size, so `page`
/// stays meaningful even if the window is resized between frames.
pub(crate) struct App {
    pub(crate) focus: Focus,
    pub(crate) dir: PathBuf,
    pub(crate) entries: Vec<Entry>,
    pub(crate) selected: usize,
    pub(crate) title: String,
    /// The path of the open document, if any. Kept apart from `title`, which
    /// is only for display: this is the canonical key under which the reading
    /// position is remembered, and the file to reopen on the next launch.
    pub(crate) current: Option<PathBuf>,
    /// The parsed document, not yet wrapped to any particular width.
    /// Layout happens at draw time, against the current column width, in
    /// the `ui` module. This stays empty until a file has been opened.
    pub(crate) blocks: Vec<RenderLine>,
    /// Bumped every time `blocks` is replaced. Laying out a whole book is
    /// too slow to repeat every frame, so the `ui` module keeps the last
    /// layout and redoes it only when this, the page size, or the spacing
    /// has changed.
    pub(crate) generation: u64,
    /// The open document's headings, shown in the sidebar as a table of
    /// contents. Each carries the block it points into, so selecting one can
    /// be resolved to a page at draw time.
    pub(crate) headings: Vec<Heading>,
    /// Which contents entry the sidebar's cursor is on, when the contents
    /// pane has focus.
    pub(crate) toc_selected: usize,
    /// The heading the current page falls under, if any. Written by the `ui`
    /// module every frame, since only it knows the row each heading lands on
    /// at the current width, and read back by the same module to highlight
    /// that entry in the contents list. This is the same kind of
    /// draw-time-to-draw-time channel as `spread`.
    pub(crate) active_heading: Option<usize>,
    /// A pending request to jump the reader to a particular block, set when a
    /// contents entry is chosen. The `ui` module consumes it on the next
    /// frame, once it knows the page size needed to turn the block into a
    /// page, and clears it back to `None`. This mirrors how `G` asks for the
    /// last page without computing it here.
    pub(crate) pending_jump: Option<usize>,
    pub(crate) page: u16,
    /// How many monospace characters code blocks and diagrams are shifted
    /// left, the keyboard version of GitHub's horizontal scrollbar. Only
    /// verbatim lines move; prose is already wrapped to fit and stays put.
    /// The `ui` module clamps this every frame to the widest verbatim line's
    /// actual overflow, the way it clamps `page`, so panning stops exactly
    /// where the content ends. Reset on page turns and jumps: a pan is a way
    /// of leaning in to inspect a wide figure, not a persistent view.
    pub(crate) pan: u16,
    /// Whether the last draw showed two pages side by side. Set by the
    /// `ui` module every frame, since it is the only place that knows the
    /// current width, and read by the event handler to decide whether a
    /// page turn should move by one page or by a whole spread.
    pub(crate) spread: bool,
    /// How the page is set: the width of a reading column, in average
    /// characters, and how much air goes between lines and between
    /// paragraphs. These are adjustable while reading, because the right
    /// values are a matter of taste and of the screen.
    pub(crate) page_width: u16,
    pub(crate) spacing: Spacing,
    /// The page last reached in every document opened, keyed by canonical
    /// path. Loaded from the saved session at startup and written back on
    /// quit, so returning to any file lands on the page it was left on. Only
    /// `current`'s entry is updated live; the rest carry over untouched.
    positions: HashMap<PathBuf, u16>,
    /// An index into `THEMES` rather than a `Theme` value, so that `App`
    /// borrows the palette instead of owning a copy of it.
    theme_index: usize,
    /// Whether a page turn sweeps the old page away over the new one, rather
    /// than swapping instantly. Off by default: the plain swap is the calmer
    /// default, and this is here for readers who want the tactile turn. Toggled
    /// with `a` and remembered across runs.
    pub(crate) animate: bool,
    /// Set by the event handler when a key turned the page, read and cleared by
    /// the `ui` module to decide whether to run the turn animation. Jumps and
    /// typography changes leave it alone, so only true page turns animate.
    pub(crate) page_turn: bool,
    /// The window's zoom factor, which is how text size is set: Ctrl with
    /// plus or minus scales everything, and the page reflows. Written back by
    /// the `ui` module each frame so the session can remember it.
    pub(crate) zoom: f32,
    /// A message for the status bar, such as why a file failed to open.
    /// Cleared by the next key, so it reads as a reply to the last action.
    pub(crate) notice: Option<String>,
    pub(crate) quit: bool,
}

/// The range each typographic setting is allowed to move within.
pub(crate) const MIN_PAGE_WIDTH: u16 = 40;
pub(crate) const MAX_PAGE_WIDTH: u16 = 96;
pub(crate) const MAX_SPACING: u16 = 3;
/// A remembered zoom outside this range is almost certainly a mistake, and
/// would open a window too small or too large to read in.
const MIN_ZOOM: f32 = 0.5;
const MAX_ZOOM: f32 = 3.0;

impl App {
    pub(crate) fn new() -> Self {
        App {
            focus: Focus::Files,
            dir: PathBuf::new(),
            entries: Vec::new(),
            selected: 0,
            title: String::new(),
            current: None,
            blocks: Vec::new(),
            generation: 0,
            headings: Vec::new(),
            toc_selected: 0,
            active_heading: None,
            pending_jump: None,
            page: 0,
            pan: 0,
            spread: false,
            page_width: 58,
            // Book leading and one step of paragraph air. The leading
            // starts inside the band legibility research recommends for body
            // text; pushed much past it, the eye can no longer track the
            // return sweep to the next line, which is what makes long reading
            // tiring. The paragraph gap keeps the structure legible.
            spacing: Spacing { line: 0, paragraph: 1 },
            positions: HashMap::new(),
            theme_index: 0,
            animate: false,
            page_turn: false,
            zoom: 1.0,
            notice: None,
            quit: false,
        }
    }

    /// The palette currently in use. The returned reference borrows from
    /// the `THEMES` static, not from `self`, so holding onto it does not
    /// keep `App` borrowed.
    pub(crate) fn theme(&self) -> &'static Theme {
        &THEMES[self.theme_index]
    }

    /// Move to the next palette. Spans carry roles rather than colors, so
    /// nothing is re-parsed: the next frame simply paints the same document
    /// in new inks, on the same page.
    pub(crate) fn cycle_theme(&mut self) {
        self.theme_index = (self.theme_index + 1) % THEMES.len();
    }

    /// Re-read the open document from disk and render it again, so edits made
    /// elsewhere show up without closing and reopening the file. The page
    /// number is deliberately kept: the `ui` module already clamps it to the
    /// document's real length every frame, so if the file shrank the reader
    /// lands on the new last page rather than past it. A gist has no path to
    /// go back to, so with nothing on disk this is a no-op, as it is when the
    /// file has vanished mid-session; keeping the stale content is the
    /// gentler failure.
    pub(crate) fn reload(&mut self) {
        let Some(path) = self.current.clone() else { return };
        let parsed = if epub::is_epub(&path) {
            match epub::load(&path) {
                Ok(book) => book.parsed,
                Err(_) => return,
            }
        } else {
            match std::fs::read_to_string(&path) {
                Ok(raw) => markdown::render_markdown(&raw),
                Err(_) => return,
            }
        };
        self.blocks = parsed.blocks;
        self.headings = parsed.headings;
        self.generation += 1;
        // The heading list may have shrunk along with the document, so the
        // contents cursor is pulled back inside it. A jump requested before
        // the reload would point into the old block numbering, so it is
        // dropped rather than landing somewhere arbitrary.
        self.toc_selected = self.toc_selected.min(self.headings.len().saturating_sub(1));
        self.pending_jump = None;
    }

    /// Read `path`, parse it, and switch keyboard focus to the reader. The
    /// page it opens on is whatever was last reached in this file, so
    /// reopening a document resumes it rather than restarting it. Markdown
    /// and EPUB part ways only here, at the parsing step; everything after
    /// the parse treats them identically.
    pub(crate) fn load_file(&mut self, path: &Path) -> Result<()> {
        // Record where the previously open file was left before moving on, so
        // a session that touches several files remembers each of them.
        self.remember_position();

        if epub::is_epub(path) {
            let book = epub::load(path)?;
            // The book's own title, when its metadata carries one, beats the
            // filename: an EPUB filename is often a whole catalog entry.
            self.title = book
                .title
                .unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
            self.blocks = book.parsed.blocks;
            self.headings = book.parsed.headings;
        } else {
            let raw = std::fs::read_to_string(path)
                .with_context(|| format!("could not read {}", path.display()))?;
            let parsed = markdown::render_markdown(&raw);
            self.title = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.blocks = parsed.blocks;
            self.headings = parsed.headings;
        }

        self.generation += 1;
        self.current = Some(canonical(path));
        self.toc_selected = 0;
        self.active_heading = None;
        self.pending_jump = None;
        self.page = self.saved_page(path);
        self.pan = 0;
        self.focus = Focus::Document;
        Ok(())
    }

    /// Open already-fetched text in the reader, the way `load_file` opens a
    /// file, but without a path behind it. This is how a gist is shown: the
    /// bytes have already come off the network, so there is nothing to read
    /// from disk. `title` is whatever should appear in the reader's header in
    /// place of a filename.
    ///
    /// `current` is deliberately left `None`. A gist has no canonical path, so
    /// it is neither remembered as the file to reopen next launch nor given an
    /// entry in the per-file position map; both of those are keyed by path.
    /// The trade-off is that a gist always opens on its first page, which is
    /// the right default for something reached by pasting a link rather than
    /// returned to like a book on a shelf.
    pub(crate) fn load_content(&mut self, raw: String, title: String) {
        // Fold away the previously open file's page before replacing it, the
        // same courtesy `load_file` extends, so opening a gist mid-session
        // does not lose the place in whatever was open before.
        self.remember_position();

        let parsed = markdown::render_markdown(&raw);
        self.title = title;
        self.current = None;
        self.blocks = parsed.blocks;
        self.headings = parsed.headings;
        self.generation += 1;
        self.toc_selected = 0;
        self.active_heading = None;
        self.pending_jump = None;
        self.page = 0;
        self.pan = 0;
        self.focus = Focus::Document;
    }

    /// Whether the sidebar should be on screen. It shows whenever it could
    /// be needed: while it owns the keyboard, and while there is nothing to
    /// read. The moment focus moves to the reader it recedes entirely, so
    /// the page stands alone the way it would on an e-reader, and Tab or
    /// `o` brings it back. Because the reading column keeps its fixed width
    /// either way, receding recenters the sheet without reflowing a single
    /// line, so page numbers do not change.
    pub(crate) fn sidebar_visible(&self) -> bool {
        !matches!(self.focus, Focus::Document) || self.blocks.is_empty()
    }

    /// Point the sidebar at `dir` and list its contents.
    pub(crate) fn enter_dir(&mut self, dir: PathBuf) {
        self.entries = browser::list_dir(&dir);
        self.dir = dir;
        self.selected = 0;
    }

    /// Ask the reader to move to the heading at `toc_index` in the contents
    /// list. The actual page is worked out at draw time, so this only records
    /// the target block and hands focus to the reader.
    pub(crate) fn jump_to_heading(&mut self, toc_index: usize) {
        if let Some(heading) = self.headings.get(toc_index) {
            self.pending_jump = Some(heading.block);
            self.pan = 0;
            self.focus = Focus::Document;
        }
    }

    /// Store the current page under the open file, so it can be resumed
    /// later. A no-op when nothing is open.
    pub(crate) fn remember_position(&mut self) {
        if let Some(current) = &self.current {
            self.positions.insert(current.clone(), self.page);
        }
    }

    /// Restore adjustable settings and remembered positions from a saved
    /// session. Called once at startup, before any file is opened, so the
    /// restored width, spacing, and theme are already in force when the first
    /// document is loaded.
    pub(crate) fn apply_session(&mut self, session: &Session) {
        self.page_width = session.page_width.clamp(MIN_PAGE_WIDTH, MAX_PAGE_WIDTH);
        self.spacing = Spacing {
            line: session.line.min(MAX_SPACING),
            paragraph: session.para.min(MAX_SPACING),
        };
        self.theme_index = session.theme_index % THEMES.len();
        self.animate = session.animate;
        self.zoom = session.zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        self.positions = session.positions.clone();
    }

    /// Capture the current settings and positions as a `Session` to be
    /// written out on quit. The caller is expected to have called
    /// `remember_position` first, so the open file's latest page is included.
    pub(crate) fn to_session(&self) -> Session {
        Session {
            last_file: self.current.clone(),
            page_width: self.page_width,
            line: self.spacing.line,
            para: self.spacing.paragraph,
            theme_index: self.theme_index,
            animate: self.animate,
            zoom: self.zoom,
            positions: self.positions.clone(),
        }
    }

    /// The page remembered for `path`, or zero if it has never been opened.
    fn saved_page(&self, path: &Path) -> u16 {
        self.positions.get(&canonical(path)).copied().unwrap_or(0)
    }
}

/// The canonical form of a path, used as the stable key for a document's
/// remembered position. Canonicalizing means the same file reached by a
/// relative path, an absolute one, or through a symlink all resolve to one
/// entry rather than three. If the path cannot be canonicalized, for
/// instance because it no longer exists, its own form is used as-is, which is
/// still a consistent key for the life of the process.
fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reloading re-reads the open file from disk, so an edit made elsewhere
    /// shows up without closing and reopening, and the page number stays
    /// put for the draw step to clamp rather than snapping back to the start.
    #[test]
    fn reload_picks_up_changes_on_disk() {
        let path = std::env::temp_dir().join(format!("booknook-reload-{}.md", std::process::id()));
        std::fs::write(&path, "# One\n\ntext\n").unwrap();
        let mut app = App::new();
        app.load_file(&path).unwrap();
        assert_eq!(app.headings.len(), 1);
        app.page = 7;
        let generation = app.generation;

        std::fs::write(&path, "# One\n\n# Two\n\nmore\n").unwrap();
        app.reload();
        std::fs::remove_file(&path).ok();

        assert_eq!(app.headings.len(), 2, "the new heading must appear");
        assert_eq!(app.page, 7, "the reader stays where it was");
        assert!(app.generation > generation, "the cached layout must be invalidated");
    }

    /// A document with no file behind it, like a gist, has nothing on disk
    /// to go back to, so reloading leaves it exactly as it is.
    #[test]
    fn reload_without_a_path_is_a_noop() {
        let mut app = App::new();
        app.load_content("# Pasted\n\ntext\n".into(), "gist".into());
        let blocks_before = app.blocks.len();
        app.reload();
        assert_eq!(app.blocks.len(), blocks_before);
    }
}
