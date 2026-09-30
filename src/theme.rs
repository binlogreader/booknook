//! Color palettes.
//!
//! A theme is plain data. Nothing here has behavior, and nothing here
//! knows what a page or a sidebar is. The `ui` module decides what to
//! paint with each color, and the `markdown` module decides which role
//! each piece of text plays.
//!
//! `page` is deliberately a slightly different shade from `bg`. The
//! reading column is painted with `page` and everything around it with
//! `bg`, so the text sits on something that reads as a sheet of paper
//! rather than filling the whole window edge to edge.
//!
//! Most of these are not booknook's own inventions. Choosing colors that
//! stay comfortable for hours is a solved problem, and the solutions have
//! names: Solarized, Gruvbox, Catppuccin, and the rest of the palettes the
//! editor world has already converged on. Each entry below maps a
//! well-known palette's published values onto booknook's roles, using the
//! scheme's own darkest tone for the surround and its base for the page,
//! so the theme reads here the way it reads everywhere else.
//!
//! With one deliberate departure. Those palettes were designed for syntax
//! highlighting, where a saturated color carries meaning: this token is a
//! string, that one is a keyword. In prose it means nothing, and it still
//! takes the eye. That cost is not aesthetic. A saturated color against a
//! page of body text is a second focal demand, because the eye cannot
//! bring two widely separated wavelengths to the retina at once, and it
//! hunts between them. Reading it as distraction rather than as blur is
//! exactly right: that is what the hunting feels like.
//!
//! It compounds with density. Technical prose runs to one inline-code span
//! every twenty words or so, which turns an accent into a scattering of
//! bright flecks through running text, each one asking the eye to refocus.
//!
//! So every role here is held to a budget, measured rather than eyeballed:
//! saturation at or below ~25%, and contrast against the page between 60%
//! and 85% of the body text's, except headings, which may exceed the body.
//! Color marks structure, which is to say headings and the chrome outside
//! the page. Inside a paragraph, distinctions are carried by weight, by
//! italics, and by the shade of the page itself. Each palette keeps its own
//! hue and its own character; what changes is how loudly it speaks.

use eframe::egui::Color32;

pub(crate) struct Theme {
    pub(crate) name: &'static str,
    /// Everything behind and around the page: sidebar, gutter, status bar.
    pub(crate) bg: Color32,
    /// The reading column itself.
    pub(crate) page: Color32,
    /// Body text. Softened rather than pure white, because maximum
    /// contrast is tiring to read for any length of time.
    pub(crate) fg: Color32,
    pub(crate) heading: Color32,
    pub(crate) code: Color32,
    /// The shade behind inline code and code blocks. A step off `page`
    /// rather than a change of hue, so a code span is set apart the way a
    /// typesetter would do it, by tinting the paper under the words rather
    /// than by recoloring the words themselves. This is what lets `code`
    /// stay close to body ink instead of having to shout.
    pub(crate) code_bg: Color32,
    /// Chrome and punctuation the eye should skip: bullets, the spine,
    /// the status bar, unselected file names.
    pub(crate) muted: Color32,
    pub(crate) quote: Color32,
    pub(crate) link: Color32,
}

