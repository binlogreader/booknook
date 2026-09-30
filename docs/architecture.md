# Architecture

This document explains how booknook is put together. It assumes you have
read the README and understand what the app is trying to be. Here we focus
on how the code delivers on that, module by module.

## The pipeline, from markdown text to a painted page

Opening a file sets three things in motion, and it helps to understand them
as a pipeline before looking at any single module.

First, the file's text is parsed. The `markdown` module reads the raw
markdown string with pulldown-cmark and walks its stream of events, turning
headings, paragraphs, lists, quotes, links, and code blocks into a flat list
of `RenderLine` values. Nothing in this step knows how wide the page is,
which typeface is installed, or which theme is in use. A `RenderLine` is one
of four things: `Prose`, a run of styled spans that can still be reflowed
later; `Verbatim`, a finished line that must never be rewrapped, such as a
line of code; `Table`, a grid of cells whose column widths depend on the
page; or `Gap`, a break between two blocks. A `Gap` records the intention to
separate, not a fixed amount of space, because how much air a break gets is
a layout decision rather than a parsing one.

Each span carries a `Style` from the `text` module, and a `Style` says what
the text is rather than how it looks: body or heading, bold or italic, prose
or code. The color, the face, and the size are chosen later, when the page is
painted. That is why switching themes or changing the text size never
reparses anything.

Second, that list is laid out. The `wrap` module takes the blocks produced
by parsing and fits them to a column a given number of points wide, one word
at a time. It measures each word through a `Measure`, which the `typeset`
module implements with egui's real glyph metrics, so a line breaks where the
actual text in the actual face runs out of room. This is also where spacing
happens, and it applies two different numbers: the leading inside a
paragraph, and a larger gap between paragraphs. Those have to differ. If the
gap within a paragraph matched the gap between paragraphs, every line would
read as its own paragraph and the page would lose all of its structure. The
result is a list of rows, each with its height, where it starts on the line,
and a hint about whether a page may end after it. `wrap::paginate` then
groups those rows into pages.

A word, in this module, is not the same thing as a span. pulldown-cmark
emits smart punctuation as its own event, so `country's` arrives as three
separate spans: `country`, then the apostrophe, then `s`. Wrapping each
span independently and rejoining the results with spaces would render that
as `country ’ s`. The wrapper therefore defines a word as whatever sits
between two runs of whitespace, however many spans and styles it crosses.

Third, the page in view is painted. The `ui` module takes that page's rows
and draws each one with egui's painter. A line of prose becomes one egui text
layout, placed where `wrap` said it starts and centered in its row so the
leading falls evenly above and below. A line of code gets a band of tinted
paper under it and is clipped at the column's edges. A table row is drawn
cell by cell, each cell clipped to its column.

The reason parsing and layout are two separate steps, rather than one, is
that the column width is not known until the window is actually being drawn.
The window can be resized, the sidebar comes and goes, the text size can
change, and a spread fits a different number of lines than a short window
does. If wrapping happened once, at parse time, none of that could be handled
without reparsing the whole document. Instead, layout runs again whenever the
width, the page height, the spacing, or the document changes, and the result
is cached in between, so a resized window reflows correctly without any
special handling anywhere else in the app.

## Module map

**`theme`** holds the color palettes and nothing else. A `Theme` is plain
data, with no logic, depending on nothing but egui's `Color32` type. The
available palettes live in a `THEMES` static, which is a `static` rather
than a `const` so that `&THEMES[i]` borrows for `'static`. That detail
saves every other module from threading a lifetime parameter through
itself just to hold a reference to the current palette.

**`text`** defines `Style`, `Ink`, and `Span`, the vocabulary the parsers
describe text in, and depends on nothing. An `Ink` names a role, such as
heading, link, or muted, which `typeset` resolves against the current theme;
the rest of a `Style` records weight, slant, underline, whether the text is
code, and its heading level. The builder methods read the way ratatui's did
when booknook ran in a terminal, `style.bold()` and `style.ink(Ink::Link)`,
which kept the parsers' style stacks as short as they were.

