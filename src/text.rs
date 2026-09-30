//! Styled text, described by what it is rather than by how it looks.
//!
//! The parsers say what a run of text is: body or heading, emphasized or
//! not, prose or code. They do not decide its color, its face, or its size.
//! Those depend on the palette, the typefaces installed on this machine, and
//! the zoom level, all of which can change while a document is open, so the
//! decision is made at paint time by the `typeset` module. A theme switch or
//! a change of text size therefore never re-parses anything.
//!
//! The builder methods on `Style` read the way ratatui's did, which keeps
//! the parsers' style stacks as short as they were: push the current style,
//! derive a new one with `style.bold()` or `style.ink(Ink::Link)`, and pop
//! it back when the tag closes.

/// Which of the theme's inks a run of text is set in. The names match the
/// roles in `Theme`, which is where each resolves to an actual color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Ink {
    #[default]
    Body,
    Heading,
    Code,
    Quote,
    Link,
    Muted,
}

/// Everything the parsers know about how a run of text should be set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct Style {
    pub(crate) ink: Ink,
    pub(crate) bold: bool,
    pub(crate) italic: bool,
    pub(crate) underline: bool,
    /// Set in the monospace face, for code.
    pub(crate) mono: bool,
    /// The paper behind the words is tinted, the way code is set apart.
    pub(crate) tint: bool,
    /// A heading level from 1 to 6, or 0 for body size.
    pub(crate) heading: u8,
    /// A quote marker. The text holds the marker's place on the line, and
    /// the painter draws a vertical rule down the block's edge in its
    /// stead, which a proportional face cannot draw with a glyph.
    pub(crate) rule: bool,
}

impl Style {
    pub(crate) fn ink(mut self, ink: Ink) -> Self {
        self.ink = ink;
        self
    }

    pub(crate) fn bold(mut self) -> Self {
        self.bold = true;
        self
    }

    pub(crate) fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    pub(crate) fn underlined(mut self) -> Self {
        self.underline = true;
        self
    }

    /// Code: monospace, in the code ink, on tinted paper.
    pub(crate) fn code(mut self) -> Self {
        self.mono = true;
        self.tint = true;
        self.ink = Ink::Code;
        self
    }

    /// A heading at `level`: larger, bold, and in the heading ink.
    pub(crate) fn heading(mut self, level: u8) -> Self {
        self.heading = level;
        self.bold = true;
        self.ink = Ink::Heading;
        self
    }

    /// A quote marker, drawn as a rule rather than as text.
    pub(crate) fn rule(mut self) -> Self {
        self.rule = true;
        self.ink = Ink::Muted;
        self
    }
}

/// A run of text in a single style. A word can span several of these, as
/// the `wrap` module explains.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Span {
    pub(crate) content: String,
    pub(crate) style: Style,
}

impl Span {
    pub(crate) fn raw(content: impl Into<String>) -> Self {
        Span { content: content.into(), style: Style::default() }
    }

    pub(crate) fn styled(content: impl Into<String>, style: Style) -> Self {
        Span { content: content.into(), style }
    }
}
