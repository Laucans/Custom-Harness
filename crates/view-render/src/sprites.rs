//! The figures, painted as flat 2D cut-outs.
//!
//! Followers, robots and the steward stand in the world the way *Cult of the
//! Lamb* and *Don't Starve* stand paper characters: big heads, bead eyes, a
//! robe with a cult mark, thick ink around everything.
//!
//! Each figure is drawn with vector paths and rasterised at run time by
//! `tiny-skia` into an RGBA texture, which [`crate::app`] puts on a quad that
//! always faces the camera. Nothing here touches a file or the GPU, so every
//! figure is tested natively: dimensions, transparency where there is no
//! figure, ink where there is one, and a different robe per model.

// Coordinates read as placement — `CX + side * apart` — and a fused
// multiply-add would gain nothing on a few hundred points painted once.
#![allow(clippy::suboptimal_flops)]

use tiny_skia::{
    FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Rect, Stroke, Transform,
};

/// The width of every figure's texture, in pixels.
pub const WIDTH: u32 = 256;
/// The height of every figure's texture, in pixels.
pub const HEIGHT: u32 = 384;
/// The texture's vertical axis, where every figure stands.
const CX: f32 = 128.0;

/// The model a figure works on — what colours its robe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Model {
    /// Opus: violet.
    Opus,
    /// Sonnet: blue.
    Sonnet,
    /// Haiku: green.
    Haiku,
    /// Anything else, or none: grey-mauve.
    Unknown,
}

impl Model {
    /// From the name the picture carries.
    #[must_use]
    pub fn parse(name: Option<&str>) -> Self {
        match name {
            Some("opus") => Self::Opus,
            Some("sonnet") => Self::Sonnet,
            Some("haiku") => Self::Haiku,
            _ => Self::Unknown,
        }
    }

    /// The robe's colour, sRGB.
    #[must_use]
    pub const fn hue(self) -> Rgb {
        match self {
            Self::Opus => (155, 111, 214),
            Self::Sonnet => (79, 143, 214),
            Self::Haiku => (79, 179, 106),
            Self::Unknown => (150, 140, 160),
        }
    }
}

/// Which figure to paint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Figure {
    /// A follower at work: robe in the model's colour, hard hat.
    Follower {
        /// The model they work on.
        model: Model,
    },
    /// The steward: hooded, bearded, red-eyed, with a staff.
    Steward,
    /// A one-eyed robot; `wrench` builds, otherwise it holds a clipboard.
    Robot {
        /// The model it opens.
        model: Model,
        /// Builder or inspector.
        wrench: bool,
        /// Its eye and button glow while it works.
        active: bool,
    },
}

/// A painted figure: straight-alpha RGBA, row-major, top row first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sprite {
    /// Pixels across.
    pub width: u32,
    /// Pixels down.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// An opaque colour.
pub type Rgb = (u8, u8, u8);

// The palette, the scene's: cream and bone, skin, gold, red, pink — and an
// ink that is a dark plum rather than black, as a pen draws it.
const INK: Rgb = (42, 27, 46);
const CREAM: Rgb = (243, 231, 201);
const BONE: Rgb = (233, 220, 189);
const SKIN: Rgb = (241, 220, 192);
const GOLD: Rgb = (226, 178, 64);
const GOLD_DARK: Rgb = (184, 138, 40);
const RED: Rgb = (198, 57, 47);
const RED_DARK: Rgb = (150, 42, 42);
const PINK: Rgb = (230, 138, 176);
const WHITE: Rgb = (252, 250, 244);
const GREEN: Rgb = (126, 224, 129);
const GREY: Rgb = (154, 154, 166);
const LAVENDER: Rgb = (199, 210, 254);
const SHADOW: Rgb = (58, 42, 62);

/// The ink line, in pixels of the texture.
const LINE: f32 = 7.0;

/// The same colour, scaled.
fn shade(c: Rgb, k: f32) -> Rgb {
    (channel(c.0, k), channel(c.1, k), channel(c.2, k))
}

// Clamped to 0..=255 before the cast, so nothing truncates or goes negative.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn channel(v: u8, k: f32) -> u8 {
    (f32::from(v) * k).clamp(0.0, 255.0) as u8
}

/// A canvas with the plant's drawing habits: anti-aliased fills, round ink.
struct Canvas {
    pixmap: Pixmap,
}

