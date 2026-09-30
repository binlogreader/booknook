//! Word-wrapping parsed blocks to a column width, and breaking the result
//! into pages a typesetter would sign off on.
//!
//! egui can wrap a paragraph by itself, but only to a single width, which
//! rules out a hanging indent for a wrapped list item, and it keeps the rows
//! it produces to itself, which rules out choosing where pages end. This
//! module does the wrapping instead, one word at a time, and hands back
//! every row with its height and its position on the line.
//!
//! Owning the rows is what makes real page breaks possible. `layout` tags
//! each row with whether a page may end after it, and `paginate` walks
//! those rows page by page, ending a page a line or two early whenever the
//! arithmetic break would strand something: a heading at a page bottom, a
//! paragraph's first line left behind, its last line alone overleaf. A
//! short page is how books solve this too; the space left at its foot is
//! invisible, while a bad break is not.
//!
//! Nothing here knows about fonts. Widths and heights come from a `Measure`,
//! which the `typeset` module implements against egui's real glyph metrics
//! and the tests below implement as a fixed grid, one unit per character and
//! per row, so pagination can be checked by counting.

use std::ops::Range;

use unicode_width::UnicodeWidthChar;

use crate::markdown::{Heading, RenderLine, TableBlock};
use crate::text::{Span, Style};

/// One word, as a run of styled pieces.
///
/// A word is not the same thing as a span. The markdown parser emits smart
/// punctuation as its own event, so the word `country's` arrives as three
/// separate spans: `country`, then `’`, then `s`. Treating each span as a
/// word would put a space on either side of the apostrophe. A word is
/// therefore whatever sits between two runs of whitespace, no matter how
/// many spans and styles it crosses.
type Word = Vec<(String, Style)>;

/// How the page is spaced vertically, as two independent steps.
///
/// These are two different numbers on purpose. If the gap inside a
/// paragraph matched the gap between paragraphs, every line would look
/// like its own paragraph, and the page would lose all of its structure.
/// `line` steps the leading, the distance from one line's baseline to the
/// next; `paragraph` steps the extra air between blocks. What a step is
/// worth in points is the `typeset` module's decision.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Spacing {
    pub(crate) line: u16,
    pub(crate) paragraph: u16,
}

/// What layout needs to know about the type it is setting.
///
/// The methods take `&mut self` because egui's glyph lookups do: measuring a
/// glyph for the first time caches it.
pub(crate) trait Measure {
    /// How far `text`, set in `style`, advances along the line, in points.
    fn width(&mut self, text: &str, style: Style) -> f32;
    /// The height of one line of text in `style`, leading included.
    fn row_height(&mut self, style: Style) -> f32;
    /// The air between one block and the next. Zero turns it off.
    fn gap(&self) -> f32;
    /// The width of one indent cell, the unit the parsers count nesting in.
    fn cell(&self) -> f32;
    /// Extra air above a heading, on top of the ordinary gap, so a heading
    /// sits closer to the text it introduces than to the text it ends.
    fn heading_space(&self) -> f32 {
        0.0
    }
}

/// One cell of a table row: where it starts, how wide its column is, and
/// what it holds. Content that cannot wrap down to the column, such as one
/// long unbreakable word, is clipped to `width` when painted.
#[derive(Clone, Debug)]
pub(crate) struct Cell {
    pub(crate) x: f32,
    pub(crate) width: f32,
    pub(crate) spans: Vec<Span>,
}

/// What one row of the page shows.
#[derive(Clone, Debug)]
pub(crate) enum RowKind {
    /// Structural air: a paragraph gap, or the space above a heading.
    Blank,
    /// A line of wrapped prose, starting `x` points in from the column's
    /// left edge. `rule` is where a quote's vertical rule runs down the row,
    /// when the block is a quotation.
    Text { x: f32, spans: Vec<Span>, rule: Option<f32> },
    /// A line of code or a code block's label, never rewrapped. `width` is
    /// its full natural width, which can exceed the column: the painter
    /// clips it at the column's edge and pans it sideways on request.
    Verbatim { span: Span, width: f32 },
    /// One line of a table row, cell by cell.
    Cells(Vec<Cell>),
    /// The rule under a table's header, as each column's left edge and
    /// width.
    TableRule { columns: Vec<(f32, f32)>, style: Style },
}

/// One row of the laid-out document, with its break hints.
///
/// `keep_with_next` means a page must not end after this row: the first
/// line of a paragraph, any line of a heading, the rule under a table
/// header. `blank` marks structural air, which a new page sheds from its
/// top; a book never opens a page with blank leading.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub(crate) kind: RowKind,
    pub(crate) height: f32,
    keep_with_next: bool,
    blank: bool,
}

