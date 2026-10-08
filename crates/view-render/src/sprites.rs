//! The figures, painted as flat 2D cut-outs.
//!
//! Followers, robots and the steward stand in the world the way *Cult of the
//! Lamb* and *Don't Starve* stand paper characters — big heads, bead eyes,
//! thick ink around everything — dressed for the atomic age of *Fallout*:
//! vault jumpsuits with gold trim, Protectron robots with a glowing visor,
//! and the Vault Boy himself as the steward, grin, quiff and thumbs-up.
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

/// The model a figure works on — what colours its suit.
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
    /// A follower at work: jumpsuit in the model's colour, domed hard hat.
    Follower {
        /// The model they work on.
        model: Model,
    },
    /// The steward: the Vault Boy, grinning, thumb up.
    Steward,
    /// A Protectron; `wrench` builds, otherwise it holds a clipboard.
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
const SKIN: Rgb = (241, 220, 192);
const GOLD: Rgb = (226, 178, 64);
const GOLD_DARK: Rgb = (184, 138, 40);
const RED: Rgb = (198, 57, 47);
const RED_DARK: Rgb = (150, 42, 42);
const PINK: Rgb = (230, 138, 176);
const WHITE: Rgb = (252, 250, 244);
const GREEN: Rgb = (126, 224, 129);
const GREY: Rgb = (154, 154, 166);

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

/// The follower: a vault jumpsuit in the model's colour with gold belt and
/// trim, a big round head, bead eyes, pink cheeks, ears poking out from
/// under a domed gold hard hat with its lamp.
fn follower(c: &mut Canvas, model: Model) {
    let hue = model.hue();
    feet(c, 366.0, 30.0, 20.0);
    // Trousers, then the jumpsuit's torso: one piece, the model's colour.
    c.shape(
        rounded(CX - 44.0, 268.0, 36.0, 96.0, 14.0),
        shade(hue, 0.85),
    );
    c.shape(rounded(CX + 8.0, 268.0, 36.0, 96.0, 14.0), shade(hue, 0.85));
    c.shape(rounded(CX - 50.0, 176.0, 100.0, 104.0, 22.0), hue);
    // The belt and the gold trim every vault suit wears.
    c.shape(rounded(CX - 52.0, 258.0, 104.0, 16.0, 6.0), GOLD);
    c.fill(rounded(CX - 8.0, 260.0, 16.0, 12.0, 3.0).as_ref(), INK, 255);
    c.line((CX - 36.0, 180.0), (CX - 36.0, 256.0), GOLD, 5.0);
    c.line((CX + 36.0, 180.0), (CX + 36.0, 256.0), GOLD, 5.0);
    // The collar, in a V.
    c.shape(
        polygon(&[(CX - 40.0, 176.0), (CX + 40.0, 176.0), (CX, 212.0)]),
        GOLD,
    );
    // Arms by the sides, gloved hands.
    c.shape(rounded(CX - 72.0, 190.0, 22.0, 78.0, 11.0), hue);
    c.shape(rounded(CX + 50.0, 190.0, 22.0, 78.0, 11.0), hue);
    c.shape(circle(CX - 61.0, 272.0, 12.0), SKIN);
    c.shape(circle(CX + 61.0, 272.0, 12.0), SKIN);
    // The head, the ears, the face.
    c.shape(circle(CX, 120.0, 78.0), SKIN);
    for side in [-1.0, 1.0] {
        let ex = CX + side * 66.0;
        c.shape(ellipse(ex, 104.0, 18.0, 26.0), SKIN);
        c.fill(ellipse(ex, 106.0, 8.0, 14.0).as_ref(), PINK, 255);
    }
    bead_eyes(c, 124.0, 24.0, 12.0, 16.0);
    c.fill(circle(CX - 46.0, 148.0, 10.0).as_ref(), PINK, 150);
    c.fill(circle(CX + 46.0, 148.0, 10.0).as_ref(), PINK, 150);
    let mut smile = PathBuilder::new();
    smile.move_to(CX - 12.0, 156.0);
    smile.quad_to(CX, 170.0, CX + 12.0, 156.0);
    c.stroke(smile.finish().as_ref(), INK, 5.0);
    // A domed hard hat with its lamp — the atomic age's helmet.
    c.shape(ellipse(CX, 60.0, 76.0, 36.0), GOLD);
    c.shape(rounded(CX - 86.0, 80.0, 172.0, 16.0, 8.0), GOLD_DARK);
    c.shape(rounded(CX - 14.0, 34.0, 28.0, 22.0, 8.0), GOLD_DARK);
    c.fill(circle(CX, 45.0, 6.0).as_ref(), WHITE, 255);
}

