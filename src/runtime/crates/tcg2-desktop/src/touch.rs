//! The console's buttons drawn for touch, where a phone or tablet has no
//! keyboard or gamepad (Android): the D-pad, A and B, Start and Select. In
//! portrait the picture sits across the top and the buttons below it, as
//! on a Game Boy; in landscape the picture is in the middle with the D-pad
//! and Select to its left, A, B and Start to its right. Each finger holds
//! the buttons under it, and sliding a finger moves to the button it
//! reaches. The buttons are drawn only with a new picture (or when a
//! touch changes which are held): they cost nothing on a still screen.

use crate::sdl::Renderer;
use tcg2_core::buttons;

/// A rectangle in window pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn centred(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            x: x - w / 2.0,
            y: y - h / 2.0,
            w,
            h,
        }
    }

    fn reaches(&self, x: f32, y: f32, margin: f32) -> bool {
        x >= self.x - margin
            && x <= self.x + self.w + margin
            && y >= self.y - margin
            && y <= self.y + self.h + margin
    }
}

/// A round button: its centre and radius, in window pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Circle {
    pub x: f32,
    pub y: f32,
    pub r: f32,
}

impl Circle {
    fn distance(&self, x: f32, y: f32) -> f32 {
        (x - self.x).hypot(y - self.y)
    }
}

/// Where the picture and the buttons go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The picture's area: the look's fit places the picture within it.
    pub screen: Rect,
    pub dpad: Circle,
    pub a: Circle,
    pub b: Circle,
    pub start: Rect,
    pub select: Rect,
}

/// How far around a button a touch still holds it, in its radius (or, for
/// Start and Select, in their height).
const REACH: f32 = 1.4;
/// The middle of the D-pad, in its radius, holds no direction.
const DEAD_ZONE: f32 = 0.2;
/// Each of the D-pad's directions holds this many degrees to either side
/// of its own: between two of them, both are held (a diagonal).
const DIRECTION_SPREAD: f32 = 60.0;
/// The buttons' white, over the picture or the black around it.
const ALPHA: f32 = 0.25;
const ALPHA_HELD: f32 = 0.55;
const ALPHA_LABEL: f32 = 0.8;

impl Layout {
    /// The layout for a `window` whose `safe` part is clear of cutouts, the
    /// system's bars and the edges its gestures start from (SDL's safe
    /// area). The buttons stay within it; the picture goes beyond it where
    /// only the gestures' edges are, at the window's sides in portrait and
    /// across its height in landscape.
    pub fn new(window: Rect, safe: Rect) -> Self {
        let Rect { x, y, w, h } = safe;
        let pill = |cx, cy, u: f32| Rect::centred(cx, cy, 0.17 * u, 0.06 * u);
        if h >= w {
            // Portrait: the picture across the top at the window's width
            // (160x144 is 0.9 as high as it is wide), below the cutout; the
            // buttons in the room below, scaled to it.
            let screen = Rect {
                x: window.x,
                y,
                w: window.w,
                h: (0.9 * window.w).min(0.55 * h),
            };
            let top = y + screen.h;
            let room = h - screen.h;
            let u = w.min(1.15 * room);
            let row = top + 0.42 * room;
            Layout {
                screen,
                dpad: Circle {
                    x: x + 0.26 * u,
                    y: row,
                    r: 0.2 * u,
                },
                a: Circle {
                    x: x + w - 0.16 * u,
                    y: row - 0.06 * u,
                    r: 0.085 * u,
                },
                b: Circle {
                    x: x + w - 0.36 * u,
                    y: row + 0.06 * u,
                    r: 0.085 * u,
                },
                select: pill(x + w / 2.0 - 0.12 * u, top + 0.86 * room, u),
                start: pill(x + w / 2.0 + 0.12 * u, top + 0.86 * room, u),
            }
        } else {
            // Landscape: the picture in the middle at the window's height,
            // the buttons beside it, scaled to the room there (over the
            // picture's edges where the screen is nearly square).
            let picture = (window.h / 0.9).min(window.w);
            let left = window.x + (window.w - picture) / 2.0;
            let side = (left - x).min(x + w - (left + picture));
            let u = (2.4 * side).clamp(0.5 * h, h);
            Layout {
                screen: window,
                dpad: Circle {
                    x: x + 0.24 * u,
                    y: y + 0.55 * h,
                    r: 0.19 * u,
                },
                a: Circle {
                    x: x + w - 0.13 * u,
                    y: y + 0.48 * h,
                    r: 0.08 * u,
                },
                b: Circle {
                    x: x + w - 0.32 * u,
                    y: y + 0.6 * h,
                    r: 0.08 * u,
                },
                select: pill(x + 0.24 * u, y + h - 0.1 * u, u),
                start: pill(x + w - 0.24 * u, y + h - 0.1 * u, u),
            }
        }
    }

