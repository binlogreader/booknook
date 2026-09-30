# booknook

**A calm, book-like reader for markdown and EPUB.**

Your notes deserve better than a scrollback buffer, and your books deserve
better than a browser tab. booknook opens a markdown file or an EPUB as a
two-page spread, with a spine down the middle, generous margins, and a
palette chosen so the words come forward and everything else recedes. It
turns pages. It does not scroll.

```sh
cargo run
```

## Why

Most markdown viewers are developer tools wearing a reader's clothes. They
scroll, they syntax-highlight, they fill the window edge to edge, and they
treat a long essay exactly the way they treat a log file.

booknook is built on the opposite premise. It is a reading device that
happens to live in a window on your desktop, and every design decision
follows from that.

**Pages, not scrolling.** An e-ink reader flips whole pages. It never leaves
you halfway between two of them. Neither does booknook. Every keypress moves
a full page, or a full spread, and lands cleanly.

**A book, not a wall of text.** Give it a wide window and it opens two
pages side by side on a single sheet, with the spine drawn on the paper
rather than as a gap between panels. Narrow the window and it becomes a
single page, the way a phone-sized e-reader would. The decision is remade
every frame, so resizing just works.

**Typography you can feel.** booknook sets its body text in Helvetica, the
way Kindle offers it, or in Arial where Helvetica is not installed, at book
leading, and word-wraps the text
itself instead of handing that job to the GUI toolkit. That is what lets it
control the rhythm of a page: the measure of the column, the leading inside
a paragraph, and the larger gap between paragraphs. Those are three
different numbers, and getting the relationship between them right is most
of what separates a page from a transcript.

**Ink, not syntax highlighting.** Six palettes, cycled with a keypress:
Kindle's Sepia, Flexoki Light, and Paper on the light side, then Tokyo
Night, Catppuccin Mocha, and Rosé Pine for a dark room. The reading column
sits on its own slightly lighter shade, so the text rests on a sheet instead
of bleeding into the window background.

The palettes are borrowed, but the saturation is not. Those schemes were
built for syntax highlighting, where a bright color means something: this
token is a string, that one is a keyword. In prose it means nothing, and it
still takes the eye. The cost is real rather than aesthetic, because the eye
cannot focus two widely separated wavelengths at once and hunts between
them, which is felt as distraction rather than as blur. Technical writing
runs to about one inline-code span every twenty words, so an accent becomes
a scattering of bright flecks through a paragraph. Here every color is held
to a measured budget, and inline code is marked by tinting the paper behind
it rather than by recoloring the words. Color marks structure. Inside a
paragraph, weight and space do the work.

**Keyboard only.** Real e-readers have buttons, not pointers. Mouse support
was left out on purpose.

## Features

- Full markdown rendering: headings, bold and italic, inline code, fenced
  code blocks with language labels, blockquotes, ordered and nested
  unordered lists, and links
- EPUB reading: a whole book opens as one continuous document, in reading
  order, with the book's real title in place of its filename. Chapter
  headings feed the contents pane; when a book carries no usable headings,
  its own navigation file is used instead. Italics and bold survive even in
  Calibre conversions that encode them as styled spans
- A file browser sidebar that recedes while you read: the moment focus
  moves to the reader, the sidebar disappears and the page stands alone on
  the sheet, the way it would on an e-reader. Tab or `o` brings it back
- A table of contents below the browser, drawn from the open document's
  headings, that jumps straight to any of them and marks where you are
- Remembers your place. Every document reopens on the page you left it on,
  the way a Kindle returns to the book you were reading, and launching with
  no argument reopens the last file you had open
- Automatic two-page spread on wide windows, single page on narrow ones
- Book-quality page breaks: a page never ends on a heading, never strands
  a paragraph's first line at its bottom or sends the last line alone onto
  the next page, and never cuts a table row through a wrapped cell. When
  the arithmetic break would land badly, the page ends a line or two early
  instead, the way a typesetter leaves a short page rather than a bad break
- Live typography controls: text size, column width, line spacing, and
  paragraph spacing, all adjustable while reading and remembered between
  runs
- An optional page turn, off by default: a leaf swept across the sheet with
  a lit edge and a soft shadow behind it, for readers who want the motion
- Six color themes, cycled with a single key, opening on Kindle-style sepia,
  each tuned so that color marks structure and never lands inside a sentence
- A status bar that says where you are in the document and nothing else
  while you read. The settings and the key legend live in the sidebar, a
  Tab away, so that nothing at the edge of vision changes as you type