/// A laid-out document: its rows, where each input block begins among them,
/// and the widest line of code, which is the bound sideways panning clamps
/// against.
///
/// `block_rows` has one entry per input block, giving the row at which that
/// block's content starts. It is what lets the reader jump to a heading:
/// the sidebar knows a heading's block index, and this maps that index to a
/// row, which pagination maps to a page.
pub(crate) struct Laid {
    pub(crate) rows: Vec<Row>,
    pub(crate) block_rows: Vec<usize>,
    pub(crate) code_width: f32,
}

/// A document broken into pages. Every page is a run of consecutive rows,
/// and the rows between two pages are air that the second page shed from
/// its top.
pub(crate) struct Pages {
    pub(crate) rows: Vec<Row>,
    pub(crate) pages: Vec<Range<usize>>,
    block_pages: Vec<usize>,
    pub(crate) code_width: f32,
}

impl Pages {
    /// How many pages there are. An empty document still has one, blank,
    /// so the reader always has a page to stand on.
    pub(crate) fn count(&self) -> usize {
        self.pages.len().max(1)
    }

    /// The rows shown on `page`, or none past the end.
    pub(crate) fn rows_on(&self, page: usize) -> &[Row] {
        match self.pages.get(page) {
            Some(range) => &self.rows[range.clone()],
            None => &[],
        }
    }

    /// The page on which block `block` begins.
    pub(crate) fn page_of_block(&self, block: usize) -> usize {
        self.block_pages.get(block).copied().unwrap_or(0)
    }
}

/// The rows of a document as they are produced, each paired with its break
/// hints. Blank rows inherit the keep flag of the row above them: a break
/// after trailing air reads, on the page, as a break after the content
/// itself, so whatever that content forbids its air must forbid too. This
/// inheritance is what carries a heading's keep-with-next across the gap
/// under it and onto the next paragraph without any special casing.
#[derive(Default)]
struct Rows {
    rows: Vec<Row>,
}

impl Rows {
    fn content(&mut self, kind: RowKind, height: f32, keep_with_next: bool) {
        self.rows.push(Row { kind, height, keep_with_next, blank: false });
    }

    fn blank(&mut self, height: f32) {
        let keep_with_next = self.rows.last().is_some_and(|r| r.keep_with_next);
        self.rows.push(Row { kind: RowKind::Blank, height, keep_with_next, blank: true });
    }

    fn len(&self) -> usize {
        self.rows.len()
    }

    fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
}

/// Lay out parsed blocks at a column `width` points wide. Every prose block
/// is word-wrapped. Verbatim blocks, such as code and ASCII diagrams, keep
/// their exact shape on a single row each; one wider than the column is
/// clipped when painted rather than soft-wrapped into a second row, which
/// would break its alignment.
///
/// `headings` tells the row-tagging which prose blocks are headings, since a
/// heading must never be the last thing on a page. The result is not yet
/// page-shaped: hand it to `paginate` with a page height to get pages whose
/// boundaries all fall at defensible breaks.
pub(crate) fn layout(blocks: &[RenderLine], headings: &[Heading], width: f32, measure: &mut impl Measure) -> Laid {
    // Heading block indices arrive in document order, so membership is a
    // binary search rather than a set build per layout.
    let heading_blocks: Vec<usize> = headings.iter().map(|h| h.block).collect();
    let gap = measure.gap();
    let heading_space = measure.heading_space();

    let mut out = Rows::default();
    let mut block_rows: Vec<usize> = Vec::with_capacity(blocks.len());
    let mut code_width = 0.0f32;
    let mut prev_was_gap = false;

    for (index, block) in blocks.iter().enumerate() {
        let is_heading = heading_blocks.binary_search(&index).is_ok();
        if is_heading && heading_space > 0.0 && !out.is_empty() {
            out.blank(heading_space);
        }
        // The row this block starts on is wherever output currently ends. A
        // collapsed gap adds nothing and simply reports the current row, so
        // the entry stays aligned with `blocks` one-for-one.
        block_rows.push(out.len());
        match block {
            // Two blocks in a row can each ask for a gap, for instance a
            // paragraph ending just before its enclosing list does. Only
            // the first one gets to draw it.
            RenderLine::Gap => {
                if !prev_was_gap && !out.is_empty() && gap > 0.0 {
                    out.blank(gap);
                }
                prev_was_gap = true;
                continue;
            }
            RenderLine::Verbatim(span) => {
                let line_width = measure.width(&span.content, span.style);
                code_width = code_width.max(line_width);
                let height = measure.row_height(span.style);
                out.content(RowKind::Verbatim { span: span.clone(), width: line_width }, height, false);
            }
            RenderLine::Table(table) => layout_table(&mut out, table, width, measure),
            RenderLine::Prose { spans, indent, hang } => {
                let cell = measure.cell();
                let indent_x = *indent as f32 * cell;
                // Continuation rows line up under the first row's text, past
                // whatever marker opens it, so the marker has to be measured
                // in the face it is actually set in.
                let lead = lead_in(spans, hang.saturating_sub(*indent) as usize);
                let hang_x = indent_x + lead.iter().map(|(text, style)| measure.width(text, *style)).sum::<f32>();
                let rule = lead.iter().any(|(_, style)| style.rule).then_some(indent_x + cell * 0.3);

                let wrapped = wrap_prose(spans, indent_x, hang_x, width, measure);
                let count = wrapped.len();
                for (i, row) in wrapped.into_iter().enumerate() {
                    // A heading holds onto whatever follows it. Body prose
                    // guards its own edges: a break after the first row would
                    // orphan it, and a break after the second-to-last would
                    // widow the last. Two- and three-line paragraphs have no
                    // row that escapes both rules, so they never split at
                    // all, which is exactly what a typesetter would do.
                    let keep = if is_heading { true } else { count > 1 && (i == 0 || i == count - 2) };
                    let height = row_height(&row, measure);
                    let x = if i == 0 { indent_x } else { hang_x };
                    out.content(RowKind::Text { x, spans: row, rule }, height, keep);
                }
            }
        }
        prev_was_gap = false;
    }
    Laid { rows: out.rows, block_rows, code_width }
}

