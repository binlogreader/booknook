//! Reading an EPUB as a booknook document.
//!
//! An EPUB is a zip of XHTML chapters plus a manifest that fixes their
//! order. That makes it reflowable text, the same thing markdown is after
//! parsing, so this module ends at exactly the same place `markdown` does: a
//! `Parsed`, blocks and headings, that the `wrap` and `ui` modules consume
//! without ever learning it came from a book. The `epub` crate handles the
//! container, the manifest, and the reading order; this module walks each
//! chapter's XHTML and turns it into `RenderLine`s.
//!
//! The contents pane has two possible sources, and real books split evenly
//! on which one is trustworthy. Headings harvested from the content, `<h1>`
//! through `<h6>`, are the primary source, the same way the markdown parser
//! harvests `#` headings; converted books often carry clean chapter headings
//! in the text while their navigation file is auto-generated garbage. But
//! the opposite conversion exists too: a book whose chapters are styled
//! paragraphs with no heading tags at all, and whose navigation file is the
//! only clean chapter list there is. So when harvesting comes up nearly
//! empty against a whole book, the book's own table of contents is resolved
//! to chapter positions and used instead.
//!
//! Inline styling in books arrives two ways: as real tags, `<em>` and
//! `<strong>`, and as classed spans, `<span class="italic">`, which is what
//! Calibre conversions emit. Both are honored, because a book where every
//! italicized title renders as plain text has quietly lost something the
//! author put there.

use std::path::Path;

use anyhow::{Context, Result};
// The leading `::` reaches the `epub` crate rather than this module, which
// shares its name.
use ::epub::doc::EpubDoc;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use unicode_width::UnicodeWidthStr;

use crate::markdown::{Heading, Parsed, RenderLine};
use crate::text::{Ink, Span, Style};

pub(crate) fn is_epub(path: &Path) -> bool {
    matches!(path.extension().and_then(|ext| ext.to_str()), Some(ext) if ext.eq_ignore_ascii_case("epub"))
}

/// A loaded book: its title from the EPUB metadata, when there is one, and
/// the same `Parsed` a markdown file produces. The title matters more here
/// than for a markdown file, because an EPUB's filename is often a catalog
/// entry, long and unreadable, while its metadata carries the actual name of
/// the book.
pub(crate) struct Book {
    pub(crate) title: Option<String>,
    pub(crate) parsed: Parsed,
}

/// Open an EPUB and convert the whole book, in spine order, into blocks.
///
/// The spine is the book's reading order, and every chapter in it is
/// rendered into one flat list of blocks with a gap between chapters, so
/// paging through the book crosses chapter boundaries the way it would in a
/// printed volume. Chapters that fail to decode are skipped rather than
/// failing the whole book; losing one damaged chapter is better than
/// refusing to open the other forty.
pub(crate) fn load(path: &Path) -> Result<Book> {
    let mut doc = EpubDoc::new(path)
        .with_context(|| format!("could not open {} as an EPUB", path.display()))?;
    let title = doc.get_title();

    let mut blocks: Vec<RenderLine> = Vec::new();
    let mut headings: Vec<Heading> = Vec::new();

    // The spine is cloned so iterating it does not hold a borrow of `doc`,
    // which `get_resource_str` needs mutably. Linear items are the book's
    // main flow; non-linear ones are asides a reader reaches by link, which
    // booknook has no way to follow, so they are skipped rather than spliced
    // into the middle of the text.
    let spine: Vec<String> =
        doc.spine.iter().filter(|item| item.linear).map(|item| item.idref.clone()).collect();
    // Where each chapter's blocks begin, keyed by the chapter's path inside
    // the container. This is what lets a navigation entry, which points at a
    // file, be resolved to a position in the flat block list.
    let mut chapter_starts: Vec<(std::path::PathBuf, usize)> = Vec::new();
    for idref in spine {
        let resource_path = doc.resources.get(&idref).map(|res| res.path.clone());
        let Some((chapter, _mime)) = doc.get_resource_str(&idref) else {
            continue;
        };
        if !blocks.is_empty() {
            blocks.push(RenderLine::Gap);
        }
        if let Some(path) = resource_path {
            chapter_starts.push((path, blocks.len()));
        }
        render_xhtml(&chapter, &mut blocks, &mut headings);
    }

    // A whole book that yielded almost no headings did not really yield a
    // table of contents; it yielded noise. In that case the book's own
    // navigation file, resolved against where each chapter actually starts,
    // is the better list, provided it actually resolves to more entries than
    // harvesting found.
    if headings.len() < 4 {
        let from_nav = toc_headings(&doc.toc, &chapter_starts);
        if from_nav.len() > headings.len() {
            headings = from_nav;
        }
    }

    Ok(Book { title, parsed: Parsed { blocks, headings } })
}