impl Canvas {
    fn fill(&mut self, path: Option<&Path>, color: Rgb, alpha: u8) {
        let Some(path) = path else { return };
        let mut paint = Paint::default();
        paint.set_color_rgba8(color.0, color.1, color.2, alpha);
        paint.anti_alias = true;
        self.pixmap
            .fill_path(path, &paint, FillRule::Winding, Transform::identity(), None);
    }

    fn stroke(&mut self, path: Option<&Path>, color: Rgb, width: f32) {
        let Some(path) = path else { return };
        let mut paint = Paint::default();
        paint.set_color_rgba8(color.0, color.1, color.2, 255);
        paint.anti_alias = true;
        let stroke = Stroke {
            width,
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };
        self.pixmap
            .stroke_path(path, &paint, &stroke, Transform::identity(), None);
    }

    /// A filled shape with its ink line — how everything here is drawn.
    fn shape(&mut self, path: Option<Path>, color: Rgb) {
        self.fill(path.as_ref(), color, 255);
        self.stroke(path.as_ref(), INK, LINE);
    }

    /// A line of ink.
    fn line(&mut self, from: (f32, f32), to: (f32, f32), color: Rgb, width: f32) {
        let mut pb = PathBuilder::new();
        pb.move_to(from.0, from.1);
        pb.line_to(to.0, to.1);
        self.stroke(pb.finish().as_ref(), color, width);
    }
}

fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Option<Path> {
    Rect::from_xywh(cx - rx, cy - ry, rx * 2.0, ry * 2.0).and_then(PathBuilder::from_oval)
}

fn circle(cx: f32, cy: f32, r: f32) -> Option<Path> {
    PathBuilder::from_circle(cx, cy, r)
}