/// Break laid-out rows into pages no taller than `height`, ending a page
/// early whenever the arithmetic boundary falls somewhere `keep_with_next`
/// forbids.
///
/// Each new page also sheds structural blanks from its top, so no page
/// opens with the tail of the previous page's paragraph gap.
///
/// One run of glued rows can exceed a whole page, for instance a heading
/// atop a long paragraph in a very short window. There is no good break
/// inside such a run, so the page is filled to the brim and the break is
/// taken as it falls: a full page is the least bad option once every option
/// is bad. A single row taller than the page gets a page of its own.
pub(crate) fn paginate(laid: Laid, height: f32) -> Pages {
    // A hair of tolerance, so rows whose heights sum to exactly the page do
    // not spill onto the next one through floating-point error.
    let limit = height + 0.01;
    let rows = laid.rows;
    let total = rows.len();
    let mut pages: Vec<Range<usize>> = Vec::new();
    let mut i = 0;

    while i < total {
        // A fresh page starts flush: leading air is dropped.
        while i < total && rows[i].blank {
            i += 1;
        }
        if i >= total {
            break;
        }

        let mut end = i + 1;
        let mut used = rows[i].height;
        while end < total && used + rows[end].height <= limit {
            used += rows[end].height;
            end += 1;
        }
        if end < total {
            // The page is full with more to come: walk the break upward
            // until it sits after a row that allows one. Landing back at the
            // page's own start means the entire page is one glued run, and
            // the original, full-page break stands.
            let mut candidate = end;
            while candidate > i && rows[candidate - 1].keep_with_next {
                candidate -= 1;
            }
            if candidate > i {
                end = candidate;
            }
        }
        pages.push(i..end);
        i = end;
    }

    let block_pages = laid.block_rows.iter().map(|&row| page_of_row(&pages, row)).collect();
    Pages { rows, pages, block_pages, code_width: laid.code_width }
}

/// The page a row belongs to. A row between two pages is air the later one
/// shed from its top, so it belongs to that later page: a heading whose gap
/// fell on a boundary resolves to the page the heading itself opens.
fn page_of_row(pages: &[Range<usize>], row: usize) -> usize {
    let after = pages.partition_point(|page| page.start <= row);
    if after == 0 {
        return 0;
    }
    let page = after - 1;
    if row >= pages[page].end && after < pages.len() { after } else { page }
}