**`browser`** knows how to list a directory's contents and how to tell a
markdown file from any other kind of file. It has no idea that an `App` or
a window exists. Given a path, it returns a sorted list of entries. This
separation means the filesystem logic could be tested, or reused in a
completely different interface, without touching anything else.

**`markdown`** is the parsing stage described above. It depends on `text`,
for styles, and on pulldown-cmark, for the actual markdown grammar. Its only
public output is `render_markdown`, which takes a string and returns a
`Parsed`: the `Vec<RenderLine>` to lay out, plus a flat list of the
document's headings for the table of contents. Both come from one pass, so
the contents list can never drift out of step with the blocks it points
into. Each heading records the index of the block that holds its text,
which is the handle the sidebar later uses to jump to it.

**`epub`** is the other way a document gets parsed. It opens an EPUB
container with the `epub` crate, walks every chapter in the book's reading
order, and converts each chapter's XHTML into the same `RenderLine` blocks
the markdown parser produces, using quick-xml for the markup. It ends at
exactly the same type `markdown` does, a `Parsed`, which is the point:
nothing downstream of parsing knows whether it is showing a note or a novel.
Chapter headings found in the content feed the table of contents, and when a
book carries no usable headings, the book's own navigation file is resolved
against each chapter's starting block and used instead. Inline styling is
honored both as semantic tags, `<em>` and `<strong>`, and as the classed
spans Calibre conversions emit, `<span class="italic">`, because a book that
loses its italics has quietly lost part of its text.

**`wrap`** is the layout stage described above. It depends on `markdown`,
for the `RenderLine` type it consumes, and on `text`, and on nothing that
knows about fonts: every width and height comes through the `Measure` trait.
Its public outputs are `layout`, which takes a slice of blocks, the
headings, a width, and a `Measure`, and returns a `Laid`, and `paginate`,
which takes a `Laid` and a page height and returns `Pages`. `Pages` knows
which rows each page holds and on which page every input block begins, which
is what turns a heading's block index into a page. The tests implement
`Measure` as a fixed grid, one unit per character and per row, so pagination
can be checked by counting, with no window involved.

**`typeset`** decides how a `Style` looks on the page. At startup it looks
for Helvetica on disk, or the nearest face drawn to its measure, Arial on
most machines, and registers it with egui along with its bold and italic
cuts, behind egui's built-in fonts so any glyph it lacks still renders. Each frame it builds a `Typesetter` from the
spacing settings, which maps a style to an egui font and text format and
implements `wrap::Measure` against egui's glyph widths. It depends on
`text`, `theme`, and `wrap`.

**`session`** knows how to read and write the small file that remembers, between
runs, the last document opened, the page reached in every document seen so
far, and the typographic settings in force. Like `theme` and `browser`, it
depends on nothing above it and has no idea an `App` exists. Its format is a
plain, line-based text file rather than JSON, so it needs no serialization
dependency and stays readable on its own; an unrecognized line is skipped
rather than treated as an error, so a field added by a future version does no
harm to an older one.

**`app`** defines the `App` struct, which is the single source of truth for
everything the program currently knows: which directory the sidebar is
showing, which file is open, its headings, which page is on screen, and
which pane has keyboard focus. It also defines the methods that are allowed
to change several of those fields together, such as `load_file` and
`enter_dir`, so that related state always changes as a unit. `load_file`, for
instance, records the previous file's page before switching, then opens the
new file at whatever page was remembered for it, so moving between documents
resumes each one rather than restarting it. `app` depends on `browser`,
`markdown`, `epub`, and `session`, since those are the modules that produce
the data an `App` holds, and on `wrap` for the `Spacing` type.

**`events`** turns keyboard input into calls against an `App`. Each frame it
reads the input events egui collected, reduces each one to a `KeyCode`, a
typed character or one of the named keys it binds, and decides what should
happen: move the sidebar selection, turn a page, switch focus, or quit. It
depends on `app`, to mutate state, and on `browser`, to check whether a
selected file is worth opening. Nothing in this module draws.

