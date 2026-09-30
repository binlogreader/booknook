//! Remembering where you left off, the way an e-reader does.
//!
//! This module knows nothing about an `App` or a window. It only knows how
//! to read and write a small file that records the last document opened, the
//! page reached in every document seen so far, and the typographic settings
//! in force. `app` reads a `Session` at startup to restore all of that, and
//! `main` writes one back out on quit.
//!
//! The reading position is stored per file, not just once, because a reader
//! expects to return to the middle of a long essay it left yesterday even
//! after dipping into three other files in between. That is what a Kindle
//! does: every book remembers its own place.
//!
//! The format is deliberately a plain, line-based, tab-separated text file,
//! not JSON, so that it stays readable in isolation, needs no serialization
//! dependency, and can be inspected or hand-edited without a tool. A line the
//! parser does not recognize is skipped rather than treated as an error, so a
//! newer field written by a future version does no harm when read by an
//! older one.

use std::collections::HashMap;
use std::path::PathBuf;

/// Everything that survives between runs. Loaded once at startup, written
/// once at quit. The fields mirror the adjustable settings on `App`, kept as
/// plain numbers here so this module depends on nothing above it.
pub(crate) struct Session {
    /// The document to reopen when booknook is launched with no path. `None`
    /// means the last run never opened a file, so start by browsing.
    pub(crate) last_file: Option<PathBuf>,
    pub(crate) page_width: u16,
    /// Blank rows inside a paragraph, matching `Spacing::line`.
    pub(crate) line: u16,
    /// Blank rows between paragraphs, matching `Spacing::paragraph`.
    pub(crate) para: u16,
    pub(crate) theme_index: usize,
    /// Whether page turns are animated. Stored as `0` or `1` in the file.
    pub(crate) animate: bool,
    /// The window's zoom factor, which is how text size is set.
    pub(crate) zoom: f32,
    /// The page reached in each document, keyed by its path. This is what
    /// makes returning to any file land on the page it was left on.
    pub(crate) positions: HashMap<PathBuf, u16>,
}

impl Default for Session {
    /// The settings a first-time run starts with, before any state file
    /// exists. These match `App::new` so that a fresh install and a restored
    /// one behave the same. Line spacing defaults to its first step, which
    /// is already book leading, and the paragraph gap carries the
    /// document's structure.
    fn default() -> Self {
        Session {
            last_file: None,
            page_width: 58,
            line: 0,
            para: 1,
            theme_index: 0,
            animate: false,
            zoom: 1.0,
            positions: HashMap::new(),
        }
    }
}

impl Session {
    /// Read the state file, falling back to defaults for anything missing.
    /// A missing file, an unreadable one, or a malformed line all resolve to
    /// a default rather than an error: losing a saved position is a small
    /// inconvenience, not a reason to refuse to start.
    pub(crate) fn load() -> Session {
        let Some(path) = state_path() else {
            return Session::default();
        };
        let Ok(contents) = std::fs::read_to_string(&path) else {
            return Session::default();
        };
        Session::parse(&contents)
    }

    /// Parse a state file's contents into a `Session`, filling in defaults
    /// for anything absent or malformed. Kept separate from `load` so the
    /// format can be exercised without any filesystem access.
    fn parse(contents: &str) -> Session {
        let mut session = Session::default();
        for line in contents.lines() {
            // Three fields at most: a key, a value, and an optional path that
            // may itself contain anything but a tab. splitn keeps the third
            // field whole, so a path is never split on stray characters.
            let mut parts = line.splitn(3, '\t');
            match (parts.next(), parts.next(), parts.next()) {
                (Some("file"), Some(value), _) => session.last_file = Some(PathBuf::from(value)),
                (Some("width"), Some(value), _) => {
                    if let Ok(n) = value.parse() {
                        session.page_width = n;
                    }
                }
                (Some("line"), Some(value), _) => {
                    if let Ok(n) = value.parse() {
                        session.line = n;
                    }
                }
                (Some("para"), Some(value), _) => {
                    if let Ok(n) = value.parse() {
                        session.para = n;
                    }
                }
                (Some("theme"), Some(value), _) => {
                    if let Ok(n) = value.parse() {
                        session.theme_index = n;
                    }
                }
                (Some("anim"), Some(value), _) => session.animate = value == "1",
                (Some("zoom"), Some(value), _) => {
                    if let Ok(n) = value.parse() {
                        session.zoom = n;
                    }
                }
                (Some("pos"), Some(page), Some(file)) => {
                    if let Ok(n) = page.parse() {
                        session.positions.insert(PathBuf::from(file), n);
                    }
                }
                _ => {}
            }
        }
        session
    }

