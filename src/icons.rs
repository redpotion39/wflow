//! A small, consistent line-icon set drawn with egui shapes.
//!
//! The app used to decorate its interface with colourful emoji, which render
//! differently on every machine and fight each other for attention. These
//! icons are monochrome, share one stroke weight and grid, and stay legible at
//! 14-18 px, which gives the interface one quiet visual voice.
//!
//! Geometry is authored on a 24x24 grid (like Lucide/Feather) and scaled to
//! the requested size at draw time.

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Mic,
    History,
    Book,
    Download,
    Gear,
    Waveform,
    FileText,
    Target,
    Bolt,
    Clock,
    Server,
    Keyboard,
    Power,
    Collapse,
    Stop,
    Check,
    Copy,
    Paste,
    Trash,
    Swap,
    Alert,
    Activity,
    Flatline,
    Dot,
    Search,
    Sparkle,
}

/// Converts a point on the 24-unit design grid to screen space.
#[inline]
fn grid(center: Pos2, s: f32, x: f32, y: f32) -> Pos2 {
    Pos2::new(center.x + (x - 12.0) * s, center.y + (y - 12.0) * s)
}

/// Draws `icon` centred on `center` inside a `size` x `size` box.
pub fn draw(painter: &egui::Painter, center: Pos2, size: f32, color: Color32, icon: Icon) {
    let s = size / 24.0;
    let stroke = Stroke::new((size / 15.0).max(0.9), color);

    let poly = |coords: &[(f32, f32)]| {
        let pts: Vec<Pos2> = coords.iter().map(|(x, y)| grid(center, s, *x, *y)).collect();
        painter.add(Shape::line(pts, stroke));
    };
    let seg = |a: (f32, f32), b: (f32, f32)| {
        painter.line_segment([grid(center, s, a.0, a.1), grid(center, s, b.0, b.1)], stroke);
    };
    let circle = |c: (f32, f32), r: f32| {
        painter.circle_stroke(grid(center, s, c.0, c.1), r * s, stroke);
    };
    let dot = |c: (f32, f32), r: f32| {
        painter.circle_filled(grid(center, s, c.0, c.1), r * s, color);
    };
    let rrect = |min: (f32, f32), max: (f32, f32), round: f32| {
        painter.rect_stroke(
            Rect::from_min_max(grid(center, s, min.0, min.1), grid(center, s, max.0, max.1)),
            egui::Rounding::same(round * s),
            stroke,
        );
    };
    // Angles are in radians with y pointing down, so PI/2 is "6 o'clock".
    // Curves are approximated with short segments, which stays crisp at the
    // 12-18 px sizes these icons are drawn at.
    let arc = |c: (f32, f32), r: f32, a0: f32, a1: f32| {
        let steps = ((a1 - a0).abs() / ((std::f32::consts::PI / 12.0))).ceil() as usize + 2;
        let pts: Vec<Pos2> = (0..=steps)
            .map(|k| {
                let a = a0 + (a1 - a0) * k as f32 / steps as f32;
                grid(center, s, c.0 + r * a.cos(), c.1 + r * a.sin())
            })
            .collect();
        painter.add(Shape::line(pts, stroke));
    };

    match icon {
        Icon::Mic => {
            rrect((9.4, 2.5), (14.6, 13.0), 2.6);
            arc((12.0, 11.0), 6.2, 0.0, std::f32::consts::PI);
            seg((12.0, 17.2), (12.0, 21.0));
            seg((8.6, 21.0), (15.4, 21.0));
        }
        Icon::History => {
            // Clock face with an open gap and a rewind arrow at the top left.
            arc((12.0, 12.5), 8.2, 1.58 * std::f32::consts::PI, 2.92 * std::f32::consts::PI);
            poly(&[(3.2, 3.4), (3.2, 8.4), (8.2, 8.4)]);
            poly(&[(12.0, 8.6), (12.0, 12.5), (15.4, 14.2)]);
        }
        Icon::Book => {
            rrect((4.6, 3.0), (19.4, 21.0), 2.0);
            seg((8.4, 3.0), (8.4, 21.0));
        }
        Icon::Download => {
            seg((12.0, 3.2), (12.0, 14.6));
            poly(&[(7.2, 9.8), (12.0, 14.8), (16.8, 9.8)]);
            poly(&[(4.0, 17.0), (4.0, 20.4), (20.0, 20.4), (20.0, 17.0)]);
        }
        Icon::Gear => {
            circle((12.0, 12.0), 3.3);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                let (dx, dy) = (a.cos(), a.sin());
                seg(
                    (12.0 + dx * 5.4, 12.0 + dy * 5.4),
                    (12.0 + dx * 8.4, 12.0 + dy * 8.4),
                );
            }
        }
        Icon::Waveform => {
            seg((4.0, 10.0), (4.0, 14.0));
            seg((8.0, 6.0), (8.0, 18.0));
            seg((12.0, 3.0), (12.0, 21.0));
            seg((16.0, 8.0), (16.0, 16.0));
            seg((20.0, 10.5), (20.0, 13.5));
        }
        Icon::FileText => {
            poly(&[(14.0, 3.0), (6.0, 3.0), (6.0, 21.0), (18.0, 21.0), (18.0, 7.0), (14.0, 3.0), (14.0, 7.0), (18.0, 7.0)]);
            seg((9.0, 13.0), (15.0, 13.0));
            seg((9.0, 17.0), (13.0, 17.0));
        }
        Icon::Target => {
            circle((12.0, 12.0), 8.4);
            circle((12.0, 12.0), 4.8);
            dot((12.0, 12.0), 1.5);
        }
        Icon::Bolt => {
            poly(&[(13.2, 2.4), (4.8, 13.6), (11.4, 13.6), (10.2, 21.6), (19.2, 10.4), (12.6, 10.4), (13.2, 2.4)]);
        }
        Icon::Clock => {
            circle((12.0, 12.0), 8.4);
            poly(&[(12.0, 6.8), (12.0, 12.0), (16.0, 14.2)]);
        }
        Icon::Server => {
            rrect((4.0, 4.0), (20.0, 10.5), 2.0);
            rrect((4.0, 13.5), (20.0, 20.0), 2.0);
            dot((7.4, 7.2), 0.9);
            dot((7.4, 16.7), 0.9);
        }
        Icon::Keyboard => {
            rrect((2.5, 6.0), (21.5, 18.0), 2.2);
            for x in [6.0, 9.5, 13.0, 16.5] {
                dot((x, 10.0), 0.7);
            }
            seg((7.5, 14.4), (16.5, 14.4));
        }
        Icon::Power => {
            arc(
                (12.0, 12.6),
                7.6,
                1.72 * std::f32::consts::PI,
                4.28 * std::f32::consts::PI,
            );
            seg((12.0, 2.6), (12.0, 11.0));
        }
        Icon::Collapse => {
            poly(&[(6.0, 7.5), (12.0, 13.5), (18.0, 7.5)]);
            seg((5.0, 18.5), (19.0, 18.5));
        }
        Icon::Stop => {
            painter.rect_filled(
                Rect::from_min_max(grid(center, s, 7.0, 7.0), grid(center, s, 17.0, 17.0)),
                egui::Rounding::same(2.0 * s),
                color,
            );
        }
        Icon::Check => {
            poly(&[(4.5, 12.8), (9.5, 17.8), (19.5, 6.6)]);
        }
        Icon::Copy => {
            rrect((8.5, 8.5), (20.0, 20.0), 2.0);
            poly(&[(4.0, 14.0), (4.0, 4.0), (14.0, 4.0)]);
        }
        Icon::Paste => {
            rrect((5.5, 4.5), (18.5, 20.5), 2.0);
            rrect((9.0, 2.4), (15.0, 6.6), 1.4);
            poly(&[(8.6, 13.4), (11.0, 15.8), (15.4, 11.0)]);
        }
        Icon::Trash => {
            seg((4.0, 7.0), (20.0, 7.0));
            poly(&[(9.2, 7.0), (9.2, 4.2), (14.8, 4.2), (14.8, 7.0)]);
            poly(&[(6.2, 7.4), (7.2, 20.2), (16.8, 20.2), (17.8, 7.4)]);
            seg((10.2, 11.0), (10.2, 16.6));
            seg((13.8, 11.0), (13.8, 16.6));
        }
        Icon::Swap => {
            seg((4.0, 8.0), (20.0, 8.0));
            poly(&[(8.0, 4.2), (4.2, 8.0), (8.0, 11.8)]);
            seg((20.0, 16.0), (4.0, 16.0));
            poly(&[(16.0, 12.2), (19.8, 16.0), (16.0, 19.8)]);
        }
        Icon::Alert => {
            poly(&[(12.0, 3.2), (21.2, 19.6), (2.8, 19.6), (12.0, 3.2)]);
            seg((12.0, 9.6), (12.0, 14.2));
            dot((12.0, 17.0), 0.9);
        }
        Icon::Activity => {
            poly(&[(2.4, 12.8), (7.0, 12.8), (10.2, 4.8), (13.8, 19.2), (16.8, 12.8), (21.6, 12.8)]);
        }
        Icon::Flatline => {
            seg((3.5, 12.0), (20.5, 12.0));
        }
        Icon::Dot => {
            dot((12.0, 12.0), 3.4);
        }
        Icon::Search => {
            circle((10.6, 10.6), 6.4);
            seg((15.4, 15.4), (20.2, 20.2));
        }
        Icon::Sparkle => {
            poly(&[
                (12.0, 3.0),
                (13.6, 9.4),
                (20.0, 11.0),
                (13.6, 12.6),
                (12.0, 19.0),
                (10.4, 12.6),
                (4.0, 11.0),
                (10.4, 9.4),
                (12.0, 3.0),
            ]);
        }
    }
}