**`anim`** is the optional page turn. A `Turn` records which page it left
and when it started, knows how far the fold has crossed the sheet at any
moment, and paints the lit edge and the shadow at the fold. It depends on
nothing in booknook but egui.

**`ui`** is the only module that draws. Its `Reader` is the eframe app:
each frame it hands the input to `events`, settles the page, and paints the
status bar, the sidebar, and the sheet. It depends on `app`, to read state,
on `events`, on `browser`, for the same readable-file check the sidebar uses
to color file names, on `theme` and `typeset`, for how things look, on
`wrap`, to lay out the document, and on `anim`, for the page turn.

**`main`** is the thin entry point. It loads the saved session, parses the
one optional command line argument, builds the initial `App`, and hands it
to `ui::run`, which opens the window. A gist or a pull request is fetched
here, before the window opens, so a network failure is reported in the
terminal the command was typed in. It contains no markdown logic, no drawing
logic, and no key handling logic of its own.

A dependency runs in one direction only. `theme`, `text`, `browser`, and
`session` depend on nothing inside booknook. `markdown` and `epub` depend
only on `text`, plus, for `epub`, the `markdown` module's block types, since
both parsers meet at the same output. `wrap` depends on `markdown` and
`text`, and `typeset` on `wrap`, `text`, and `theme`. `app` depends on
`browser`, `markdown`, `epub`, `session`, and `wrap`. `events` and `ui` both
depend on `app`, plus whatever lower-level module they need directly. `main`
depends on everything. Nothing lower in this list ever depends on something
higher, which is what makes each module possible to read in isolation.

## The App struct as the single source of truth

booknook draws in immediate mode, which is how egui works. There is no
long-lived widget tree and no incremental updates. Every frame,
`Reader::frame` reads the current `App` from scratch and paints the entire
window again, sidebar, sheet, and status bar included. This is simpler to
reason about than a retained UI, at the cost of doing more work per frame.
The cost stays small for two reasons. egui only asks for a frame when
something happens, a key or a resize, so between keypresses the program
sleeps. And the one expensive step, laying out the whole document, is cached
by `Reader` and redone only when something that shapes it has changed.

Because painting is stateless apart from that cache, all the state that
matters lives in one place. If a value affects what gets drawn, it belongs on
`App`. A consequence of this is that a few fields exist purely to let two
otherwise separate steps talk to each other across a frame boundary. The
clearest example is `spread`: `Reader::prepare` is the only code that knows
whether the window is currently wide enough for a two-page layout, but
`events::handle_document_key` needs that same fact to decide whether a page
turn should move by one page or by two. Rather than recomputing the width
check in the event handler, `prepare` writes the answer into `App` every
frame, and the event handler reads it back on the next keypress. This is a
deliberate exception to the general rule that state flows one direction,
from input to app to render, and it is called out in the field's own comment
so it does not look accidental later.

Two more fields work the same way, both in service of the table of contents.
`pending_jump` carries a request in the other direction, from input toward
render: choosing a heading sets it to that heading's block index, and the
next frame resolves the block to a page and clears it, because only the draw
step knows the page size that decides which page a block lands on.
`active_heading` carries an answer back the same way `spread` does: each
frame works out which heading the visible page falls under and records it,
so the sidebar can highlight that entry. Both are called out in their own
comments for the same reason `spread` is.

egui adds one ordering constraint of its own. Side panels have to be added
before the central panel, which takes whatever space is left, but the status
bar and the sidebar want answers only layout can give: how many pages there
are, and which heading is in view. So `Reader::frame` predicts the reading
area from the same widths it gives the panels, lays the document out against
that prediction in `prepare`, and only then draws the panels and the sheet.
A test checks that the prediction matches the rect the central panel
actually receives.

## Pages, not scroll rows

