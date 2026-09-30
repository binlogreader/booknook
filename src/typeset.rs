//! How a `Style` looks on the page: which face, what size, which ink.
//!
//! In the terminal, booknook could control the measure, the rhythm, and the
//! color, but never the typeface or the leading, which belonged to the
//! terminal emulator. In a window both are booknook's to set, and this
//! module is where that happens.
//!
//! Body text is set in Helvetica, or the nearest thing to it on the machine,
//! the way Kindle offers it: a plain neo-grotesque sans with an even color on
//! the page. Few systems carry Helvetica itself, so the search moves down a
//! short list of faces drawn to its measure, Arial first, since it ships with
//! Windows and macOS. Each has real bold and italic cuts, which egui cannot
//! synthesize. Failing all of them it settles on egui's built-in sans, with
//! slanted rather than true italics. Code is set in egui's bundled Hack,
//! which is the same everywhere.
//!
//! Sizes are in points before zoom. Ctrl with plus or minus scales the
//! whole window, the way a browser does, and the page reflows to match.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::epaint::FontsView;
use eframe::egui::text::LayoutJob;
use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextFormat};

use crate::text::{Ink, Span, Style};
use crate::theme::Theme;
use crate::wrap::{Measure, Spacing};

/// Body text size in points, before zoom.
pub(crate) const BODY: f32 = 19.0;

/// The sidebar and status bar are chrome: set smaller than the page, in the
/// same face, so they recede without looking like a different program.
pub(crate) const CHROME: f32 = 15.0;

/// The four text families booknook registers with egui, in the order the
/// `Typesetter` indexes them: regular, bold, italic, bold italic.
const FAMILIES: [&str; 4] = ["text", "text-bold", "text-italic", "text-bold-italic"];

/// Helvetica and the faces drawn to its measure, best first, as the file
/// names of their regular, bold, italic, and bold italic cuts. The first
/// whose regular cut exists in any font directory wins.
const FACES: &[[&str; 4]] = &[
    // Helvetica itself, where someone has installed it.
    ["Helvetica.ttf", "Helvetica-Bold.ttf", "Helvetica-Oblique.ttf", "Helvetica-BoldOblique.ttf"],
    // Arial, drawn in 1982 to Helvetica's exact widths. It ships with
    // Windows and macOS, which name its files differently.
    ["arial.ttf", "arialbd.ttf", "ariali.ttf", "arialbi.ttf"],
    ["Arial.ttf", "Arial Bold.ttf", "Arial Italic.ttf", "Arial Bold Italic.ttf"],
    // The free Helvetica clones Linux distributions carry.
    ["NimbusSans-Regular.otf", "NimbusSans-Bold.otf", "NimbusSans-Italic.otf", "NimbusSans-BoldItalic.otf"],
    [
        "texgyreheros-regular.otf",
        "texgyreheros-bold.otf",
        "texgyreheros-italic.otf",
        "texgyreheros-bolditalic.otf",
    ],
    [
        "LiberationSans-Regular.ttf",
        "LiberationSans-Bold.ttf",
        "LiberationSans-Italic.ttf",
        "LiberationSans-BoldItalic.ttf",
    ],
];

/// Which of the face's cuts were actually found. A missing italic is
/// slanted by egui instead; a missing bold falls back to regular, since egui
/// has no way to embolden a face.
#[derive(Clone, Copy, Default, Debug)]
pub(crate) struct Faces {
    pub(crate) italic: bool,
}

/// Find the text face, register it with egui behind the built-in fonts'
/// glyph coverage, and report which cuts it has. The new fonts take effect
/// at the start of the next frame.
pub(crate) fn install(ctx: &egui::Context) -> Faces {
    let (definitions, faces) = definitions();
    ctx.set_fonts(definitions);
    faces
}