/// Turn the book's navigation tree into contents entries, one per nav point
/// that resolves to a known chapter. Depth in the tree becomes the entry's
/// level, and a fragment anchor is ignored: the entry lands on its chapter's
/// start, which is as precise as a file-level mapping can be.
fn toc_headings(nav: &[::epub::doc::NavPoint], chapter_starts: &[(std::path::PathBuf, usize)]) -> Vec<Heading> {
    let mut headings = Vec::new();
    flatten_nav(nav, 1, chapter_starts, &mut headings);
    // Several nav points can resolve to the same chapter start, for example
    // a fragment-anchored subsection alongside its chapter. Keep the first.
    headings.dedup_by(|a, b| a.block == b.block);
    headings
}

fn flatten_nav(
    nav: &[::epub::doc::NavPoint],
    level: u8,
    chapter_starts: &[(std::path::PathBuf, usize)],
    out: &mut Vec<Heading>,
) {
    for point in nav {
        // The nav's target file, with any `#fragment` stripped: the fragment
        // rides along inside the final path component. Separators are
        // normalized before comparing, because on Windows the nav paths come
        // out of a `PathBuf::join` with backslashes while the manifest paths
        // keep the forward slashes they were written with; the same file,
        // spelled two ways.
        let target = normalize_separators(&point.content.to_string_lossy());
        let target = target.split('#').next().unwrap_or(&target).to_string();
        let text = normalize_whitespace(&point.label);
        if !text.is_empty()
            && let Some((_, block)) = chapter_starts
                .iter()
                .find(|(path, _)| normalize_separators(&path.to_string_lossy()) == target)
        {
            out.push(Heading { level, text, block: *block });
        }
        flatten_nav(&point.children, level.saturating_add(1).min(6), chapter_starts, out);
    }
}

/// The one true separator, for comparing container paths that different
/// parsing routes spelled differently.
fn normalize_separators(path: &str) -> String {
    path.replace('\\', "/")
}

/// What kind of list the converter is currently inside, mirroring the
/// markdown parser's own tracking. Ordered lists carry the next item number.
enum ListKind {
    Bullet,
    Ordered(u64),
}