The reading pane stores `page: u16`, a page number, rather than a scroll
offset. This choice is what makes booknook behave like an e-ink reader
instead of a text pager. A scroll offset is a derived quantity: it depends on
how tall the current page is, which changes if the window is resized or the
text size changes. A page number is the real thing being tracked. Turning a
page increments or decrements that number, and only at draw time does `ui`
look up which rows that page holds. If the window is resized between two
frames, the app does not end up scrolled to a strange half-page position. It
reflows and lands back on the same page number, wherever that page's content
now starts.

Where each page ends is decided by `wrap::paginate`. After layout produces
the document's rows, each tagged with whether a page may end after it,
pagination fills a page with rows until the next one would not fit, then
moves any break that would land badly: a heading at a page bottom, a
paragraph's first line stranded there, its last line alone at the top of the
next page, a table row cut through a wrapped cell. The page ends a line or
two early instead, the way a typesetter leaves a short page rather than a
bad break. Each page is kept as a range of rows, so a short page simply holds
fewer of them, and the next page starts where the last one ended, less any
blank air, which a new page sheds from its top.

Because the true number of pages is not known until layout has happened,
`events::handle_document_key` sometimes asks for a page that does not
exist yet. Jumping to the last page, bound to `G`, sets `page` to `u16::MAX`
rather than trying to compute the real last page number itself. The one
place that does know the real bound, `Reader::prepare`, clamps whatever
value it is given down to something that actually exists. This keeps the
knowledge of page bounds in a single place, rather than duplicating that
computation in the key handler. Sideways panning of wide code follows the
same pattern: `.` asks for more, and `prepare` trims the request to the
widest code line's actual overflow.

## The two-page spread

When the window is wide enough, booknook shows two pages side by side on
one sheet, with a rule down the middle standing in for a book's spine and the
paper shaded slightly toward it on both sides. Both halves come from one
layout, at the same column width, so that continuing from the left page to
the right page reads exactly like turning past the middle of an open book
rather than reflowing into a different shape.

The left page always shows an even-numbered page, the same way a real
book's left-hand pages are always even. `Reader::prepare` enforces this by
rounding `app.page` down by one whenever it is odd, every frame, and
`events::handle_document_key` steps by two pages at a time whenever
`app.spread` is true. Neither function needs to know about the other's
half of this rule. Each one keeps its own part correct, and the two stay in
sync as a result.

If the window is not wide enough for two comfortable columns, the same frame
that would have drawn a spread draws a single page instead. `Geometry::new`
makes that decision from the current width, every frame, so resizing the
window between narrow and wide switches modes immediately, with no toggle or
setting involved.

## The table of contents

The sidebar shows two stacked lists: the file browser on top and, once a
document with headings is open, its table of contents below. The contents
list is a navigation aid and a "you are here" marker at once, and making it
work threads a single fact, a heading's position, through three modules
without any of them having to know the whole story.

Parsing is where a heading becomes trackable. As `markdown::render_markdown`
walks the event stream, each heading it closes is recorded as a `Heading`
carrying its level, its text, and the index of the block that holds it. A
block index is the right handle to keep, rather than a row or a page, because
it survives reflow. The row a heading sits on changes every time the column
width changes; which block it is does not.

Turning that block index into a page is a job for layout, and only at draw
time. `wrap::layout` already walks every block to produce the rows, so it
records, as it goes, the row each block starts on, and `paginate` turns each
of those rows into the page that holds it. To jump to a heading, `ui` asks
the `Pages` which page its block begins on and lands there. The same lookup,
read the other way, answers the reverse question: which heading is the
current page under. That is just the last heading that begins on or before
the last page in view, and because headings and their pages both run in
document order, the search stops at the first heading that has not appeared
yet.

The event handler, in the middle, knows none of this. Choosing a heading only
sets `pending_jump` to its block index and hands focus to the reader. It never
computes a page, because at the moment a key is pressed the page size that a
page depends on is not its to know. The draw step owns that knowledge, so the
draw step does the conversion, on the next frame, and clears the request.
This is the same division of labor that lets `G` ask for the last page
without computing it: the key handler states an intention, and the one place
that knows the true bounds resolves it.

