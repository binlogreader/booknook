//! Turning keyboard input into state changes.
//!
//! egui reports input as a list of events per frame. This module reads that
//! list, reduces each event to a `KeyCode`, and hands it to the handler for
//! whichever pane has focus. Each handler takes the current `App` and a
//! `KeyCode` and decides what should change. Nothing in this module draws
//! anything. That is the `ui` module's job.
//!
//! Characters and named keys arrive by different routes. A typed character,
//! whatever the keyboard layout, comes as text, which is why `{` and `+`
//! work without this module knowing which physical keys produce them. The
//! arrows, Tab, Enter, and the rest come as key presses. Anything held with
//! Ctrl is left alone: those chords belong to egui, which uses Ctrl with
//! plus, minus, and zero to zoom.

use eframe::egui;

use crate::anim::Direction;
use crate::app::{App, Focus, MAX_PAGE_WIDTH, MAX_SPACING, MIN_PAGE_WIDTH};
use crate::browser::is_readable;

/// A key, as far as booknook cares: a typed character, or one of the named
/// keys it binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyCode {
    Char(char),
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Tab,
    Enter,
    Backspace,
    Esc,
}

/// Apply every key pressed since the last frame, in order.
pub(crate) fn handle_input(ctx: &egui::Context, app: &mut App) {
    let keys: Vec<KeyCode> = ctx.input(|input| input.events.iter().flat_map(translate).collect());
    for key in keys {
        handle_key(app, key);
        if app.quit {
            break;
        }
    }
}

/// The keys one egui event stands for. Text can carry several characters at
/// once, from an input method or a fast typist, so this returns a list.
/// Releases are ignored; holding a key repeats its press.
fn translate(event: &egui::Event) -> Vec<KeyCode> {
    match event {
        egui::Event::Text(text) => text.chars().map(KeyCode::Char).collect(),
        egui::Event::Key { key, pressed: true, modifiers, .. } if !modifiers.command && !modifiers.alt => {
            let code = match key {
                egui::Key::ArrowLeft => KeyCode::Left,
                egui::Key::ArrowRight => KeyCode::Right,
                egui::Key::ArrowUp => KeyCode::Up,
                egui::Key::ArrowDown => KeyCode::Down,
                egui::Key::PageUp => KeyCode::PageUp,
                egui::Key::PageDown => KeyCode::PageDown,
                egui::Key::Tab => KeyCode::Tab,
                egui::Key::Enter => KeyCode::Enter,
                egui::Key::Backspace => KeyCode::Backspace,
                egui::Key::Escape => KeyCode::Esc,
                _ => return Vec::new(),
            };
            vec![code]
        }
        _ => Vec::new(),
    }
}

/// Act on one key.
pub(crate) fn handle_key(app: &mut App, code: KeyCode) {
    // A notice answers the last action; the next key moves on from it.
    app.notice = None;

    // Quitting and switching focus work from either pane, so they are
    // handled once here instead of being duplicated in both key handlers.
    match code {
        KeyCode::Char('q') | KeyCode::Esc => {
            app.quit = true;
            return;
        }
        KeyCode::Tab => {
            // Files, then the contents list, then the reader, then back. The
            // contents step is skipped when the open document has no headings,
            // or none is open, so Tab never lands on an empty pane.
            app.focus = match app.focus {
                Focus::Files if !app.headings.is_empty() => Focus::Toc,
                Focus::Files => Focus::Document,
                Focus::Toc => Focus::Document,
                Focus::Document => Focus::Files,
            };
            return;
        }
        KeyCode::Char('t') => {
            app.cycle_theme();
            return;
        }
        KeyCode::Char('a') => {
            app.animate = !app.animate;
            return;
        }
        KeyCode::Char('r') => {
            app.reload();
            return;
        }
        _ => {}
    }

    match app.focus {
        Focus::Files => {
            // A file that will not open, a damaged book or one deleted since
            // the listing was read, says so in the status bar and leaves the
            // reader where it was.
            if let Err(err) = handle_files_key(app, code) {
                app.notice = Some(format!("{err:#}"));
            }
        }
        Focus::Toc => handle_toc_key(app, code),
        Focus::Document => handle_document_key(app, code),
    }
}

