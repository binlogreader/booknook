//! The page-turn animation: a leaf swept across the sheet.
//!
//! Turning a page is the one place booknook moves on its own. Everywhere
//! else a frame is drawn once and the program goes back to sleep until the
//! next key. The turn is the exception: for a fifth of a second it asks egui
//! for frame after frame while a fold sweeps across the sheet, the outgoing
//! page still showing on one side of it and the incoming page revealed on
//! the other, the way a thumb carries a leaf across an open book.
//!
//! The fold is drawn with two cues and nothing else: a narrow strip of the
//! leaf's blank back catching the light where it stands up, and a band of
//! shadow it casts on the page coming into view. A lit edge and the shadow
//! behind it are what read as depth, and they keep the text on both pages
//! legible for the whole turn instead of distorting it.
//!
//! Only the sheet moves. The sidebar and the status bar show the settled
//! destination from the first frame, so the motion reads as a page turning
//! inside the book rather than the whole window animating.
//!
//! The effect is cheap: both pages are already laid out, so each frame just
//! paints them again under a moving clip. It is off by default, and a turn
//! that lands on the same page, such as pressing forward on the last page,
//! never starts one.

use eframe::egui::{Color32, Mesh, Painter, Rect, Shape, pos2};

/// Which way the pages travel. A forward turn carries the leaf from the
/// right edge of the sheet to the left; a back turn mirrors it, so paging
/// backward reads as the reverse motion of paging forward.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Forward,
    Back,
}

/// How long a turn takes, in seconds: long enough to read as a turn, short
/// enough to never feel in the way.
const DURATION: f64 = 0.22;

/// How wide the lit edge of the leaf and the shadow behind it are, in
/// points. The edge is kept thin, a lit crease; the shadow is wider, since a
/// soft band of shade reads as depth where a hard line would read as a seam.
const EDGE: f32 = 5.0;
const SHADOW: f32 = 28.0;

/// A turn in progress.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Turn {
    /// The page the turn left, painted on the part of the sheet the fold
    /// has not yet crossed.
    pub(crate) from: u16,
    pub(crate) dir: Direction,
    /// When the turn began, on egui's clock.
    start: f64,
    /// The document the turn belongs to. A reload or a new document ends
    /// it, since `from` would point into pages that no longer exist.
    pub(crate) generation: u64,
}

impl Turn {
    pub(crate) fn new(from: u16, dir: Direction, start: f64, generation: u64) -> Self {
        Turn { from, dir, start, generation }
    }

    /// How far along the turn is at `now`, from 0 to 1, eased so the leaf
    /// lifts away quickly and settles gently.
    pub(crate) fn progress(&self, now: f64) -> f32 {
        let t = ((now - self.start) / DURATION).clamp(0.0, 1.0) as f32;
        1.0 - (1.0 - t).powi(3)
    }

    pub(crate) fn done(&self, now: f64) -> bool {
        now - self.start >= DURATION
    }

    /// Where the fold stands across `sheet` at `progress`.
    fn fold_x(&self, sheet: Rect, progress: f32) -> f32 {
        match self.dir {
            Direction::Forward => sheet.right() + (sheet.left() - sheet.right()) * progress,
            Direction::Back => sheet.left() + (sheet.right() - sheet.left()) * progress,
        }
    }

    /// The part of the sheet still showing the outgoing page: the side of
    /// the fold it has not yet swept.
    pub(crate) fn old_region(&self, sheet: Rect, progress: f32) -> Rect {
        let x = self.fold_x(sheet, progress);
        match self.dir {
            Direction::Forward => Rect::from_min_max(sheet.min, pos2(x, sheet.max.y)),
            Direction::Back => Rect::from_min_max(pos2(x, sheet.min.y), sheet.max),
        }
    }

    /// Paint the fold over both pages: the lit edge on the outgoing side and
    /// the shadow on the incoming one. `paper` is the page color the edge
    /// is lightened from.
    pub(crate) fn paint_fold(&self, painter: &Painter, sheet: Rect, progress: f32, paper: Color32) {
        let x = self.fold_x(sheet, progress);
        let (edge, shadow) = match self.dir {
            Direction::Forward => (x - EDGE..=x, x..=x + SHADOW),
            Direction::Back => (x..=x + EDGE, x - SHADOW..=x),
        };
        let edge = Rect::from_x_y_ranges(edge, sheet.y_range()).intersect(sheet);
        let shadow = Rect::from_x_y_ranges(shadow, sheet.y_range()).intersect(sheet);
        if edge.is_positive() {
            painter.rect_filled(edge, 0.0, lift(paper));
        }
        if shadow.is_positive() {
            let dark = Color32::from_black_alpha(70);
            let (left, right) = match self.dir {
                Direction::Forward => (dark, Color32::TRANSPARENT),
                Direction::Back => (Color32::TRANSPARENT, dark),
            };
            painter.add(gradient(shadow, left, right));
        }
    }
}

/// A rectangle shaded from `left` to `right`. Used for the fold's shadow and
/// for the shading either side of a spread's spine.
pub(crate) fn gradient(rect: Rect, left: Color32, right: Color32) -> Shape {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), left);
    mesh.colored_vertex(rect.right_top(), right);
    mesh.colored_vertex(rect.right_bottom(), right);
    mesh.colored_vertex(rect.left_bottom(), left);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    Shape::mesh(mesh)
}

/// Lighten a color toward white, for the lit edge of the lifting leaf.
fn lift(color: Color32) -> Color32 {
    let toward_white = |v: u8| (v as u16 + (255 - v as u16) * 45 / 100) as u8;
    Color32::from_rgb(toward_white(color.r()), toward_white(color.g()), toward_white(color.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet() -> Rect {
        Rect::from_min_max(pos2(100.0, 0.0), pos2(500.0, 300.0))
    }

    /// A forward turn starts with the whole sheet showing the old page and
    /// ends with none of it, the fold having crossed from right to left.
    #[test]
    fn a_forward_turn_sweeps_from_the_right_edge_to_the_left() {
        let turn = Turn::new(4, Direction::Forward, 0.0, 0);
        assert_eq!(turn.old_region(sheet(), 0.0), sheet());
        let halfway = turn.old_region(sheet(), 0.5);
        assert_eq!((halfway.left(), halfway.right()), (100.0, 300.0));
        assert!(!turn.old_region(sheet(), 1.0).is_positive(), "nothing of the old page remains");
    }

    /// A back turn is the mirror image: the old page holds the right side
    /// and gives way from the left.
    #[test]
    fn a_back_turn_sweeps_from_the_left_edge_to_the_right() {
        let turn = Turn::new(4, Direction::Back, 0.0, 0);
        let halfway = turn.old_region(sheet(), 0.5);
        assert_eq!((halfway.left(), halfway.right()), (300.0, 500.0));
        assert!(!turn.old_region(sheet(), 1.0).is_positive());
    }

    /// Progress runs from nothing to complete, never backward, and holds at
    /// complete once the turn's time is up.
    #[test]
    fn progress_eases_forward_and_clamps() {
        let turn = Turn::new(0, Direction::Forward, 10.0, 0);
        let samples: Vec<f32> = (0..=10).map(|i| turn.progress(10.0 + DURATION * i as f64 / 10.0)).collect();
        assert_eq!(samples[0], 0.0);
        assert_eq!(samples[10], 1.0);
        assert!(samples.windows(2).all(|w| w[0] <= w[1]), "the fold never moves backward");
        assert_eq!(turn.progress(99.0), 1.0);
        assert!(!turn.done(10.1) && turn.done(10.0 + DURATION));
    }
}