## Remembering the reading position

booknook reopens a document on the page it was left on, and launched with no
argument it reopens the last file entirely, the way a Kindle returns to the
book you closed. All of that lives in the `session` module and a handful of
fields on `App`, and the shape of it follows one decision: the position is
remembered per file, not once globally, because a reader expects to return to
the middle of a long essay it left yesterday even after dipping into three
other files since.

So `App` holds a map from a file's path to the page reached in it. The map is
loaded from the saved session at startup and written back on quit, and only
the open file's entry changes while running. Two moments update it: opening a
different file, which records the outgoing file's page before switching, and
quitting, which records the current file's page before saving. Quitting is
`Reader::on_exit`, which eframe calls however the window is closed, whether
by `q` or by the window's own close button. Between those, the live page
number is enough; there is no need to touch the map on every page turn.

The keys in that map are canonical paths, resolved through
`fs::canonicalize`, so the same file reached by a relative path, an absolute
one, or a symlink all land on one entry rather than three. A path that cannot
be canonicalized, because the file has since moved, falls back to its own
form, which is still a consistent key for the life of the process.

The state file itself is deliberately plain text, one `key\tvalue` line at a
time, not JSON. That keeps `session` free of any serialization dependency,
keeps the file readable and hand-editable, and makes forward compatibility
trivial: a line whose key the parser does not recognize is skipped, so an
older build reading a file written by a newer one simply ignores the fields
it does not understand rather than failing to load. Reading and writing are
split into a pure parse-and-serialize pair with the file I/O wrapped around
them, which is what lets the format be tested by round-tripping a `Session`
through a string with no filesystem involved.

## Ownership and the borrow checker in practice

booknook was also built as a way to learn Rust by writing it, and its
ownership choices are worth walking through on their own, since they show
up as concrete decisions rather than abstract rules.

**Owned data outlives the parser that produced it.** pulldown-cmark hands
`markdown::render_markdown` borrowed text, in the form of a `CowStr` tied to
the lifetime of the original markdown string. Every span kept in a
`RenderLine`, though, is built with `Span::styled(text.into_string(),
style)`, which copies that borrowed text into an owned `String`. This is
what lets `Vec<RenderLine>` live on `App` as `blocks` with no lifetime
attached to it. If the spans still borrowed from the source string, the
parsed document could not outlive the local variable holding the raw file
contents inside `load_file`, and `App` would need a lifetime parameter of
its own just to hold a parsed document. Paying for a handful of string
copies once, at load time, avoids that entirely.

**`mem::take` moves a value out of a mutable reference.** Rust will not let
you move a value out of a `&mut Vec<T>` and leave the original variable
behind in an undefined state; the compiler has no way to know what should
be there afterward. `std::mem::take` is the escape hatch: it swaps in a
fresh, empty `Vec` and returns the old one, fully owned. Both
`markdown::flush_prose` and the word-wrap loop in `wrap::wrap_prose` use
this to hand off a batch of spans, `std::mem::take(spans)` and
`std::mem::take(&mut current)`, without needing to clone anything or
restructure the surrounding loop.

**Borrowing decides a function's signature, not just its body.** The
methods on `Reader` that only read state, such as `status_bar` and
`legend`, take `&self`. The ones that also need to clamp `app.page` once the
true page count is known, such as `prepare`, or to remember where a sidebar
list was scrolled, such as `files`, take `&mut self` instead. This is not an
arbitrary choice enforced after the fact. A shared reference simply does not
compile against code that assigns to a field, so the signature itself is a
true statement about what the function can do, readable without looking at
the body at all.

**Methods keep a single mutable borrow instead of many.** `App::load_file`
and `App::enter_dir` take `&mut self` and write several related fields
inside one method body. Before this existed as methods, the equivalent free
functions took `app: &mut App` as a parameter, which works the same way at
the borrow-checker level, but grouping the writes as methods on `App`
itself keeps the invariant, that `dir` and `entries` always change
together, or that loading a file always clears the contents cursor and any
pending jump, defined in exactly one place rather than trusted to every call
site.