    /// Write the state file, creating its parent directory if needed. Errors
    /// are returned rather than swallowed so the caller can decide, but in
    /// practice `main` ignores them: failing to save a position should never
    /// mask the real exit status of the program.
    pub(crate) fn save(&self) -> std::io::Result<()> {
        let Some(path) = state_path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, self.serialize())
    }

    /// Render this session in the state file's line-based format. Kept
    /// separate from `save` so a round trip can be checked without writing to
    /// disk.
    fn serialize(&self) -> String {
        let mut out = String::new();
        if let Some(file) = &self.last_file {
            out.push_str(&format!("file\t{}\n", file.display()));
        }
        out.push_str(&format!("width\t{}\n", self.page_width));
        out.push_str(&format!("line\t{}\n", self.line));
        out.push_str(&format!("para\t{}\n", self.para));
        out.push_str(&format!("theme\t{}\n", self.theme_index));
        out.push_str(&format!("anim\t{}\n", if self.animate { 1 } else { 0 }));
        out.push_str(&format!("zoom\t{}\n", self.zoom));
        for (file, page) in &self.positions {
            out.push_str(&format!("pos\t{}\t{}\n", page, file.display()));
        }
        out
    }
}

/// Where the state file lives: the platform's per-user data directory, under
/// a `booknook` folder. On Windows that is `%APPDATA%`, on Linux
/// `~/.local/share`, on macOS `~/Library/Application Support`. `dirs`
/// resolves the right one for each. If no such directory can be found, there
/// is nowhere to persist, and the reader simply forgets between runs.
fn state_path() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("booknook").join("session.tsv"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session written out and read back must come through unchanged,
    /// including a per-file page and a path with spaces in it.
    #[test]
    fn round_trips_through_the_state_format() {
        let mut positions = HashMap::new();
        positions.insert(PathBuf::from("/notes/a book.md"), 12u16);
        positions.insert(PathBuf::from("/notes/other.md"), 3u16);
        let session = Session {
            last_file: Some(PathBuf::from("/notes/a book.md")),
            page_width: 64,
            line: 0,
            para: 1,
            theme_index: 2,
            animate: true,
            zoom: 1.25,
            positions,
        };

        let restored = Session::parse(&session.serialize());

        assert_eq!(restored.last_file, session.last_file);
        assert_eq!(restored.page_width, 64);
        assert_eq!(restored.theme_index, 2);
        assert!(restored.animate, "the animation toggle should round-trip");
        assert_eq!(restored.zoom, 1.25, "the zoom should round-trip");
        assert_eq!(restored.positions.get(&PathBuf::from("/notes/a book.md")), Some(&12));
        assert_eq!(restored.positions.get(&PathBuf::from("/notes/other.md")), Some(&3));
    }

    /// Missing and garbled lines fall back to defaults rather than failing,
    /// and an unknown key from some future version is ignored.
    #[test]
    fn tolerates_missing_and_unknown_fields() {
        let session = Session::parse("width\tnonsense\ntheme\t3\nfuture\tvalue\n");
        assert_eq!(session.page_width, 58); // default, since the value did not parse
        assert_eq!(session.theme_index, 3);
        assert_eq!(session.last_file, None);
    }
}