/// The opening `columns` character cells of a block's text, as styled
/// pieces: the list marker or quote marker whose width a hanging indent
/// has to clear.
fn lead_in(spans: &[Span], columns: usize) -> Word {
    let mut pieces: Word = Vec::new();
    let mut used = 0usize;
    for span in spans {
        if used >= columns {
            break;
        }
        let mut kept = String::new();
        for ch in span.content.chars() {
            if used >= columns {
                break;
            }
            kept.push(ch);
            used += UnicodeWidthChar::width(ch).unwrap_or(0);
        }
        if !kept.is_empty() {
            pieces.push((kept, span.style));
        }
    }
    pieces
}

/// Break a block's spans into words, where a word is a run of non-whitespace
/// text that may cross span boundaries and carry more than one style.
fn split_words(spans: &[Span]) -> Vec<Word> {
    let mut words: Vec<Word> = Vec::new();
    let mut current: Word = Vec::new();

    for span in spans {
        let style = span.style;
        let mut rest: &str = span.content.as_str();
        while !rest.is_empty() {
            // Walk one run at a time, alternating between whitespace and
            // non-whitespace, so a word can be assembled across as many
            // spans as it takes.
            let leading_is_space = rest.starts_with(char::is_whitespace);
            let end = rest.find(|c: char| c.is_whitespace() != leading_is_space).unwrap_or(rest.len());
            let (chunk, tail) = rest.split_at(end);

            if leading_is_space {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            } else {
                current.push((chunk.to_string(), style));
            }
            rest = tail;
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// The style a space between two words is set in. Between two pieces of
/// one style, a link or a code span running across the space, it is that
/// style, so an underline or a tint carries across unbroken. Otherwise it is
/// plain, at the size of the larger neighbor, so the gaps in a heading are
/// heading-sized.
fn space_style(before: Style, after: Style) -> Style {
    if before == after {
        before
    } else {
        Style { heading: before.heading.max(after.heading), ..Style::default() }
    }
}

/// Word-wrap one block's spans into rows, keeping each word's styling. The
/// first row starts `indent` points in and every row after it `hang` points
/// in, so a wrapped list item's second line lands under its text instead of
/// under the bullet. The returned rows carry no indent of their own; the
/// caller places them.
fn wrap_prose(spans: &[Span], indent: f32, hang: f32, width: f32, measure: &mut impl Measure) -> Vec<Vec<Span>> {
    let words = split_words(spans);
    if words.is_empty() {
        return vec![Vec::new()];
    }

    let mut rows: Vec<Vec<Span>> = Vec::new();
    let mut current: Vec<Span> = Vec::new();
    let mut current_width = 0.0f32;
    let mut last_style = Style::default();

    for word in words {
        let this_width: f32 = word.iter().map(|(text, style)| measure.width(text, *style)).sum();
        let row_start = if rows.is_empty() { indent } else { hang };
        let available = (width - row_start).max(1.0);
        let gap_style = space_style(last_style, word[0].1);
        let space = measure.width(" ", gap_style);

        if !current.is_empty() && current_width + space + this_width > available {
            rows.push(std::mem::take(&mut current));
            current_width = 0.0;
        }
        if !current.is_empty() {
            current.push(Span::styled(" ", gap_style));
            current_width += space;
        }
        current_width += this_width;
        for (text, style) in word {
            last_style = style;
            current.push(Span::styled(text, style));
        }
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

/// The height of a row of spans: that of its tallest style. Inline code is
/// set a little smaller than the prose around it, so it never makes its
/// line taller than its neighbors.
fn row_height(spans: &[Span], measure: &mut impl Measure) -> f32 {
    let body = measure.row_height(Style::default());
    spans.iter().map(|span| measure.row_height(span.style)).fold(body, f32::max)
}

/// Indent cells between adjacent table columns.
const TABLE_GUTTER: f32 = 2.0;

/// Lay a table out at the page's width: size the columns, then set the
/// header, a rule under it, and every row. Cells word-wrap inside their own
/// column, so a wide table gets taller rather than spilling off the page.
///
/// Break hints: a page may end between logical rows but never inside one,
/// since half a wrapped cell is gibberish, and never right after the header
/// or its rule, which would strand the column names away from every value
/// they name.
fn layout_table(out: &mut Rows, table: &TableBlock, width: f32, measure: &mut impl Measure) {
    let ncols = table.rows.iter().map(Vec::len).chain(std::iter::once(table.header.len())).max().unwrap_or(0);
    if ncols == 0 {
        return;
    }

    // A column's natural width is its widest cell, set on one line.
    let mut natural = vec![0.0f32; ncols];
    for row in std::iter::once(&table.header).chain(&table.rows) {
        for (col, cell) in row.iter().enumerate() {
            let cell_width: f32 = cell.iter().map(|s| measure.width(&s.content, s.style)).sum();
            natural[col] = natural[col].max(cell_width);
        }
    }

    let gutter = TABLE_GUTTER * measure.cell();
    let avail = (width - gutter * (ncols - 1) as f32).max(ncols as f32 * measure.cell());
    let widths = fit_columns(&natural, avail);
    let mut xs = Vec::with_capacity(ncols);
    let mut x = 0.0f32;
    for col_width in &widths {
        xs.push(x);
        x += col_width + gutter;
    }

    if !table.header.is_empty() {
        // The header and its rule hold onto the first data row, so a page
        // never ends on column names with nothing under them.
        render_table_row(out, &table.header, &xs, &widths, true, measure);
        let columns = xs.iter().copied().zip(widths.iter().copied()).collect();
        let height = measure.row_height(table.rule_style) * 0.5;
        out.content(RowKind::TableRule { columns, style: table.rule_style }, height, true);
    }
    for row in &table.rows {
        render_table_row(out, row, &xs, &widths, false, measure);
    }
}

/// Decide each column's width when the table must be squeezed. Columns that
/// fit inside an equal share of the available width keep their natural
/// size, and the space they leave unused is re-divided among the wider
/// ones, so a short id column never pays for a long description column.
fn fit_columns(natural: &[f32], avail: f32) -> Vec<f32> {
    if natural.iter().sum::<f32>() <= avail {
        return natural.to_vec();
    }
    let mut widths = vec![0.0f32; natural.len()];
    let mut pending: Vec<usize> = (0..natural.len()).collect();
    let mut left = avail;
    loop {
        let fair = left / pending.len() as f32;
        let fitting: Vec<usize> = pending.iter().copied().filter(|&i| natural[i] <= fair).collect();
        if fitting.is_empty() {
            // Every remaining column wants more than its share: split what
            // is left evenly.
            let share = (left / pending.len() as f32).max(1.0);
            for &i in &pending {
                widths[i] = share;
            }
            return widths;
        }
        for &i in &fitting {
            widths[i] = natural[i];
            left -= natural[i];
        }
        pending.retain(|i| !fitting.contains(i));
        if pending.is_empty() {
            return widths;
        }
    }
}

/// Set one logical table row as however many lines its tallest cell needs,
/// each cell word-wrapped to its own column.
///
/// Every line but the logical row's last is glued to the next, so a page can
/// never cut through the middle of a wrapped cell. `keep_all` glues the last
/// one too, for the header, which must never end a page.
fn render_table_row(
    out: &mut Rows,
    cells: &[Vec<Span>],
    xs: &[f32],
    widths: &[f32],
    keep_all: bool,
    measure: &mut impl Measure,
) {
    let empty: &[Span] = &[];
    let wrapped: Vec<Vec<Vec<Span>>> = widths
        .iter()
        .enumerate()
        .map(|(col, col_width)| {
            let spans = cells.get(col).map(Vec::as_slice).unwrap_or(empty);
            wrap_prose(spans, 0.0, 0.0, *col_width, measure)
        })
        .collect();
    let height = wrapped.iter().map(Vec::len).max().unwrap_or(0);
    for line in 0..height {
        let row: Vec<Cell> = wrapped
            .iter()
            .enumerate()
            .map(|(col, lines)| Cell { x: xs[col], width: widths[col], spans: lines.get(line).cloned().unwrap_or_default() })
            .collect();
        let row_h = row.iter().map(|cell| self::row_height(&cell.spans, measure)).fold(0.0, f32::max);
        out.content(RowKind::Cells(row), row_h, keep_all || line + 1 < height);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    /// A fixed grid: every character one unit wide, every row one unit
    /// tall, one unit of air per paragraph step. This is the terminal the
    /// app used to run in, and it makes pagination checkable by counting.
    struct Grid {
        gap: f32,
    }

    impl Measure for Grid {
        fn width(&mut self, text: &str, _style: Style) -> f32 {
            UnicodeWidthStr::width(text) as f32
        }
        fn row_height(&mut self, _style: Style) -> f32 {
            1.0
        }
        fn gap(&self) -> f32 {
            self.gap
        }
        fn cell(&self) -> f32 {
            1.0
        }
    }

    /// One unit of air between paragraphs, the default spacing.
    fn tight() -> Grid {
        Grid { gap: 1.0 }
    }

    fn text_of(spans: &[Span]) -> String {
        spans.iter().map(|s| s.content.as_str()).collect()
    }

    /// A row as it would look on the grid, with blank rows shown as empty
    /// strings and table cells padded out to where their columns start.
    fn show(row: &Row) -> String {
        match &row.kind {
            RowKind::Blank => String::new(),
            RowKind::Text { spans, .. } => text_of(spans),
            RowKind::Verbatim { span, .. } => span.content.clone(),
            RowKind::Cells(cells) => {
                let mut line = String::new();
                for cell in cells {
                    let text = text_of(&cell.spans);
                    if !text.is_empty() {
                        while line.chars().count() < cell.x as usize {
                            line.push(' ');
                        }
                        line.push_str(&text);
                    }
                }
                line
            }
            RowKind::TableRule { columns, .. } => {
                let mut line = String::new();
                for (x, width) in columns {
                    while line.chars().count() < *x as usize {
                        line.push(' ');
                    }
                    line.push_str(&"─".repeat(*width as usize));
                }
                line
            }
        }
    }

    fn rows_of(laid: &Laid) -> Vec<String> {
        laid.rows.iter().map(show).collect()
    }

    /// Every page's rows, so a whole pagination can be asserted at a glance.
    fn pages_of(pages: &Pages) -> Vec<Vec<String>> {
        (0..pages.pages.len()).map(|p| pages.rows_on(p).iter().map(show).collect()).collect()
    }

    fn wrap_to_strings(spans: &[Span], width: f32) -> Vec<String> {
        wrap_prose(spans, 0.0, 0.0, width, &mut tight()).iter().map(|row| text_of(row)).collect()
    }

    fn prose(text: &str) -> RenderLine {
        RenderLine::Prose { spans: vec![Span::raw(text)], indent: 0, hang: 0 }
    }

    /// The parser emits smart punctuation as its own span, so an
    /// apostrophe splits `country's` into three. It must still render as
    /// one word, with no spaces introduced around the apostrophe.
    #[test]
    fn joins_words_split_across_spans() {
        let spans = vec![Span::raw("the country"), Span::raw("’"), Span::raw("s fight")];
        assert_eq!(wrap_to_strings(&spans, 40.0), vec!["the country’s fight"]);
    }

    #[test]
    fn joins_currency_split_across_spans() {
        let spans = vec![Span::raw("at "), Span::raw("$"), Span::raw("78.02")];
        assert_eq!(wrap_to_strings(&spans, 40.0), vec!["at $78.02"]);
    }

    #[test]
    fn collapses_runs_of_whitespace_between_words() {
        let spans = vec![Span::raw("a  b"), Span::raw("   c")];
        assert_eq!(wrap_to_strings(&spans, 40.0), vec!["a b c"]);
    }

    #[test]
    fn wraps_at_the_column_width() {
        let spans = vec![Span::raw("alpha beta gamma")];
        assert_eq!(wrap_to_strings(&spans, 10.0), vec!["alpha beta", "gamma"]);
    }

    /// Continuation rows start at the hang, clear of the bullet, so wrapped
    /// list items line up under their own text. The hang is measured from
    /// the marker itself, so it holds whatever face the marker is set in.
    #[test]
    fn hanging_indent_applies_only_after_the_first_row() {
        let blocks = vec![RenderLine::Prose { spans: vec![Span::raw("• alpha beta gamma")], indent: 0, hang: 2 }];
        let laid = layout(&blocks, &[], 10.0, &mut tight());
        assert_eq!(rows_of(&laid), vec!["• alpha", "beta", "gamma"]);
        let xs: Vec<f32> = laid
            .rows
            .iter()
            .map(|row| match row.kind {
                RowKind::Text { x, .. } => x,
                _ => f32::NAN,
            })
            .collect();
        assert_eq!(xs, vec![0.0, 2.0, 2.0]);
    }

    /// A quotation's marker becomes a rule down every row of the block,
    /// continuation rows included, so the quote reads as one piece.
    #[test]
    fn a_quote_marker_rules_every_row_of_its_block() {
        let spans = vec![Span::styled("┃ ", Style::default().rule()), Span::raw("alpha beta gamma")];
        let laid = layout(&[RenderLine::Prose { spans, indent: 0, hang: 2 }], &[], 10.0, &mut tight());
        assert!(laid.rows.len() > 1, "the quote should wrap");
        assert!(laid.rows.iter().all(|row| matches!(row.kind, RowKind::Text { rule: Some(_), .. })));
    }

    /// A heading whose section would start overleaf moves to the next page
    /// whole, taking at least the first lines of its paragraph with it. The
    /// page it leaves behind ends short.
    #[test]
    fn a_heading_is_never_stranded_at_a_page_bottom() {
        // One row of filler, a gap, a heading, a gap, then a two-line
        // paragraph, which is atomic. At a page height of 4, the arithmetic
        // break lands right after the heading's gap.
        let blocks = vec![prose("filler"), RenderLine::Gap, prose("Heading"), RenderLine::Gap, prose("alpha beta")];
        let headings = [Heading { level: 1, text: "Heading".into(), block: 2 }];
        let pages = paginate(layout(&blocks, &headings, 5.0, &mut tight()), 4.0);

        assert_eq!(
            pages_of(&pages),
            vec![
                // Page one: the filler and a short page.
                vec!["filler", ""],
                // Page two: the heading with its whole paragraph.
                vec!["Heading", "", "alpha", "beta"],
            ]
        );
        // The heading's block must follow it to its new page.
        assert_eq!(pages.page_of_block(2), 1);
    }

    /// A paragraph never leaves its first line at the bottom of a page. If
    /// only one line would fit, the whole paragraph waits for the next page.
    #[test]
    fn a_paragraph_first_line_is_never_orphaned() {
        // Two rows of filler (an atomic two-line paragraph), a gap, then a
        // four-line paragraph. At a height of 4 the arithmetic break falls
        // after the big paragraph's first line.
        let blocks = vec![prose("fill one"), RenderLine::Gap, prose("alpha beta gamma delta")];
        let pages = paginate(layout(&blocks, &[], 5.0, &mut tight()), 4.0);

        assert_eq!(pages_of(&pages), vec![vec!["fill", "one", ""], vec!["alpha", "beta", "gamma", "delta"]]);
    }

    /// A paragraph never sends its last line alone onto the next page: the
    /// break backs up one line so at least two travel together.
    #[test]
    fn a_paragraph_last_line_is_never_widowed() {
        // A four-line paragraph at a height of 3: the arithmetic break
        // would leave "delta" alone overleaf, so "gamma" goes with it.
        let blocks = vec![prose("alpha beta gamma delta")];
        let pages = paginate(layout(&blocks, &[], 5.0, &mut tight()), 3.0);

        assert_eq!(pages_of(&pages), vec![vec!["alpha", "beta"], vec!["gamma", "delta"]]);
    }

    /// A gap that lands on a page boundary is simply consumed: the next page
    /// starts flush with real content, never with leftover air, and no blank
    /// page is manufactured along the way.
    #[test]
    fn a_new_page_starts_flush_with_content() {
        let blocks = vec![prose("alpha"), RenderLine::Gap, prose("beta")];
        let pages = paginate(layout(&blocks, &[], 10.0, &mut tight()), 1.0);

        assert_eq!(pages_of(&pages), vec![vec!["alpha"], vec!["beta"]]);
        // The gap block resolves to the page it vanished into the top of.
        let block_pages: Vec<usize> = (0..3).map(|b| pages.page_of_block(b)).collect();
        assert_eq!(block_pages, vec![0, 1, 1]);
    }

    /// A logical table row whose cells wrapped to two lines crosses pages
    /// whole, and the header and rule travel with the first data row rather
    /// than ending a page as a title with nothing under it.
    #[test]
    fn a_table_never_splits_mid_cell_or_after_its_header() {
        let table = TableBlock {
            header: vec![cell("id"), cell("meaning")],
            rows: vec![vec![cell("a"), cell("alpha beta gamma")]],
            rule_style: Style::default(),
        };
        // Width 14 squeezes the second column to 10, wrapping the data row
        // onto two lines. One filler row, then the table: at a height of 4
        // the arithmetic break would cut between them.
        let blocks = vec![prose("filler"), RenderLine::Gap, RenderLine::Table(table)];
        let pages = paginate(layout(&blocks, &[], 14.0, &mut tight()), 4.0);

        assert_eq!(
            pages_of(&pages),
            vec![vec!["filler", ""], vec!["id  meaning", "──  ──────────", "a   alpha beta", "    gamma"]]
        );
    }

    /// A glued run taller than the page itself has no good break, so the page
    /// is filled to the brim instead of thrashing or leaving an empty page.
    #[test]
    fn a_run_taller_than_the_page_fills_it() {
        // A heading followed by a long atomic paragraph, at a page too small
        // for both. Every row is glued, so pagination must fall back to full
        // pages.
        let blocks = vec![prose("Heading"), RenderLine::Gap, prose("alpha beta gamma")];
        let headings = [Heading { level: 1, text: "Heading".into(), block: 0 }];
        let pages = paginate(layout(&blocks, &headings, 5.0, &mut tight()), 2.0);

        assert_eq!(pages_of(&pages), vec![vec!["Heading", ""], vec!["alpha", "beta"], vec!["gamma"]]);
    }

    /// A row taller than the whole page still lands somewhere, on a page of
    /// its own, rather than stalling pagination.
    #[test]
    fn a_row_taller_than_the_page_gets_its_own_page() {
        let blocks = vec![prose("one"), RenderLine::Gap, prose("two")];
        let pages = paginate(layout(&blocks, &[], 10.0, &mut tight()), 0.5);
        assert_eq!(pages_of(&pages), vec![vec!["one"], vec!["two"]]);
    }

    /// `block_rows` must report the row each block starts on, so a heading's
    /// block index can be turned into a page. A collapsed gap contributes no
    /// rows but still gets an entry, keeping the mapping one-to-one with the
    /// input blocks.
    #[test]
    fn block_rows_track_where_each_block_begins() {
        let blocks = vec![prose("a b"), RenderLine::Gap, prose("c")];
        let laid = layout(&blocks, &[], 40.0, &mut tight());
        // "a b" on row 0, one blank row for the paragraph gap, "c" on row 2.
        assert_eq!(laid.block_rows, vec![0, 1, 2]);
        assert_eq!(laid.rows.len(), 3);
    }

    /// With paragraph spacing turned off, a gap adds no row at all rather
    /// than a zero-height one.
    #[test]
    fn a_zero_gap_adds_no_row() {
        let blocks = vec![prose("a"), RenderLine::Gap, prose("b")];
        let laid = layout(&blocks, &[], 40.0, &mut Grid { gap: 0.0 });
        assert_eq!(rows_of(&laid), vec!["a", "b"]);
    }

    /// A verbatim line wider than the column stays one row, never wrapped
    /// into a second, so ASCII art keeps its shape. Its full width is kept
    /// for the painter to clip and pan against.
    #[test]
    fn wide_verbatim_lines_stay_on_one_row() {
        let blocks = vec![RenderLine::Verbatim(Span::raw("0123456789ABCDEF"))];
        let laid = layout(&blocks, &[], 8.0, &mut tight());

        assert_eq!(rows_of(&laid), vec!["0123456789ABCDEF"], "a wide code line must stay one row");
        assert!(matches!(laid.rows[0].kind, RowKind::Verbatim { width, .. } if width == 16.0));
    }

    /// The widest verbatim line is what panning clamps against; prose and
    /// tables do not count, since they wrap to fit and never pan.
    #[test]
    fn code_width_measures_only_code() {
        let blocks = vec![
            prose("a paragraph that is quite long and wraps"),
            RenderLine::Verbatim(Span::raw("0123456789")),
            RenderLine::Verbatim(Span::raw("0123")),
        ];
        assert_eq!(layout(&blocks, &[], 8.0, &mut tight()).code_width, 10.0);
        assert_eq!(layout(&[prose("no code here")], &[], 8.0, &mut tight()).code_width, 0.0);
    }

    fn cell(text: &str) -> Vec<Span> {
        vec![Span::raw(text)]
    }

    /// With room to spare, every column takes its natural width, cells line
    /// up so columns start at the same place on every row, and the header
    /// gets a rule under it.
    #[test]
    fn table_columns_line_up_under_a_header_rule() {
        let table = TableBlock {
            header: vec![cell("Column"), cell("What it is")],
            rows: vec![vec![cell("id"), cell("Unique id")], vec![cell("title"), cell("Incident title")]],
            rule_style: Style::default(),
        };
        let laid = layout(&[RenderLine::Table(table)], &[], 40.0, &mut tight());
        assert_eq!(
            rows_of(&laid),
            vec!["Column  What it is", "──────  ──────────────", "id      Unique id", "title   Incident title"]
        );
    }

    /// A table wider than the page wraps cell text inside its column
    /// instead of spilling past the edge: the narrow column keeps its
    /// natural width, the wide one absorbs the squeeze and grows downward.
    #[test]
    fn wide_tables_wrap_cells_inside_their_columns() {
        let table = TableBlock {
            header: vec![cell("id"), cell("meaning")],
            rows: vec![vec![cell("a"), cell("alpha beta gamma")]],
            rule_style: Style::default(),
        };
        let laid = layout(&[RenderLine::Table(table)], &[], 14.0, &mut tight());
        assert_eq!(rows_of(&laid), vec!["id  meaning", "──  ──────────", "a   alpha beta", "    gamma"]);
    }
}