fn definitions() -> (FontDefinitions, Faces) {
    let mut defs = FontDefinitions::default();

    let mut names: [Option<String>; 4] = Default::default();
    if let Some(files) = find_face() {
        for (i, file) in files.into_iter().enumerate() {
            if let Some(bytes) = file.and_then(|path| std::fs::read(path).ok()) {
                let name = format!("booknook-{}", FAMILIES[i]);
                defs.font_data.insert(name.clone(), Arc::new(FontData::from_owned(bytes)));
                names[i] = Some(name);
            }
        }
    }

    // egui's own proportional chain stays behind the face, so a glyph it
    // lacks, a box-drawing line or an emoji, still renders. Hack goes
    // second, for the symbols egui's sans lacks.
    let mut fallback = defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    fallback.insert(fallback.len().min(1), "Hack".to_owned());

    // Each cut falls back to the nearest one that exists: bold italic to
    // bold, then to italic, then to regular.
    let preference: [&[usize]; 4] = [&[0], &[1, 0], &[2, 0], &[3, 1, 2, 0]];
    for (i, order) in preference.iter().enumerate() {
        let mut chain: Vec<String> = order.iter().find_map(|&j| names[j].clone()).into_iter().collect();
        chain.extend(fallback.iter().cloned());
        defs.families.insert(FontFamily::Name(FAMILIES[i].into()), chain);
    }
    // egui's own widgets, the sidebar's lists among them, read in the same
    // face as the page.
    if let Some(regular) = defs.families.get(&FontFamily::Name(FAMILIES[0].into())).cloned() {
        defs.families.insert(FontFamily::Proportional, regular);
    }

    (defs, Faces { italic: names[2].is_some() })
}

/// The first face from `FACES` found on disk, as paths to its four cuts. A
/// cut that is missing comes back as `None`.
fn find_face() -> Option<[Option<PathBuf>; 4]> {
    let dirs = font_dirs();
    for family in FACES {
        for dir in &dirs {
            if dir.join(family[0]).is_file() {
                return Some(family.map(|file| Some(dir.join(file)).filter(|path| path.is_file())));
            }
        }
    }
    None
}

/// Where fonts live on the three desktop platforms, system-wide and per
/// user. Directories that do not exist are harmless; they simply match
/// nothing.
fn font_dirs() -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Some(windir) = std::env::var_os("WINDIR") {
        found.push(PathBuf::from(windir).join("Fonts"));
    }
    if let Some(local) = dirs::data_local_dir() {
        found.push(local.join("Microsoft").join("Windows").join("Fonts"));
    }
    if let Some(home) = dirs::home_dir() {
        found.push(home.join("Library").join("Fonts"));
        found.push(home.join(".local").join("share").join("fonts"));
        found.push(home.join(".fonts"));
    }
    for dir in [
        "/System/Library/Fonts/Supplemental",
        "/Library/Fonts",
        "/usr/share/fonts/urw-base35",
        "/usr/share/fonts/opentype/urw-base35",
        "/usr/share/fonts/opentype/texgyre",
        "/usr/share/fonts/TTF",
        "/usr/share/fonts/truetype/liberation",
        "/usr/share/fonts/liberation-sans",
    ] {
        found.push(PathBuf::from(dir));
    }
    found
}

/// The resolved typography for one frame: the reader's spacing settings
/// turned into points, and styles turned into egui text formats.
#[derive(Clone)]
pub(crate) struct Typesetter {
    faces: Faces,
    families: [FontFamily; 4],
    /// Line height as a multiple of the type size, for body text.
    leading: f32,
    /// The air between two blocks, in points.
    gap: f32,
}

impl Typesetter {
    /// Leading starts at 1.4, inside the 1.2 to 1.45 band legibility
    /// research recommends for body text, and each `]` adds 0.15. The
    /// paragraph gap is most of a line per `}`: enough that the eye sees a
    /// new block, not so much that the page falls apart into pieces.
    pub(crate) fn new(faces: Faces, spacing: Spacing) -> Self {
        Typesetter {
            faces,
            families: FAMILIES.map(|name| FontFamily::Name(name.into())),
            leading: 1.4 + 0.15 * spacing.line as f32,
            gap: 0.8 * BODY * spacing.paragraph as f32,
        }
    }

    /// The size `style` is set at. Headings step up by level. Code steps
    /// down, because a monospace face at the same nominal size looks larger
    /// than the text around it.
    pub(crate) fn size(&self, style: Style) -> f32 {
        let heading = match style.heading {
            1 => 1.55,
            2 => 1.3,
            3 => 1.15,
            _ => 1.0,
        };
        let mono = if style.mono { 0.86 } else { 1.0 };
        BODY * heading * mono
    }