    /// The buttons a finger at (x, y) holds.
    pub fn buttons_at(&self, x: f32, y: f32) -> u8 {
        let pad = self.dpad;
        let distance = pad.distance(x, y);
        if distance <= REACH * pad.r {
            if distance < DEAD_ZONE * pad.r {
                return 0;
            }
            // Degrees clockwise from the right (y grows downwards).
            let angle = (y - pad.y).atan2(x - pad.x).to_degrees();
            return [
                (0.0, buttons::RIGHT),
                (90.0, buttons::DOWN),
                (180.0, buttons::LEFT),
                (-90.0, buttons::UP),
            ]
            .into_iter()
            .filter(|&(axis, _)| ((angle - axis + 540.0) % 360.0 - 180.0).abs() < DIRECTION_SPREAD)
            .fold(0, |held, (_, button)| held | button);
        }
        // A or B, whichever is nearer.
        let (a, b) = (self.a.distance(x, y), self.b.distance(x, y));
        if a.min(b) <= REACH * self.a.r {
            return if a <= b { buttons::A } else { buttons::B };
        }
        for (rect, button) in [(self.start, buttons::START), (self.select, buttons::SELECT)] {
            if rect.reaches(x, y, (REACH - 1.0) * rect.h) {
                return button;
            }
        }
        0
    }

    /// Draws the buttons over the picture, those in `held` brighter.
    pub fn draw(&self, renderer: &mut Renderer, held: u8) {
        let alpha = |button| {
            if held & button != 0 {
                ALPHA_HELD
            } else {
                ALPHA
            }
        };
        // The D-pad's cross: its middle, and an arm for each direction.
        let Circle { x, y, r } = self.dpad;
        let t = r / 3.0;
        renderer.fill(Rect::centred(x, y, 2.0 * t, 2.0 * t), ALPHA);
        for (button, arm) in [
            (
                buttons::UP,
                Rect::centred(x, y - (r + t) / 2.0, 2.0 * t, r - t),
            ),
            (
                buttons::DOWN,
                Rect::centred(x, y + (r + t) / 2.0, 2.0 * t, r - t),
            ),
            (
                buttons::LEFT,
                Rect::centred(x - (r + t) / 2.0, y, r - t, 2.0 * t),
            ),
            (
                buttons::RIGHT,
                Rect::centred(x + (r + t) / 2.0, y, r - t, 2.0 * t),
            ),
        ] {
            renderer.fill(arm, alpha(button));
        }
        for (circle, button, name) in [(self.a, buttons::A, "A"), (self.b, buttons::B, "B")] {
            renderer.disc(circle.x, circle.y, circle.r, alpha(button));
            renderer.label(circle.x, circle.y, name, circle.r, ALPHA_LABEL);
        }
        for (rect, button, name) in [
            (self.select, buttons::SELECT, "SELECT"),
            (self.start, buttons::START, "START"),
        ] {
            renderer.fill(rect, alpha(button));
            // As tall as half the button, and narrower than it.
            let height = (0.5 * rect.h).min(0.8 * rect.w / name.len() as f32);
            renderer.label(
                rect.x + rect.w / 2.0,
                rect.y + rect.h / 2.0,
                name,
                height,
                ALPHA_LABEL,
            );
        }
    }
}

/// The fingers on the screen and the buttons each holds.
#[derive(Default)]
pub struct Fingers {
    held: Vec<(u64, u8)>,
}

impl Fingers {
    /// A finger down or moved, now holding `buttons`.
    pub fn set(&mut self, finger: u64, buttons: u8) {
        match self.held.iter_mut().find(|(id, _)| *id == finger) {
            Some(entry) => entry.1 = buttons,
            None => self.held.push((finger, buttons)),
        }
    }

    /// A finger lifted (or the touch cancelled).
    pub fn lift(&mut self, finger: u64) {
        self.held.retain(|(id, _)| *id != finger);
    }

    pub fn clear(&mut self) {
        self.held.clear();
    }

    /// The buttons all fingers hold, as the console's pad reports them:
    /// never two opposite directions at once.
    pub fn buttons(&self) -> u8 {
        let mut held = self.held.iter().fold(0, |held, &(_, b)| held | b);
        for pair in [buttons::LEFT | buttons::RIGHT, buttons::UP | buttons::DOWN] {
            if held & pair == pair {
                held &= !pair;
            }
        }
        held
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1080x2400 phone (the emulator's Pixel 6) and its safe area as SDL
    /// reports it there: the camera cutout and the edges the system's
    /// gestures start from left out.
    const PHONE: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1080.0,
        h: 2400.0,
    };
    const PORTRAIT: Rect = Rect {
        x: 78.0,
        y: 160.0,
        w: 924.0,
        h: 2177.0,
    };
    const PHONE_LANDSCAPE: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 2400.0,
        h: 1080.0,
    };
    const LANDSCAPE: Rect = Rect {
        x: 128.0,
        y: 62.0,
        w: 2194.0,
        h: 934.0,
    };