- Correct handling of smart punctuation, so `country's` and `$78.02` render
  as words rather than as fragments with spaces wedged into them
- Code blocks and ASCII diagrams keep their exact shape, clipped at the page
  edge rather than reflowed, so their alignment survives. When one is wider
  than the column, `,` and `.` pan it sideways, the keyboard version of a
  horizontal scrollbar, with `‹` and `›` marking what lies past each edge

## Install

Requires a recent stable Rust toolchain.

```sh
git clone https://github.com/ankit481/booknook
cd booknook
cargo build --release
```

The binary lands in `target/release/booknook`.

## Usage

With no arguments, booknook reopens the last document you had open, on the
page you left it on. The very first time, with nothing yet remembered, it
opens the file browser in the current directory instead. Pass a path to open
a file directly, or to start browsing somewhere specific.

```sh
booknook                    # reopen where you left off
booknook path/to/notes      # browse a folder
booknook path/to/file.md    # open a document
booknook path/to/book.epub  # open a book
```

### Keys

Available anywhere:

| Key | Action |
|---|---|
| `Tab` | Move focus: files, then contents, then the reader, then back |
| `t` | Cycle color theme |
| `a` | Turn the page-turn animation on or off |
| `r` | Reload the open document from disk |
| `Ctrl` `+` / `Ctrl` `-` | Larger or smaller text; `Ctrl` `0` resets |
| `q` / `Esc` | Quit |

In the file browser:

| Key | Action |
|---|---|
| `↑` `↓` or `k` `j` | Move the selection |
| `→` / `l` / `Enter` | Open a folder or a markdown file |
| `←` / `h` / `Backspace` | Go up to the parent folder |

In the contents list:

| Key | Action |
|---|---|
| `↑` `↓` or `k` `j` | Move the selection |
| `→` / `l` / `Enter` / space | Jump the reader to that heading |
| `g` / `G` | Jump to the first or last heading |
| `←` / `h` / `Backspace` | Back to the file browser |

In the reader:

| Key | Action |
|---|---|
| `→` / `l` / space | Turn to the next page or spread |
| `←` / `h` / `Backspace` | Turn back |
| `g` / `G` | Jump to the first or last page |
| `-` / `+` | Narrow or widen the reading column |
| `[` / `]` | Less or more space between lines |
| `{` / `}` | Less or more space between paragraphs |
| `,` / `.` | Pan wide code blocks sideways; a page turn resets |
| `o` | Back to the file browser |

## Getting the page right

In the terminal, booknook controlled the column, the paragraph rhythm, the
margins, and the color, but the typeface and the space between lines
belonged to the terminal emulator. In a window it sets all of them itself.

Body text is set in Helvetica, the plain sans Kindle offers among its
fonts, which keeps an even color across the page. Few machines have
Helvetica itself, so booknook falls back to faces drawn to its measure:
Arial, which was cut to Helvetica's exact widths and ships with Windows and
macOS, and then Nimbus Sans, TeX Gyre Heros, and Liberation Sans on Linux.
If it finds none of them it uses egui's built-in sans. Code is set in Hack,
which egui bundles, so it looks the same everywhere.

Leading is where most of the comfort comes from. Legibility research puts
the ideal leading for body text at roughly 1.2 to 1.45 times the type size,
and booknook starts at 1.4. Push it much past that, toward 2.0, and the eye
can no longer make the return sweep to the start of the next line reliably.
It overshoots, has to re-fixate, and the rhythm that lets you read without
noticing you are reading breaks down. That is the loose, hard-to-focus
feeling of over-spaced text. The gap between paragraphs is a separate and
larger step, and it is what your eye actually uses to tell one block from
the next.

If you prefer more air, it is one keystroke away: `]` opens up the leading
and `}` the gap between paragraphs. Text size follows `Ctrl` with `+` and
`-`, the way a browser zooms, and `Ctrl` `0` puts it back. All of these are
remembered between runs. The defaults are the recommendation, not a lock.

## Built with

pulldown-cmark and quick-xml for parsing, egui and eframe for the window,
and a hand-rolled word-wrapper and paginator in between. No async, no
unsafe, about five thousand lines of Rust.

If you want to know how the pieces fit together, or you are learning Rust and
want a small real codebase to read, see [docs/architecture.md](docs/architecture.md).
It covers the parse-then-wrap-then-paint pipeline, why pages are stored as
numbers rather than scroll offsets, and how ownership and borrowing show up
as concrete decisions throughout.

## License

MIT. See [LICENSE](LICENSE).