    /// Line height as a multiple of size. Headings are set tighter than
    /// body text, since a heading is read as a unit rather than scanned
    /// line to line. Code is tighter too, and fixed, so the strokes of an
    /// ASCII diagram stay close enough to read as lines.
    fn leading(&self, style: Style) -> f32 {
        if style.heading > 0 {
            1.25
        } else if style.mono {
            1.3
        } else {
            self.leading
        }
    }

    /// The height of one line of body text, leading included.
    pub(crate) fn body_row(&self) -> f32 {
        BODY * self.leading
    }

    /// The text face at `size`, for chrome: the sidebar, the status bar, and
    /// the hint shown when nothing is open.
    pub(crate) fn chrome(&self, size: f32, bold: bool, italic: bool) -> FontId {
        FontId::new(size, self.families[usize::from(bold) | usize::from(italic) << 1].clone())
    }

    pub(crate) fn font_id(&self, style: Style) -> FontId {
        let family = if style.mono {
            FontFamily::Monospace
        } else {
            self.families[usize::from(style.bold) | usize::from(style.italic) << 1].clone()
        };
        FontId::new(self.size(style), family)
    }

    /// How `style` looks against `theme`. A quote marker is set in
    /// transparent ink, since the painter draws a rule in its place.
    pub(crate) fn format(&self, style: Style, theme: &Theme) -> TextFormat {
        let color = if style.rule { Color32::TRANSPARENT } else { ink(style.ink, theme) };
        TextFormat {
            font_id: self.font_id(style),
            color,
            background: if style.tint { theme.code_bg } else { Color32::TRANSPARENT },
            italics: style.italic && (style.mono || !self.faces.italic),
            underline: if style.underline { Stroke::new(1.0, color) } else { Stroke::NONE },
            ..Default::default()
        }
    }

    /// One line of spans as a single egui layout job, never wrapped: the
    /// `wrap` module has already decided where the line ends.
    pub(crate) fn job(&self, spans: &[Span], theme: &Theme) -> LayoutJob {
        let mut job = LayoutJob::default();
        for span in spans {
            job.append(&span.content, 0.0, self.format(span.style, theme));
        }
        job
    }

    /// The average advance of a lowercase letter in body text, which is what
    /// "a column 58 characters wide" means in points.
    pub(crate) fn char_width(&self, fonts: &mut FontsView<'_>) -> f32 {
        let font = self.font_id(Style::default());
        ('a'..='z').map(|c| fonts.glyph_width(&font, c)).sum::<f32>() / 26.0
    }

    /// The advance of one monospace character, the unit a sideways pan
    /// steps in.
    pub(crate) fn mono_width(&self, fonts: &mut FontsView<'_>) -> f32 {
        fonts.glyph_width(&self.font_id(Style::default().code()), '0')
    }

    /// A `Measure` for the `wrap` module, reading real glyph widths.
    pub(crate) fn metrics<'a, 'f>(&'a self, fonts: &'a mut FontsView<'f>) -> Metrics<'a, 'f> {
        Metrics { ts: self, fonts }
    }
}

/// The color an ink resolves to in `theme`.
pub(crate) fn ink(ink: Ink, theme: &Theme) -> Color32 {
    match ink {
        Ink::Body => theme.fg,
        Ink::Heading => theme.heading,
        Ink::Code => theme.code,
        Ink::Quote => theme.quote,
        Ink::Link => theme.link,
        Ink::Muted => theme.muted,
    }
}

/// egui's glyph metrics, seen through the typesetter's choice of face and
/// size. Kerning is left out of the widths, and kerning only ever tightens
/// a line, so a line measured this way never comes out wider when painted.
pub(crate) struct Metrics<'a, 'f> {
    ts: &'a Typesetter,
    fonts: &'a mut FontsView<'f>,
}

impl Measure for Metrics<'_, '_> {
    fn width(&mut self, text: &str, style: Style) -> f32 {
        let font = self.ts.font_id(style);
        text.chars().map(|c| self.fonts.glyph_width(&font, c)).sum()
    }

    fn row_height(&mut self, style: Style) -> f32 {
        self.ts.size(style) * self.ts.leading(style)
    }

    fn gap(&self) -> f32 {
        self.ts.gap
    }

    fn cell(&self) -> f32 {
        BODY * 0.6
    }

    fn heading_space(&self) -> f32 {
        self.ts.body_row() * 0.6
    }
}