/// Move the selection, descend into a directory, go back up, or open a
/// markdown file. Right, `l`, and Enter mean "go deeper." Left, `h`, and
/// Backspace mean "go back," which are the same directions the reader uses
/// for page turns.
fn handle_files_key(app: &mut App, code: KeyCode) -> anyhow::Result<()> {
    match code {
        KeyCode::Char('j') | KeyCode::Down => {
            if !app.entries.is_empty() {
                app.selected = (app.selected + 1).min(app.entries.len() - 1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => app.selected = app.selected.saturating_sub(1),
        KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => {
            if let Some(parent) = app.dir.parent() {
                let parent = parent.to_path_buf();
                app.enter_dir(parent);
            }
        }
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
            if let Some(entry) = app.entries.get(app.selected) {
                if entry.is_dir {
                    let target = entry.path.clone();
                    app.enter_dir(target);
                } else if is_readable(&entry.path) {
                    let target = entry.path.clone();
                    app.load_file(&target)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Move through the contents list and jump to a heading. Right, `l`, Enter,
/// and space all mean "take me there," matching the reader's own
/// forward-motion keys. Left and `h` step back to the file list, the same
/// direction that goes up a folder.
fn handle_toc_key(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Char('j') | KeyCode::Down => {
            if !app.headings.is_empty() {
                app.toc_selected = (app.toc_selected + 1).min(app.headings.len() - 1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => app.toc_selected = app.toc_selected.saturating_sub(1),
        KeyCode::Char('g') => app.toc_selected = 0,
        KeyCode::Char('G') => app.toc_selected = app.headings.len().saturating_sub(1),
        KeyCode::Char('l' | ' ') | KeyCode::Right | KeyCode::Enter => app.jump_to_heading(app.toc_selected),
        KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => app.focus = Focus::Files,
        _ => {}
    }
}

/// The direction a key turns the page, if it turns it at all. Everything
/// else, jumps and typography included, returns `None` and is left
/// un-animated.
pub(crate) fn turn_direction(code: KeyCode) -> Option<Direction> {
    match code {
        KeyCode::Char(' ' | 'l' | 'j') | KeyCode::Right | KeyCode::Down | KeyCode::PageDown => Some(Direction::Forward),
        KeyCode::Char('h' | 'k') | KeyCode::Left | KeyCode::Up | KeyCode::PageUp | KeyCode::Backspace => {
            Some(Direction::Back)
        }
        _ => None,
    }
}

/// How many monospace characters one press of `,` or `.` pans wide code
/// sideways. Big enough to make progress across a wide diagram, small
/// enough that the eye keeps its place between presses.
const PAN_STEP: u16 = 8;

/// Turn `app.page` by one step in `dir`, clamped at zero. The upper bound is
/// not known here, since the last page depends on the page size, so a
/// forward turn is left to be clamped at draw time the same way `G` is.
/// Turning a page also ends any sideways pan: the lean-in to inspect a wide
/// figure is over once the reader moves on.
fn turn_page(app: &mut App, dir: Direction) {
    let step = if app.spread { 2 } else { 1 };
    match dir {
        Direction::Forward => app.page = app.page.saturating_add(step),
        Direction::Back => app.page = app.page.saturating_sub(step),
    }
    app.pan = 0;
}

fn handle_document_key(app: &mut App, code: KeyCode) {
    // In a two-page spread, a page turn flips the whole spread, both
    // pages, the way it would with a real book, rather than just the one
    // page currently in view.
    if let Some(dir) = turn_direction(code) {
        turn_page(app, dir);
        // Flag the turn so the `ui` module knows to animate it, if animation
        // is on. Jumps and typography deliberately do not set this.
        app.page_turn = true;
        return;
    }
    match code {
        KeyCode::Char('g') => {
            app.page = 0;
            app.pan = 0;
        }
        // The last page number is not known until the `ui` module computes
        // it from the page size, so this asks for "as far as possible" and
        // lets the draw step clamp it to something real.
        KeyCode::Char('G') => {
            app.page = u16::MAX;
            app.pan = 0;
        }
        KeyCode::Char('o') => app.focus = Focus::Files,

        // Pan wide code blocks and diagrams sideways, the keyboard version
        // of a horizontal scrollbar. Like `G`, the upper bound is not known
        // here: only layout knows the widest verbatim line at the current
        // width, so this asks for more and lets the draw step clamp it.
        KeyCode::Char('.' | '>') => app.pan = app.pan.saturating_add(PAN_STEP),
        KeyCode::Char(',' | '<') => app.pan = app.pan.saturating_sub(PAN_STEP),

        // Typography, adjustable while reading. Changing any of these
        // reflows the document on the next frame, which can move the text
        // currently on screen onto a different page, so they deliberately
        // leave `page` alone rather than trying to preserve a position.
        KeyCode::Char('[') => app.spacing.line = app.spacing.line.saturating_sub(1),
        KeyCode::Char(']') => app.spacing.line = (app.spacing.line + 1).min(MAX_SPACING),
        KeyCode::Char('{') => app.spacing.paragraph = app.spacing.paragraph.saturating_sub(1),
        KeyCode::Char('}') => app.spacing.paragraph = (app.spacing.paragraph + 1).min(MAX_SPACING),
        KeyCode::Char('-') => app.page_width = app.page_width.saturating_sub(2).max(MIN_PAGE_WIDTH),
        KeyCode::Char('=' | '+') => app.page_width = (app.page_width + 2).min(MAX_PAGE_WIDTH),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Panning steps sideways with `.` and back with `,`, and a page turn
    /// drops the pan entirely: the lean-in to a wide figure ends when the
    /// reader moves on.
    #[test]
    fn pan_steps_and_a_page_turn_resets_it() {
        let mut app = App::new();
        handle_document_key(&mut app, KeyCode::Char('.'));
        handle_document_key(&mut app, KeyCode::Char('.'));
        assert_eq!(app.pan, 2 * PAN_STEP);
        handle_document_key(&mut app, KeyCode::Char(','));
        assert_eq!(app.pan, PAN_STEP);

        handle_document_key(&mut app, KeyCode::Char(' '));
        assert_eq!(app.pan, 0, "turning the page ends the pan");
        assert!(app.page_turn, "the turn itself must still register");
    }

    /// Typed characters arrive as text and named keys as presses. A chord
    /// held with Ctrl belongs to egui's zoom and must not reach booknook,
    /// or Ctrl and minus would also narrow the column.
    #[test]
    fn events_translate_to_keys_and_ctrl_chords_are_left_alone() {
        assert_eq!(translate(&egui::Event::Text("]}".into())), vec![KeyCode::Char(']'), KeyCode::Char('}')]);

        let press = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        assert_eq!(translate(&press(egui::Key::ArrowRight, egui::Modifiers::NONE)), vec![KeyCode::Right]);
        assert_eq!(translate(&press(egui::Key::Minus, egui::Modifiers::COMMAND)), vec![]);
        assert_eq!(translate(&press(egui::Key::Tab, egui::Modifiers::COMMAND)), vec![]);
    }

    /// A file that fails to open leaves a notice for the status bar instead
    /// of ending the program, and the next key clears it.
    #[test]
    fn a_failed_open_leaves_a_notice() {
        let dir = std::env::temp_dir().join(format!("booknook-notice-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("broken.epub"), b"not a zip").unwrap();

        let mut app = App::new();
        app.enter_dir(dir.clone());
        handle_key(&mut app, KeyCode::Enter);
        std::fs::remove_dir_all(&dir).ok();

        assert!(app.notice.is_some(), "the failure should be reported");
        assert!(app.blocks.is_empty(), "nothing should have opened");
        handle_key(&mut app, KeyCode::Down);
        assert!(app.notice.is_none(), "the next key moves on from it");
    }
}