/// Cycled through with `t` while reading. Sepia comes first because it is
/// the quietest of them and the one to open on: a warm light page is the
/// standard recommendation for long sessions, and a light background
/// constricts the pupil, which deepens the eye's depth of focus and makes
/// focusing more forgiving. The rest follow, light before dark, since dark
/// earns its place in a dim room rather than as a default.
///
/// A `static` rather than a `const` so that `&THEMES[i]` borrows for
/// `'static` and callers never have to thread a lifetime through.
pub(crate) static THEMES: [Theme; 6] = [
    // Kindle's sepia: a warm cream page with soft brown ink, the classic
    // e-reader setting for long sessions and for reading at night without the
    // glare of a white background. The page is the exact tone Kindle uses,
    // and every other value is pulled onto the same warm brown axis so
    // nothing on the page reads as gray against it. The link was the one
    // loud note here, a near-orange at half saturation; it now sits on the
    // same brown as the rest and is told apart by its underline.
    Theme {
        name: "Sepia",
        bg: Color32::from_rgb(228, 216, 191),
        page: Color32::from_rgb(251, 240, 217),
        fg: Color32::from_rgb(91, 70, 54),
        heading: Color32::from_rgb(58, 48, 40),
        code: Color32::from_rgb(104, 86, 70),
        code_bg: Color32::from_rgb(240, 227, 200),
        muted: Color32::from_rgb(166, 146, 118),
        quote: Color32::from_rgb(120, 100, 82),
        link: Color32::from_rgb(112, 92, 74),
    },
    // Flexoki's paper side: an off-white the tone of unbleached paper with
    // warm near-black ink, designed, like its dark twin, for prose first.
    // Flexoki's own orange and cyan were the most saturated pair in the set,
    // and against a near-white page they were the worst case of the two
    // wavelengths pulling apart. Both are replaced by the scheme's warm
    // grays; the paper and the ink, which are what make Flexoki, are intact.
    Theme {
        name: "Flexoki Light",
        bg: Color32::from_rgb(242, 240, 229),
        page: Color32::from_rgb(255, 252, 240),
        fg: Color32::from_rgb(52, 51, 49),
        heading: Color32::from_rgb(16, 15, 15),
        code: Color32::from_rgb(84, 82, 79),
        code_bg: Color32::from_rgb(242, 239, 226),
        muted: Color32::from_rgb(135, 133, 128),
        quote: Color32::from_rgb(86, 85, 82),
        link: Color32::from_rgb(84, 82, 78),
    },
    // Light, for reading in daylight. This is the one that actually looks
    // like an e-ink screen, since e-ink is a reflective surface and is at
    // its best bright. It was already the quietest of the six and needed
    // the least done to it.
    Theme {
        name: "Paper",
        bg: Color32::from_rgb(222, 216, 202),
        page: Color32::from_rgb(240, 235, 222),
        fg: Color32::from_rgb(58, 54, 48),
        heading: Color32::from_rgb(28, 26, 22),
        code: Color32::from_rgb(96, 90, 80),
        code_bg: Color32::from_rgb(230, 224, 210),
        muted: Color32::from_rgb(140, 132, 118),
        quote: Color32::from_rgb(96, 90, 80),
        link: Color32::from_rgb(88, 84, 76),
    },
    // Cool and dark, in the Tokyo Night family. The blues are pulled well
    // back from where a syntax highlighter would put them, which is why
    // this was already the least distracting of the dark three.
    Theme {
        name: "Tokyo Night",
        bg: Color32::from_rgb(22, 22, 30),
        page: Color32::from_rgb(26, 27, 38),
        fg: Color32::from_rgb(200, 206, 224),
        heading: Color32::from_rgb(230, 232, 238),
        code: Color32::from_rgb(160, 168, 184),
        code_bg: Color32::from_rgb(35, 36, 48),
        muted: Color32::from_rgb(92, 96, 120),
        quote: Color32::from_rgb(156, 162, 180),
        link: Color32::from_rgb(152, 166, 182),
    },
    // Catppuccin's dark flagship: soft pastels on a blue-charcoal base.
    // Mantle for the surround, Base for the page, Text for the ink. The
    // published Lavender, Peach, and Blue all sit near full saturation,
    // which is right in an editor and wrong in a paragraph, so each is kept
    // on its own hue and brought down toward the Text tone.
    Theme {
        name: "Catppuccin Mocha",
        bg: Color32::from_rgb(24, 24, 37),
        page: Color32::from_rgb(30, 30, 46),
        fg: Color32::from_rgb(205, 214, 244),
        heading: Color32::from_rgb(224, 226, 234),
        code: Color32::from_rgb(178, 176, 180),
        code_bg: Color32::from_rgb(40, 40, 58),
        muted: Color32::from_rgb(112, 116, 138),
        quote: Color32::from_rgb(164, 170, 192),
        link: Color32::from_rgb(160, 172, 196),
    },
    // Rosé Pine's main variant: dusk purples and soft rose on near-black.
    // Base and Surface carry the page. Iris and Gold are the scheme's
    // signature, and both are held here at a fraction of their published
    // chroma: enough that the theme still reads as Rosé Pine, not enough to
    // take the eye off the line being read.
    Theme {
        name: "Rosé Pine",
        bg: Color32::from_rgb(25, 23, 36),
        page: Color32::from_rgb(31, 29, 46),
        fg: Color32::from_rgb(224, 222, 244),
        heading: Color32::from_rgb(230, 227, 236),
        code: Color32::from_rgb(190, 184, 180),
        code_bg: Color32::from_rgb(42, 39, 58),
        muted: Color32::from_rgb(114, 110, 138),
        quote: Color32::from_rgb(174, 170, 190),
        link: Color32::from_rgb(170, 184, 192),
    },
];