/// Convert one chapter's XHTML into blocks, appending to `blocks` and
/// `headings` so a whole book accumulates into a single document.
///
/// This is a hand-rolled walk over quick-xml's event stream, shaped exactly
/// like `markdown::render_markdown`'s walk over pulldown-cmark's: a style
/// stack for inline formatting, structural counters for quotes and lists,
/// and a flush whenever a block ends. XHTML inside an EPUB is required to be
/// well-formed XML, which is what makes an XML parser sufficient where
/// general HTML would need a real HTML parser.
fn render_xhtml(input: &str, blocks: &mut Vec<RenderLine>, headings: &mut Vec<Heading>) {
    let mut reader = Reader::from_str(input);
    // Books converted from other formats are not always perfectly nested.
    // Checking end-tag names would make one stray tag abort the chapter;
    // without it, the walk just keeps going.
    reader.config_mut().check_end_names = false;

    let mut spans: Vec<Span> = Vec::new();
    let mut style_stack: Vec<Style> = Vec::new();
    let mut style = Style::default();
    let mut list_stack: Vec<ListKind> = Vec::new();
    let mut quote_depth = 0u32;
    let mut indent = 0u16;
    let mut hang = 0u16;
    // Inside <head>, <style>, or <script>, text is markup plumbing, not
    // prose. A depth counter rather than a flag, because they can nest.
    let mut skip_depth = 0u32;
    // The level of the heading currently open, if any, matching the way the
    // markdown parser tracks the same thing.
    let mut heading_level: Option<u8> = None;
    let mut in_pre = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = local_name(&e);
                match name.as_str() {
                    "head" | "style" | "script" | "title" => skip_depth += 1,
                    "p" | "div" if quote_depth > 0 && name == "p" => {
                        flush(blocks, &mut spans);
                        spans.push(Span::styled("┃ ", Style::default().rule()));
                        hang = 2;
                    }
                    "p" => flush(blocks, &mut spans),
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        flush(blocks, &mut spans);
                        blocks.push(RenderLine::Gap);
                        heading_level = name[1..].parse::<u8>().ok();
                        style_stack.push(style);
                        style = style.heading(heading_level.unwrap_or(1));
                    }
                    "em" | "i" | "cite" | "dfn" => {
                        style_stack.push(style);
                        style = style.italic();
                    }
                    "strong" | "b" => {
                        style_stack.push(style);
                        style = style.bold();
                    }
                    "a" => {
                        style_stack.push(style);
                        style = style.ink(Ink::Link).underlined();
                    }
                    "code" | "tt" | "kbd" | "samp" => {
                        style_stack.push(style);
                        // Tinted paper rather than colored words, matching
                        // markdown's inline code.
                        style = style.code();
                    }
                    // Calibre and its kin encode italics and bold as classed
                    // spans instead of semantic tags. The class attribute is
                    // the only signal there is.
                    "span" => {
                        style_stack.push(style);
                        let class = attr(&e, "class").unwrap_or_default();
                        if class.contains("italic") || class.contains("oblique") {
                            style = style.italic();
                        }
                        if class.contains("bold") {
                            style = style.bold();
                        }
                    }
                    "blockquote" => {
                        flush(blocks, &mut spans);
                        quote_depth += 1;
                        style_stack.push(style);
                        style = style.ink(Ink::Quote).italic();
                    }
                    "ul" => list_stack.push(ListKind::Bullet),
                    "ol" => list_stack.push(ListKind::Ordered(1)),
                    "li" => {
                        flush(blocks, &mut spans);
                        let depth = list_stack.len().max(1);
                        let nesting = 2 * depth.saturating_sub(1) as u16;
                        let marker = match list_stack.last_mut() {
                            Some(ListKind::Ordered(n)) => {
                                let marker = format!("{n}. ");
                                *n += 1;
                                marker
                            }
                            _ => "• ".to_string(),
                        };
                        indent = nesting;
                        hang = nesting + UnicodeWidthStr::width(marker.as_str()) as u16;
                        spans.push(Span::styled(marker, Style::default().ink(Ink::Muted)));
                    }
                    "pre" => {
                        flush(blocks, &mut spans);
                        in_pre = true;
                    }
                    _ => {}
                }
            }
            Ok(Event::End(e)) => {
                let name = local_name_end(e.name().as_ref());
                match name.as_str() {
                    "head" | "style" | "script" | "title" => skip_depth = skip_depth.saturating_sub(1),
                    "p" | "li" => {
                        flush_block(blocks, &mut spans, indent, hang, in_pre);
                        indent = 0;
                        hang = 0;
                        // Like markdown, list items sit tight; only real
                        // paragraphs earn a gap.
                        if name == "p" {
                            blocks.push(RenderLine::Gap);
                        }
                    }
                    "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                        if let Some(level) = heading_level.take() {
                            let text: String = spans.iter().map(|s| s.content.as_str()).collect();
                            let text = normalize_whitespace(&text);
                            if !text.is_empty() {
                                headings.push(Heading { level, text, block: blocks.len() });
                            }
                        }
                        flush(blocks, &mut spans);
                        blocks.push(RenderLine::Gap);
                        style = style_stack.pop().unwrap_or_default();
                    }
                    "em" | "i" | "cite" | "dfn" | "strong" | "b" | "a" | "code" | "tt" | "kbd" | "samp"
                    | "span" => {
                        style = style_stack.pop().unwrap_or_default();
                    }
                    "blockquote" => {
                        flush(blocks, &mut spans);
                        quote_depth = quote_depth.saturating_sub(1);
                        style = style_stack.pop().unwrap_or_default();
                        if quote_depth == 0 {
                            blocks.push(RenderLine::Gap);
                        }
                        indent = 0;
                        hang = 0;
                    }
                    "ul" | "ol" => {
                        flush(blocks, &mut spans);
                        list_stack.pop();
                        if list_stack.is_empty() {
                            blocks.push(RenderLine::Gap);
                        }
                    }
                    "pre" => {
                        flush_block(blocks, &mut spans, indent, hang, in_pre);
                        in_pre = false;
                        blocks.push(RenderLine::Gap);
                    }
                    _ => {}
                }
            }
            Ok(Event::Empty(e)) => {
                let name = local_name(&e);
                match name.as_str() {
                    // A line break inside a paragraph ends the current prose
                    // row without earning a paragraph gap.
                    "br" => flush_block(blocks, &mut spans, indent, hang, in_pre),
                    "hr" => {
                        flush(blocks, &mut spans);
                        blocks.push(RenderLine::Gap);
                        blocks.push(RenderLine::Prose {
                            spans: vec![Span::styled("· · ·", Style::default().ink(Ink::Muted))],
                            indent: 0,
                            hang: 0,
                        });
                        blocks.push(RenderLine::Gap);
                    }
                    // booknook does not draw pictures yet. The alt text, when
                    // the book bothered to write one, is the readable residue
                    // of the image; a bare marker otherwise, so a figure does
                    // not vanish without a trace.
                    "img" | "image" => {
                        flush(blocks, &mut spans);
                        let label = match attr(&e, "alt").filter(|alt| !alt.trim().is_empty()) {
                            Some(alt) => format!("[image: {}]", alt.trim()),
                            None => "[image]".to_string(),
                        };
                        blocks.push(RenderLine::Prose {
                            spans: vec![Span::styled(label, Style::default().ink(Ink::Muted).italic())],
                            indent: 0,
                            hang: 0,
                        });
                        blocks.push(RenderLine::Gap);
                    }
                    _ => {}
                }
            }
            Ok(Event::Text(e)) => {
                if skip_depth == 0 {
                    let text = unescape_entities(&String::from_utf8_lossy(e.as_ref()));
                    if in_pre {
                        // Preformatted text keeps its own line structure; each
                        // line becomes a verbatim row exactly as markdown's
                        // fenced code does.
                        for line in text.split('\n') {
                            if !line.trim().is_empty() {
                                blocks.push(RenderLine::Verbatim(Span::styled(
                                    format!("  {line}"),
                                    Style::default().code(),
                                )));
                            }
                        }
                    } else if !text.is_empty() {
                        spans.push(Span::styled(text, style));
                    }
                }
            }
            Ok(Event::CData(e)) => {
                if skip_depth == 0 {
                    let text = String::from_utf8_lossy(e.as_ref()).into_owned();
                    if !text.trim().is_empty() {
                        spans.push(Span::styled(text, style));
                    }
                }
            }
            Ok(Event::Eof) => break,
            // A malformed stretch of markup should cost the stretch, not the
            // chapter. Skip the event and keep walking.
            Err(_) => continue,
            _ => {}
        }
    }
    flush(blocks, &mut spans);
}