    fn inside(outer: Rect, inner: Rect) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.x + inner.w <= outer.x + outer.w + 0.01
            && inner.y + inner.h <= outer.y + outer.h + 0.01
    }

    fn bounds(c: Circle) -> Rect {
        Rect::centred(c.x, c.y, 2.0 * c.r, 2.0 * c.r)
    }

    fn overlap(a: Rect, b: Rect) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    #[test]
    fn buttons_stay_in_the_safe_area_apart_and_off_a_phones_picture() {
        for (window, safe) in [(PHONE, PORTRAIT), (PHONE_LANDSCAPE, LANDSCAPE)] {
            let layout = Layout::new(window, safe);
            let shapes = [
                bounds(layout.dpad),
                bounds(layout.a),
                bounds(layout.b),
                layout.start,
                layout.select,
            ];
            // The picture as the aspect fit places it in its area.
            let (x, y, w, h) =
                tcg2_render::Fit::Aspect.viewport((layout.screen.w as u32, layout.screen.h as u32));
            let picture = Rect {
                x: layout.screen.x + x,
                y: layout.screen.y + y,
                w,
                h,
            };
            assert!(inside(window, layout.screen), "{safe:?}");
            for (i, shape) in shapes.iter().enumerate() {
                assert!(inside(safe, *shape), "{safe:?}: {shape:?}");
                assert!(
                    !overlap(picture, *shape),
                    "{safe:?}: {shape:?} on {picture:?}"
                );
                for other in &shapes[i + 1..] {
                    assert!(!overlap(*shape, *other), "{safe:?}: {shape:?} on {other:?}");
                }
            }
        }
        // In portrait the picture fills the window's width, below the
        // cutout; in landscape its height.
        let screen = Layout::new(PHONE, PORTRAIT).screen;
        assert_eq!(
            (screen.x, screen.y, screen.w, screen.h),
            (0.0, 160.0, 1080.0, 972.0)
        );
        assert_eq!(
            Layout::new(PHONE_LANDSCAPE, LANDSCAPE).screen,
            PHONE_LANDSCAPE
        );
    }

    #[test]
    fn touches_hold_the_buttons_under_them() {
        for (window, safe) in [(PHONE, PORTRAIT), (PHONE_LANDSCAPE, LANDSCAPE)] {
            let l = Layout::new(window, safe);
            let at = |c: Circle, dx: f32, dy: f32| l.buttons_at(c.x + dx * c.r, c.y + dy * c.r);
            assert_eq!(at(l.dpad, 0.8, 0.0), buttons::RIGHT);
            assert_eq!(at(l.dpad, -0.8, 0.0), buttons::LEFT);
            assert_eq!(at(l.dpad, 0.0, -0.8), buttons::UP);
            assert_eq!(at(l.dpad, 0.0, 0.8), buttons::DOWN);
            assert_eq!(at(l.dpad, 0.6, 0.6), buttons::RIGHT | buttons::DOWN);
            assert_eq!(at(l.dpad, -0.6, -0.6), buttons::LEFT | buttons::UP);
            // 25 degrees off an axis holds that direction alone.
            let (s, c) = 25f32.to_radians().sin_cos();
            assert_eq!(at(l.dpad, 0.8 * c, -0.8 * s), buttons::RIGHT);
            assert_eq!(at(l.dpad, 0.05, 0.05), 0);
            assert_eq!(at(l.dpad, 1.3, 0.0), buttons::RIGHT);
            assert_eq!(at(l.a, 0.0, 0.0), buttons::A);
            assert_eq!(at(l.a, 1.2, 0.0), buttons::A);
            assert_eq!(at(l.b, 0.0, 0.0), buttons::B);
            let middle = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
            let (x, y) = middle(l.start);
            assert_eq!(l.buttons_at(x, y), buttons::START);
            let (x, y) = middle(l.select);
            assert_eq!(l.buttons_at(x, y), buttons::SELECT);
            let (x, y) = middle(l.screen);
            assert_eq!(l.buttons_at(x, y), 0);
        }
    }

    #[test]
    fn fingers_hold_their_own_buttons_until_lifted() {
        let mut fingers = Fingers::default();
        fingers.set(1, buttons::LEFT);
        fingers.set(2, buttons::B);
        assert_eq!(fingers.buttons(), buttons::LEFT | buttons::B);
        // Sliding from left to right.
        fingers.set(1, buttons::RIGHT);
        assert_eq!(fingers.buttons(), buttons::RIGHT | buttons::B);
        // A second finger on the opposite direction cancels both.
        fingers.set(3, buttons::LEFT);
        assert_eq!(fingers.buttons(), buttons::B);
        fingers.lift(3);
        fingers.lift(2);
        assert_eq!(fingers.buttons(), buttons::RIGHT);
        fingers.clear();
        assert_eq!(fingers.buttons(), 0);
    }
}