/// The steward: the Vault Boy — a grinning mascot of the atomic age in a
/// blue jumpsuit with gold trim, a swept blond quiff with its curl, a wink
/// and a thumbs-up.
fn steward(c: &mut Canvas) {
    let suit = (79, 143, 214);
    feet(c, 366.0, 26.0, 18.0);
    // Trousers and torso: the jumpsuit, gold belt, gold collar.
    c.shape(
        rounded(CX - 40.0, 266.0, 34.0, 98.0, 14.0),
        shade(suit, 0.85),
    );
    c.shape(
        rounded(CX + 6.0, 266.0, 34.0, 98.0, 14.0),
        shade(suit, 0.85),
    );
    c.shape(rounded(CX - 48.0, 178.0, 96.0, 102.0, 22.0), suit);
    c.shape(rounded(CX - 50.0, 256.0, 100.0, 16.0, 6.0), GOLD);
    c.fill(rounded(CX - 8.0, 258.0, 16.0, 12.0, 3.0).as_ref(), INK, 255);
    c.line((CX - 34.0, 182.0), (CX - 34.0, 254.0), GOLD, 5.0);
    c.line((CX + 34.0, 182.0), (CX + 34.0, 254.0), GOLD, 5.0);
    c.shape(
        polygon(&[(CX - 38.0, 178.0), (CX + 38.0, 178.0), (CX, 212.0)]),
        GOLD,
    );
    // Left arm by the side; right arm up, fist closed, thumb up.
    c.shape(rounded(CX - 70.0, 190.0, 22.0, 76.0, 11.0), suit);
    c.shape(circle(CX - 59.0, 270.0, 12.0), SKIN);
    let mut arm = PathBuilder::new();
    arm.move_to(CX + 44.0, 200.0);
    arm.quad_to(CX + 84.0, 196.0, CX + 86.0, 150.0);
    arm.line_to(CX + 108.0, 154.0);
    arm.quad_to(CX + 104.0, 212.0, CX + 48.0, 222.0);
    arm.close();
    c.shape(arm.finish(), suit);
    c.shape(rounded(CX + 80.0, 118.0, 34.0, 36.0, 12.0), SKIN);
    c.shape(rounded(CX + 90.0, 92.0, 14.0, 32.0, 7.0), SKIN);
    // The head: round, big, with ears and a wink.
    c.shape(circle(CX, 112.0, 70.0), SKIN);
    for side in [-1.0, 1.0] {
        c.shape(ellipse(CX + side * 62.0, 112.0, 14.0, 20.0), SKIN);
    }
    // Open eye on the left, winking eye on the right, both browed.
    c.shape(ellipse(CX - 22.0, 108.0, 14.0, 17.0), WHITE);
    c.fill(circle(CX - 19.0, 110.0, 7.0).as_ref(), INK, 255);
    c.line((CX + 10.0, 110.0), (CX + 36.0, 110.0), INK, 5.0);
    c.line((CX - 36.0, 86.0), (CX - 8.0, 84.0), INK, 5.0);
    c.line((CX + 10.0, 84.0), (CX + 38.0, 90.0), INK, 5.0);
    // The grin: a wide white mouth with a line of teeth.
    let mut grin = PathBuilder::new();
    grin.move_to(CX - 36.0, 134.0);
    grin.quad_to(CX, 178.0, CX + 36.0, 134.0);
    grin.close();
    c.shape(grin.finish(), WHITE);
    c.line((CX - 30.0, 148.0), (CX + 30.0, 148.0), INK, 2.5);
    c.fill(circle(CX - 50.0, 138.0, 9.0).as_ref(), PINK, 150);
    c.fill(circle(CX + 50.0, 138.0, 9.0).as_ref(), PINK, 150);
    // The quiff: a cap of gold hair swept to the right, and its curl.
    let mut hair = PathBuilder::new();
    hair.move_to(CX - 70.0, 92.0);
    hair.quad_to(CX - 70.0, 36.0, CX - 10.0, 34.0);
    hair.quad_to(CX + 40.0, 30.0, CX + 66.0, 70.0);
    hair.quad_to(CX + 30.0, 56.0, CX, 66.0);
    hair.quad_to(CX - 30.0, 72.0, CX - 70.0, 92.0);
    hair.close();
    c.shape(hair.finish(), GOLD);
    let mut curl = PathBuilder::new();
    curl.move_to(CX + 30.0, 40.0);
    curl.quad_to(CX + 70.0, 10.0, CX + 86.0, 44.0);
    curl.quad_to(CX + 70.0, 36.0, CX + 56.0, 50.0);
    curl.close();
    c.shape(curl.finish(), GOLD);
}