/// Flush collected spans as a plain prose block with no indent.
fn flush(blocks: &mut Vec<RenderLine>, spans: &mut Vec<Span>) {
    flush_block(blocks, spans, 0, 0, false);
}

/// Move collected spans into a finished block, if they hold any actual text.
/// XHTML is full of whitespace-only text nodes between tags; a block made of
/// nothing but those would render as a stray blank line, so they are dropped
/// here rather than filtered by every caller.
fn flush_block(blocks: &mut Vec<RenderLine>, spans: &mut Vec<Span>, indent: u16, hang: u16, in_pre: bool) {
    if in_pre {
        // Preformatted text was already pushed as verbatim rows on arrival.
        spans.clear();
        return;
    }
    let has_text = spans.iter().any(|s| !s.content.trim().is_empty());
    if has_text {
        blocks.push(RenderLine::Prose { spans: std::mem::take(spans), indent, hang });
    } else {
        spans.clear();
    }
}

/// The tag's name without any namespace prefix, lowercased. EPUB XHTML can
/// arrive as `<html:p>` under a prefixed namespace, and tag case is not
/// worth trusting in converted books.
fn local_name(e: &BytesStart) -> String {
    local_name_end(e.name().as_ref())
}

fn local_name_end(raw: &[u8]) -> String {
    let name = String::from_utf8_lossy(raw);
    let local = name.rsplit(':').next().unwrap_or(&name);
    local.to_ascii_lowercase()
}

