# What is booknook?

booknook is a desktop reader for markdown and EPUB. It lays a document out
as book pages, with a spine, margins, and a table of contents, and it turns
pages instead of scrolling. It exists because two everyday reading
situations are much worse than they need to be.

## Problem 1: reading a markdown doc inside an open PR

Design docs, TDDs, and runbooks increasingly arrive as a markdown file in a
pull request, and the surface you are given to read them is GitHub's diff
view. That view is built for comparing code, not for reading prose: the
document is chopped into a table of green added lines, sentences wrap
wherever the diff column runs out, and for a brand-new file there is nothing
to diff against, so the entire essay is rendered as one giant insertion. You
end up skimming line fragments and calling it a review.

booknook takes the PR URL directly and opens the file as it will read once
merged, laid out as pages:

```sh
# the whole PR: opens its first changed markdown file
booknook https://github.com/acme/data-platform/pull/869

# one specific changed file: copy its link from the PR's Files tab
booknook "https://github.com/acme/data-platform/pull/869/files#diff-4ae61…"
```

It shells out to the `gh` CLI you are already logged into, so private repos,
SSO, and enterprise hosts all work. Read the document as a document, then go
back to the diff view only to leave line comments.

Public gists and any raw markdown URL work the same way:

```sh
booknook https://gist.github.com/someone/abc123
```

## Problem 2: scrolling a long doc with no mental model of it

Open a 2,000-line markdown file in `less`, a browser tab, or your editor and
you get a wall of text with no shape. You do not know how long the document
is, where you are in it, or what its structure looks like, so you scroll by
feel. And because a scrolled document has no fixed geography, your spatial
memory gets nothing to hold onto: there is no "that diagram was near the top
of a left-hand page" to navigate back to, just a position on an endless
strip that looked the same everywhere.

booknook gives you the same anchors an e-reader does:

- **Pages, not a scrollbar.** Every keypress turns exactly one page or
  spread and lands cleanly, never halfway through a paragraph. The status
  bar reads `page 14 / 62`, so you always know where you are and how much
  is left.
- **A contents pane drawn from the headings.** The document's structure is
  visible before you read a word, it marks the section you are in, and it
  jumps straight to any heading.
- **It remembers your place.** Every document reopens on the page you left
  it on, and launching with no argument reopens the last thing you were
  reading.

## Try it

```sh
cargo build --release       # binary lands in target/release/booknook

booknook docs/architecture.md   # this repo's own design doc
booknook ~/notes                # browse a folder of notes
booknook book.epub              # open a book
booknook <PR or gist URL>       # read a doc out of a PR
```

Space or `→` turns the page, `Tab` toggles the sidebar, `t` cycles the
six themes. The full key table is in the [README](../README.md).