/// A robot: a Protectron of the atomic age — a domed head over a glowing
/// visor, a riveted chest plate, stubby chrome arms, a tool in the right
/// one.
fn robot(c: &mut Canvas, model: Model, wrench: bool, active: bool) {
    let hue = model.hue();
    let glow = if active { GREEN } else { INK };
    // Feet and chrome legs.
    c.shape(rounded(CX - 44.0, 340.0, 34.0, 28.0, 10.0), INK);
    c.shape(rounded(CX + 10.0, 340.0, 34.0, 28.0, 10.0), INK);
    c.shape(rounded(CX - 38.0, 290.0, 24.0, 56.0, 10.0), GREY);
    c.shape(rounded(CX + 14.0, 290.0, 24.0, 56.0, 10.0), GREY);
    // The body: a rounded hull in the model's colour, a cream chest plate
    // with four rivets and a status lamp.
    c.shape(rounded(CX - 60.0, 180.0, 120.0, 120.0, 30.0), hue);
    c.shape(rounded(CX - 40.0, 200.0, 80.0, 64.0, 10.0), CREAM);
    for (dx, dy) in [(-30.0, 208.0), (30.0, 208.0), (-30.0, 254.0), (30.0, 254.0)] {
        c.fill(circle(CX + dx, dy, 4.0).as_ref(), INK, 255);
    }
    c.shape(
        circle(CX, 232.0, 10.0),
        if active { GREEN } else { RED_DARK },
    );
    c.line((CX - 24.0, 280.0), (CX + 24.0, 280.0), shade(hue, 0.7), 6.0);
    // Chrome arms: the left down, the right out with the tool.
    c.shape(rounded(CX - 86.0, 196.0, 24.0, 86.0, 12.0), GREY);
    c.shape(circle(CX - 74.0, 290.0, 13.0), shade(GREY, 0.8));
    c.shape(rounded(CX + 56.0, 196.0, 56.0, 24.0, 12.0), GREY);
    if wrench {
        c.shape(
            rounded(CX + 96.0, 150.0, 16.0, 70.0, 8.0),
            shade(GREY, 1.15),
        );
        c.shape(circle(CX + 104.0, 142.0, 20.0), shade(GREY, 1.15));
        c.fill(
            rounded(CX + 96.0, 120.0, 16.0, 18.0, 3.0).as_ref(),
            INK,
            255,
        );
    } else {
        c.shape(rounded(CX + 80.0, 140.0, 46.0, 66.0, 6.0), CREAM);
        c.fill(
            rounded(CX + 94.0, 132.0, 18.0, 12.0, 4.0).as_ref(),
            GREY,
            255,
        );
        for (dy, len) in [(0.0, 30.0), (12.0, 24.0), (24.0, 28.0)] {
            c.fill(
                rounded(CX + 88.0, 158.0 + dy, len, 4.0, 2.0).as_ref(),
                INK,
                255,
            );
        }
    }
    // The neck, the domed head, the visor with its two glowing slits.
    c.shape(rounded(CX - 22.0, 160.0, 44.0, 24.0, 8.0), GREY);
    c.shape(ellipse(CX, 110.0, 64.0, 56.0), shade(hue, 1.1));
    c.shape(rounded(CX - 48.0, 104.0, 96.0, 28.0, 10.0), INK);
    c.fill(
        rounded(CX - 36.0, 112.0, 28.0, 12.0, 5.0).as_ref(),
        glow,
        255,
    );
    c.fill(
        rounded(CX + 8.0, 112.0, 28.0, 12.0, 5.0).as_ref(),
        glow,
        255,
    );
    if active {
        c.fill(
            rounded(CX - 40.0, 106.0, 80.0, 24.0, 8.0).as_ref(),
            GREEN,
            60,
        );
    }
    // The antenna and its bulb.
    c.line((CX + 30.0, 62.0), (CX + 40.0, 26.0), INK, 6.0);
    if active {
        c.fill(circle(CX + 42.0, 20.0, 18.0).as_ref(), GREEN, 80);
    }
    c.shape(
        circle(CX + 42.0, 20.0, 9.0),
        if active { GREEN } else { RED },
    );
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
        // The jumpsuit's torso, between the collar and the belt.
        let p = pixel(&opus, WIDTH / 2 - 20, 240);
        let q = pixel(&sonnet, WIDTH / 2 - 20, 240);
        assert_eq!(&p[..3], &[155, 111, 214], "opus suit {p:?}");
        assert_eq!(&q[..3], &[79, 143, 214], "sonnet suit {q:?}");
        assert_ne!(opus, sonnet);
    }

    #[test]
    fn a_working_robot_s_visor_glows_green_and_an_idle_one_s_is_ink() {
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
        let slit = pixel(&busy, WIDTH / 2 - 22, 118);
        assert_eq!(&slit[..3], &[126, 224, 129], "{slit:?}");
        let dark = pixel(&idle, WIDTH / 2 - 22, 118);
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
    fn the_steward_is_the_vault_boy_gold_quiff_over_a_blue_suit() {
        let s = paint(Figure::Steward);
        let hair = pixel(&s, WIDTH / 2 - 20, 50);
        assert_eq!(&hair[..3], &[226, 178, 64], "{hair:?}");
        let suit = pixel(&s, WIDTH / 2 - 10, 232);
        assert_eq!(&suit[..3], &[79, 143, 214], "{suit:?}");
        let grin = pixel(&s, WIDTH / 2, 140);
        assert_eq!(&grin[..3], &[252, 250, 244], "a white grin {grin:?}");
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