// `x`, `y`, `w`, `h`, `r`: the names a rectangle has had since geometry.
#[allow(clippy::many_single_char_names)]
fn rounded(x: f32, y: f32, w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

/// A closed shape through `points`, every corner softened by a curve — the
/// blob every body here is made of.
fn blob(points: &[(f32, f32)]) -> Option<Path> {
    let (Some(&last), Some(&first)) = (points.last(), points.first()) else {
        return None;
    };
    if points.len() < 3 {
        return None;
    }
    let mid = |a: (f32, f32), b: (f32, f32)| (f32::midpoint(a.0, b.0), f32::midpoint(a.1, b.1));
    let mut pb = PathBuilder::new();
    let start = mid(last, first);
    pb.move_to(start.0, start.1);
    for (&point, &next) in points.iter().zip(points.iter().cycle().skip(1)) {
        let m = mid(point, next);
        pb.quad_to(point.0, point.1, m.0, m.1);
    }
    pb.close();
    pb.finish()
}

/// A closed shape with straight sides.
fn polygon(points: &[(f32, f32)]) -> Option<Path> {
    let (first, rest) = points.split_first()?;
    let mut pb = PathBuilder::new();
    pb.move_to(first.0, first.1);
    for p in rest {
        pb.line_to(p.0, p.1);
    }
    pb.close();
    pb.finish()
}

/// A robe from the shoulders to the hem, flaring out, in one soft curve.
fn robe(
    c: &mut Canvas,
    shoulder_y: f32,
    hem_y: f32,
    shoulder_half: f32,
    hem_half: f32,
    color: Rgb,
) {
    let waist_y = f32::midpoint(shoulder_y, hem_y);
    let mut pb = PathBuilder::new();
    pb.move_to(CX - shoulder_half, shoulder_y);
    pb.quad_to(CX - hem_half - 10.0, waist_y, CX - hem_half, hem_y);
    pb.quad_to(CX, hem_y + 10.0, CX + hem_half, hem_y);
    pb.quad_to(
        CX + hem_half + 10.0,
        waist_y,
        CX + shoulder_half,
        shoulder_y,
    );
    pb.close();
    c.shape(pb.finish(), color);
}

fn feet(c: &mut Canvas, y: f32, spread: f32, size: f32) {
    c.shape(ellipse(CX - spread, y, size, size * 0.5), INK);
    c.shape(ellipse(CX + spread, y, size, size * 0.5), INK);
}

/// Two bead eyes with a glint, as every creature in the cult has them.
fn bead_eyes(c: &mut Canvas, y: f32, apart: f32, rx: f32, ry: f32) {
    for side in [-1.0, 1.0] {
        let ex = CX + side * apart;
        c.fill(ellipse(ex, y, rx, ry).as_ref(), INK, 255);
        c.fill(
            circle(ex - rx * 0.35, y - ry * 0.45, rx * 0.35).as_ref(),
            WHITE,
            255,
        );
    }
}

/// The follower: a short robe in the model's colour with the cult's eye on
/// the chest, a big round head, bead eyes, pink cheeks, ears poking out
/// from under a gold hard hat.
fn follower(c: &mut Canvas, model: Model) {
    let hue = model.hue();
    feet(c, 366.0, 30.0, 20.0);
    robe(c, 182.0, 352.0, 42.0, 66.0, hue);
    c.fill(
        rounded(CX - 58.0, 322.0, 116.0, 20.0, 8.0).as_ref(),
        CREAM,
        190,
    );
    c.fill(
        rounded(CX - 60.0, 300.0, 120.0, 9.0, 4.0).as_ref(),
        shade(hue, 0.72),
        255,
    );
    // The cult's mark: an eye on the chest.
    c.shape(ellipse(CX, 248.0, 22.0, 14.0), CREAM);
    c.fill(circle(CX, 248.0, 7.0).as_ref(), INK, 255);
    c.stroke(circle(CX, 248.0, 11.0).as_ref(), RED, 2.5);
    // The collar.
    c.shape(
        polygon(&[
            (CX - 44.0, 180.0),
            (CX + 44.0, 180.0),
            (CX + 22.0, 208.0),
            (CX - 22.0, 208.0),
        ]),
        CREAM,
    );
    // The head, the ears, the face.
    c.shape(circle(CX, 120.0, 80.0), SKIN);
    for side in [-1.0, 1.0] {
        let ex = CX + side * 68.0;
        c.shape(ellipse(ex, 100.0, 20.0, 30.0), SKIN);
        c.fill(ellipse(ex, 102.0, 9.0, 17.0).as_ref(), PINK, 255);
    }
    bead_eyes(c, 124.0, 24.0, 12.0, 16.0);
    c.fill(circle(CX - 46.0, 148.0, 10.0).as_ref(), PINK, 150);
    c.fill(circle(CX + 46.0, 148.0, 10.0).as_ref(), PINK, 150);
    let mut smile = PathBuilder::new();
    smile.move_to(CX - 10.0, 156.0);
    smile.quad_to(CX, 167.0, CX + 10.0, 156.0);
    c.stroke(smile.finish().as_ref(), INK, 5.0);
    // The hard hat, over the forehead.
    c.shape(ellipse(CX, 58.0, 74.0, 32.0), GOLD);
    c.shape(rounded(CX - 82.0, 78.0, 164.0, 18.0, 9.0), GOLD_DARK);
    c.fill(
        rounded(CX - 12.0, 32.0, 24.0, 40.0, 10.0).as_ref(),
        GOLD_DARK,
        255,
    );
}

/// The steward: a tall cream robe with a red stripe, a pointed hood over a
/// shadowed face with two red eyes, a long bone beard, a staff with a pale
/// orb.
fn steward(c: &mut Canvas) {
    feet(c, 368.0, 24.0, 18.0);
    robe(c, 122.0, 356.0, 40.0, 70.0, CREAM);
    c.fill(
        rounded(CX - 10.0, 140.0, 20.0, 200.0, 8.0).as_ref(),
        RED,
        255,
    );
    c.fill(
        rounded(CX - 66.0, 330.0, 132.0, 14.0, 6.0).as_ref(),
        RED,
        255,
    );
    c.shape(
        blob(&[
            (CX - 40.0, 118.0),
            (CX + 40.0, 118.0),
            (CX + 46.0, 150.0),
            (CX + 42.0, 172.0),
            (CX - 42.0, 172.0),
            (CX - 46.0, 150.0),
        ]),
        shade(CREAM, 0.92),
    );
    // The hood, and the face in its shadow.
    let mut hood = PathBuilder::new();
    hood.move_to(CX, 8.0);
    hood.quad_to(CX - 48.0, 40.0, CX - 54.0, 122.0);
    hood.line_to(CX + 54.0, 122.0);
    hood.quad_to(CX + 48.0, 40.0, CX, 8.0);
    hood.close();
    c.shape(hood.finish(), CREAM);
    c.fill(ellipse(CX, 98.0, 38.0, 42.0).as_ref(), SHADOW, 255);
    for side in [-1.0, 1.0] {
        let ex = CX + side * 14.0;
        c.fill(circle(ex, 96.0, 11.0).as_ref(), RED, 90);
        c.fill(ellipse(ex, 96.0, 5.0, 8.0).as_ref(), RED, 255);
    }
    // The beard.
    let mut beard = PathBuilder::new();
    beard.move_to(CX - 26.0, 120.0);
    beard.quad_to(CX - 26.0, 200.0, CX, 240.0);
    beard.quad_to(CX + 26.0, 200.0, CX + 26.0, 120.0);
    beard.close();
    c.shape(beard.finish(), (246, 241, 230));
    c.line((CX - 8.0, 150.0), (CX - 4.0, 205.0), shade(BONE, 0.9), 3.0);
    c.line((CX + 8.0, 150.0), (CX + 4.0, 205.0), shade(BONE, 0.9), 3.0);
    // The staff and the orb.
    c.line((CX + 86.0, 44.0), (CX + 80.0, 360.0), INK, 11.0);
    c.fill(circle(CX + 88.0, 36.0, 28.0).as_ref(), LAVENDER, 70);
    c.shape(circle(CX + 88.0, 36.0, 16.0), LAVENDER);
    c.fill(circle(CX + 83.0, 30.0, 5.0).as_ref(), WHITE, 255);
    c.shape(circle(CX + 78.0, 232.0, 14.0), SKIN);
}

/// A robot: a rounded head with one big eye and an antenna, a body with a
/// cream panel and a button, a tool in its hand.
fn robot(c: &mut Canvas, model: Model, wrench: bool, active: bool) {
    let hue = model.hue();
    c.shape(rounded(CX - 38.0, 342.0, 28.0, 26.0, 10.0), INK);
    c.shape(rounded(CX + 10.0, 342.0, 28.0, 26.0, 10.0), INK);
    c.shape(rounded(CX - 58.0, 196.0, 116.0, 152.0, 26.0), hue);
    c.shape(rounded(CX - 36.0, 224.0, 72.0, 54.0, 12.0), CREAM);
    c.fill(circle(CX - 24.0, 236.0, 4.0).as_ref(), INK, 255);
    c.fill(circle(CX + 24.0, 236.0, 4.0).as_ref(), INK, 255);
    let button = if active { GREEN } else { RED_DARK };
    c.shape(circle(CX, 260.0, 9.0), button);
    // The arm and the tool.
    c.shape(
        rounded(CX + 48.0, 214.0, 54.0, 22.0, 11.0),
        shade(hue, 0.78),
    );
    if wrench {
        c.shape(rounded(CX + 88.0, 160.0, 16.0, 70.0, 8.0), GREY);
        c.shape(circle(CX + 96.0, 152.0, 20.0), GREY);
        c.fill(
            rounded(CX + 88.0, 130.0, 16.0, 18.0, 3.0).as_ref(),
            INK,
            255,
        );
    } else {
        c.shape(rounded(CX + 72.0, 150.0, 46.0, 66.0, 6.0), CREAM);
        c.fill(
            rounded(CX + 86.0, 142.0, 18.0, 12.0, 4.0).as_ref(),
            GREY,
            255,
        );
        for (dy, len) in [(0.0, 30.0), (12.0, 24.0), (24.0, 28.0)] {
            c.fill(
                rounded(CX + 80.0, 168.0 + dy, len, 4.0, 2.0).as_ref(),
                INK,
                255,
            );
        }
    }
    // The head, the eye, the antenna.
    c.shape(
        rounded(CX - 66.0, 62.0, 132.0, 120.0, 40.0),
        shade(hue, 1.1),
    );
    c.shape(circle(CX, 122.0, 38.0), WHITE);
    c.fill(
        circle(CX, 122.0, 17.0).as_ref(),
        if active { GREEN } else { INK },
        255,
    );
    c.fill(circle(CX - 10.0, 110.0, 6.0).as_ref(), WHITE, 255);
    c.line((CX, 62.0), (CX, 30.0), INK, 6.0);
    if active {
        c.fill(circle(CX, 24.0, 18.0).as_ref(), GREEN, 80);
    }
    c.shape(circle(CX, 24.0, 9.0), if active { GREEN } else { RED });
}

/// Paints a figure into a fresh texture.
#[must_use]
pub fn paint(figure: Figure) -> Sprite {
    let Some(pixmap) = Pixmap::new(WIDTH, HEIGHT) else {
        // Only a zero size fails, and the sizes are constants: a blank card
        // rather than a stop, should that ever change.
        return Sprite {
            width: WIDTH,
            height: HEIGHT,
            rgba: vec![0; (WIDTH * HEIGHT * 4) as usize],
        };
    };
    let mut canvas = Canvas { pixmap };
    match figure {
        Figure::Follower { model } => follower(&mut canvas, model),
        Figure::Steward => steward(&mut canvas),
        Figure::Robot {
            model,
            wrench,
            active,
        } => robot(&mut canvas, model, wrench, active),
    }
    // tiny-skia keeps premultiplied pixels; the GPU wants straight alpha.
    let rgba = canvas
        .pixmap
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    Sprite {
        width: WIDTH,
        height: HEIGHT,
        rgba,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(s: &Sprite, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * s.width + x) * 4) as usize;
        [s.rgba[at], s.rgba[at + 1], s.rgba[at + 2], s.rgba[at + 3]]
    }

    fn opaque(s: &Sprite) -> usize {
        s.rgba.chunks(4).filter(|p| p[3] > 200).count()
    }

    fn inked(s: &Sprite) -> usize {
        s.rgba
            .chunks(4)
            .filter(|p| p[3] > 200 && p[0] < 70 && p[1] < 60 && p[2] < 80)
            .count()
    }

    #[test]
    fn every_figure_fills_its_texture_with_a_transparent_margin_and_ink() {
        for figure in [
            Figure::Follower { model: Model::Opus },
            Figure::Steward,
            Figure::Robot {
                model: Model::Sonnet,
                wrench: true,
                active: true,
            },
            Figure::Robot {
                model: Model::Haiku,
                wrench: false,
                active: false,
            },
        ] {
            let s = paint(figure);
            assert_eq!((s.width, s.height), (WIDTH, HEIGHT));
            assert_eq!(s.rgba.len(), (WIDTH * HEIGHT * 4) as usize);
            assert_eq!(pixel(&s, 0, 0)[3], 0, "{figure:?}: the corner is empty");
            assert_eq!(
                pixel(&s, WIDTH - 1, HEIGHT - 1)[3],
                0,
                "{figure:?}: the corner is empty"
            );
            let total = (WIDTH * HEIGHT) as usize;
            let painted = opaque(&s);
            assert!(
                painted > total / 6,
                "{figure:?}: a figure, not a speck ({painted})"
            );
            assert!(
                painted < total * 3 / 4,
                "{figure:?}: a figure, not a wall ({painted})"
            );
            assert!(
                inked(&s) > total / 60,
                "{figure:?}: thick ink lines ({})",
                inked(&s)
            );
        }
    }

    #[test]
    fn a_follower_wears_the_colour_of_their_model() {
        let opus = paint(Figure::Follower { model: Model::Opus });
        let sonnet = paint(Figure::Follower {
            model: Model::Sonnet,
        });
        // The robe, below the collar and above the hem band.
        let p = pixel(&opus, WIDTH / 2 - 30, 270);
        let q = pixel(&sonnet, WIDTH / 2 - 30, 270);
        assert_eq!(&p[..3], &[155, 111, 214], "opus robe {p:?}");
        assert_eq!(&q[..3], &[79, 143, 214], "sonnet robe {q:?}");
        assert_ne!(opus, sonnet);
    }

    #[test]
    fn a_working_robot_s_eye_glows_green_and_an_idle_one_s_is_ink() {
        let busy = paint(Figure::Robot {
            model: Model::Opus,
            wrench: true,
            active: true,
        });
        let idle = paint(Figure::Robot {
            model: Model::Opus,
            wrench: true,
            active: false,
        });
        let eye = pixel(&busy, WIDTH / 2 + 4, 126);
        assert_eq!(&eye[..3], &[126, 224, 129], "{eye:?}");
        let dark = pixel(&idle, WIDTH / 2 + 4, 126);
        assert_eq!(&dark[..3], &[42, 27, 46], "{dark:?}");
        assert_ne!(busy, idle);
        let clipboard = paint(Figure::Robot {
            model: Model::Opus,
            wrench: false,
            active: true,
        });
        assert_ne!(busy, clipboard, "the tool differs");
    }

    #[test]
    fn the_steward_has_red_eyes_in_a_shadowed_hood() {
        let s = paint(Figure::Steward);
        let eye = pixel(&s, WIDTH / 2 - 14, 96);
        assert_eq!(&eye[..3], &[198, 57, 47], "{eye:?}");
        let shadow = pixel(&s, WIDTH / 2, 110);
        assert_eq!(&shadow[..3], &[58, 42, 62], "{shadow:?}");
    }

    #[test]
    fn models_parse_from_the_picture_s_names() {
        assert_eq!(Model::parse(Some("opus")), Model::Opus);
        assert_eq!(Model::parse(Some("sonnet")), Model::Sonnet);
        assert_eq!(Model::parse(Some("haiku")), Model::Haiku);
        assert_eq!(Model::parse(Some("gpt")), Model::Unknown);
        assert_eq!(Model::parse(None), Model::Unknown);
    }
}