/// One attribute's value, unescaped enough for the uses this module has:
/// class names and alt text.
fn attr(e: &BytesStart, name: &str) -> Option<String> {
    e.attributes().filter_map(|a| a.ok()).find_map(|a| {
        if local_name_end(a.key.as_ref()) == name {
            Some(unescape_entities(&String::from_utf8_lossy(&a.value)))
        } else {
            None
        }
    })
}

/// Collapse runs of whitespace to single spaces and trim. Heading text
/// gathered across XHTML text nodes carries the source file's newlines and
/// indentation, none of which belongs in a contents entry.
fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Resolve character references by hand: the numeric forms, the XML five,
/// and the named entities that publishing actually uses. quick-xml leaves
/// entity references in text as-is, and hand-rolling the resolution means an
/// unknown entity degrades to staying visible as typed, `&weird;`, instead
/// of aborting the chapter the way strict XML unescaping would.
fn unescape_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        rest = &rest[pos..];
        // An entity is `&`, up to a `;` within a short distance. Anything
        // else is a bare ampersand that belongs in the text.
        match rest[1..].find(';').filter(|end| *end <= 10) {
            Some(end) => {
                let name = &rest[1..1 + end];
                match resolve_entity(name) {
                    Some(replacement) => out.push_str(replacement),
                    None => {
                        // Numeric reference, decimal or hex.
                        let ch = name
                            .strip_prefix("#x")
                            .or_else(|| name.strip_prefix("#X"))
                            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                            .or_else(|| name.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                            .and_then(char::from_u32);
                        match ch {
                            Some(ch) => out.push(ch),
                            // Unknown: keep it visible rather than lose it.
                            None => out.push_str(&rest[..end + 2]),
                        }
                    }
                }
                rest = &rest[end + 2..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The named entities worth knowing: the XML five plus the punctuation and
/// symbols that show up in real books. Typographic quotes and dashes are the
/// common ones; the rest cost nothing to carry.
fn resolve_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => " ",
        "shy" => "",
        "mdash" => "—",
        "ndash" => "–",
        "hellip" => "…",
        "lsquo" => "\u{2018}",
        "rsquo" => "\u{2019}",
        "ldquo" => "\u{201C}",
        "rdquo" => "\u{201D}",
        "laquo" => "«",
        "raquo" => "»",
        "bull" => "•",
        "middot" => "·",
        "dagger" => "†",
        "Dagger" => "‡",
        "sect" => "§",
        "para" => "¶",
        "copy" => "©",
        "reg" => "®",
        "trade" => "™",
        "deg" => "°",
        "plusmn" => "±",
        "times" => "×",
        "divide" => "÷",
        "frac12" => "½",
        "frac14" => "¼",
        "frac34" => "¾",
        "pound" => "£",
        "euro" => "€",
        "cent" => "¢",
        "yen" => "¥",
        "prime" => "′",
        "Prime" => "″",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(xhtml: &str) -> Parsed {
        let mut blocks = Vec::new();
        let mut headings = Vec::new();
        render_xhtml(xhtml, &mut blocks, &mut headings);
        Parsed { blocks, headings }
    }

    fn prose_texts(parsed: &Parsed) -> Vec<String> {
        parsed
            .blocks
            .iter()
            .filter_map(|b| match b {
                RenderLine::Prose { spans, .. } => {
                    Some(normalize_whitespace(&spans.iter().map(|s| s.content.as_str()).collect::<String>()))
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn paragraphs_become_prose_blocks_and_head_matter_is_skipped() {
        let parsed = convert(
            "<html><head><title>Junk</title><style>p{}</style></head>\
             <body><p>First paragraph.</p><p>Second.</p></body></html>",
        );
        assert_eq!(prose_texts(&parsed), vec!["First paragraph.", "Second."]);
    }

    #[test]
    fn headings_are_recorded_with_level_and_block() {
        let parsed = convert("<body><h2>STEWART</h2><p>Text.</p></body>");
        assert_eq!(parsed.headings.len(), 1);
        assert_eq!(parsed.headings[0].level, 2);
        assert_eq!(parsed.headings[0].text, "STEWART");
        // The recorded block resolves back to the heading's own text.
        let all = prose_texts(&parsed);
        assert!(all.contains(&"STEWART".to_string()));
    }

    /// Calibre encodes italics and bold as classed spans. The class is the
    /// only signal, and both can nest.
    #[test]
    fn classed_spans_carry_italic_and_bold() {
        let parsed = convert(
            r#"<body><p><span class="italic"><span class="bold">Seeing Anew</span></span></p></body>"#,
        );
        let RenderLine::Prose { spans, .. } = &parsed.blocks[0] else {
            panic!("expected prose");
        };
        let styled = spans.iter().find(|s| s.content.as_str() == "Seeing Anew").expect("span text");
        assert!(styled.style.italic);
        assert!(styled.style.bold);
    }

    #[test]
    fn semantic_tags_carry_style_too() {
        let parsed = convert("<body><p>a <em>slanted</em> and <strong>heavy</strong> word</p></body>");
        let RenderLine::Prose { spans, .. } = &parsed.blocks[0] else {
            panic!("expected prose");
        };
        let em = spans.iter().find(|s| s.content.as_str() == "slanted").unwrap();
        let strong = spans.iter().find(|s| s.content.as_str() == "heavy").unwrap();
        assert!(em.style.italic);
        assert!(strong.style.bold);
    }

    #[test]
    fn entities_resolve_including_numeric_forms() {
        assert_eq!(unescape_entities("Tom &amp; Jerry&mdash;friends&#33; &#x2764;"), "Tom & Jerry—friends! ❤");
        // Unknown entities stay visible instead of vanishing.
        assert_eq!(unescape_entities("keep &weird; text"), "keep &weird; text");
        // A bare ampersand is just an ampersand.
        assert_eq!(unescape_entities("AT&T works"), "AT&T works");
    }

    #[test]
    fn whitespace_only_nodes_do_not_become_blocks() {
        let parsed = convert("<body>\n  <p>Real text.</p>\n  \n</body>");
        assert_eq!(prose_texts(&parsed), vec!["Real text."]);
    }

    #[test]
    fn images_leave_a_readable_trace() {
        let parsed = convert(r#"<body><p>before</p><img src="x.jpg" alt="A map of Europe"/></body>"#);
        assert!(prose_texts(&parsed).contains(&"[image: A map of Europe]".to_string()));
    }

    #[test]
    fn lists_get_markers() {
        let parsed = convert("<body><ul><li>one</li><li>two</li></ul></body>");
        let texts = prose_texts(&parsed);
        assert_eq!(texts, vec!["• one", "• two"]);
    }

    /// The real proof: open an actual book from disk. Ignored by default so
    /// CI and clean checkouts do not need a novel on hand; run it with
    /// BOOKNOOK_EPUB pointing at any .epub:
    /// `BOOKNOOK_EPUB=path/to/book.epub cargo test -- --ignored --nocapture`
    #[test]
    #[ignore = "needs a real epub; set BOOKNOOK_EPUB"]
    fn loads_a_real_book() {
        let Some(path) = std::env::var_os("BOOKNOOK_EPUB") else {
            return;
        };
        let book = load(Path::new(&path)).expect("book should open");
        println!("title: {:?}", book.title);
        println!("blocks: {}", book.parsed.blocks.len());
        println!("headings: {}", book.parsed.headings.len());
        for h in book.parsed.headings.iter().take(20) {
            println!("  h{} @block {}: {}", h.level, h.block, h.text);
        }
        assert!(book.parsed.blocks.len() > 100, "a book should produce many blocks");
        assert!(!book.parsed.headings.is_empty(), "chapter headings should be harvested");
    }
}