**Disjoint fields can be borrowed at the same time.** `Reader::prepare`
keeps a shared borrow of the cached layout alive while it writes to the app:

```rust
let pages = &self.cache.as_ref()?.pages;
// ...
self.app.pan = self.app.pan.min((max_pan / mono_width).ceil() as u16);
if let Some(block) = self.app.pending_jump.take() {
    self.app.page = page_number(pages.page_of_block(block));
}
```

`pages` borrows `self.cache` until the end of the function, and yet
`self.app` is assigned to while it is alive. That compiles because, inside a
single function body, the borrow checker tracks each field of a struct
separately: a shared borrow of `self.cache` and a mutable use of `self.app`
touch different memory, so they may overlap. The same lines would not
compile if the page were set through a helper method taking `&mut self`,
because a method call borrows the whole of `self`, cache included. That is
why `prepare` assigns the fields directly. The layout step just above it
leans on the same rule: it borrows `self.app.blocks` for the closure that
lays the document out, and then assigns `self.cache`, a different field.

**The smallest possible clone breaks a borrow conflict.** In
`events::handle_files_key`, opening the selected entry looks like this:

```rust
if let Some(entry) = app.entries.get(app.selected) {
    if entry.is_dir {
        let target = entry.path.clone();
        app.enter_dir(target);
    }
    // ...
}
```

`entry` is a shared reference borrowed from `app.entries`, which is itself
part of `app`. Calling `app.enter_dir(...)` needs a mutable borrow of the
whole `App`, which cannot coexist with a live shared borrow of one of its
fields. Cloning `entry.path` into an owned `PathBuf` first, called `target`,
ends the dependency on `entry` before `app.enter_dir` is called. Under
Rust's non-lexical lifetimes, a borrow's lifetime ends at its last actual
use, not at the end of the block it was created in, so the borrow of
`app.entries` through `entry` is already over by the time `target` is used,
and the compiler accepts the mutable borrow that follows without any extra
scoping. This pattern, clone the one small piece of data that truly needs
to survive, then let the original borrow end, comes up often enough in this
codebase that it is worth recognizing on sight rather than re-deriving each
time.

## Extending booknook

A new inline markdown feature, such as strikethrough or footnotes, belongs
in `markdown::render_markdown`. Most of these follow the same shape as the
existing handlers: push the current style onto `style_stack` on the matching
`Start` event, derive a new one with a builder method on `Style`, and pop it
back off on the matching `End` event. If the feature needs a look of its
own, it gets a field on `Style`, and `Typesetter::format` learns what that
field means on screen.

A new block-level feature, such as tables, is more work, because it has to
decide how that block behaves under `wrap::layout`. Something that should
reflow with the rest of the paragraph belongs in a `RenderLine::Prose`
block, whose `indent` and `hang` fields control where its first row and its
continuation rows begin. Note that the word splitter discards leading
whitespace, so an indent has to travel in that `indent` field rather than
as spaces baked into the text. Something with its own fixed shape, the way
a code block does, belongs in one or more `RenderLine::Verbatim` lines
instead. A separation between blocks is a `RenderLine::Gap`, which lets
`wrap` decide how much space it is actually worth.

A new color palette is an entry appended to the `THEMES` static in `theme`,
and nothing else. Nothing needs to change anywhere else, because `t` cycles
by index over whatever is in that array.

A whole new pane, alongside the sidebar and the reader, would touch four
modules. `app` would need new state for whatever that pane shows. `events`
would need a new key handler, and a way to route keys to it when it has
focus. `ui` would need a method on `Reader` that draws it, added as a panel
in `Reader::frame`, and `document_area` would need to leave room for it, so
the page is laid out for the space it will actually get. `main` would not
need to change at all, since it only hands the `App` to the window.
