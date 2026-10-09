//! The figures, painted as flat 2D cut-outs.
//!
//! Followers, robots and the steward stand in the world the way *Cult of the
//! Lamb* and *Don't Starve* stand paper characters — big heads, bead eyes,
//! thick ink around everything — dressed for the atomic age of *Fallout*:
//! vault jumpsuits with gold trim, *Futurama*'s robots on the line —
//! Bender in steel grey builds, Hedonismbot in gold inspects from his
//! chaise — and the Vault Boy himself as the steward, grin, quiff and
//! thumbs-up.
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
    /// The elder: an old scientist in a lab coat, who stands for a crew of
    /// several agents at one station.
    Elder,
    /// A robot: with `wrench` it is Bender, the steel-grey builder;
    /// otherwise Hedonismbot, the gold inspector on his chaise, clipboard
    /// in hand.
    Robot {
        /// The model it runs, worn as a badge.
        model: Model,
        /// Builder or inspector.
        wrench: bool,
        /// Its eyes light up (or open) while it works.
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
const PINK: Rgb = (230, 138, 176);
const WHITE: Rgb = (252, 250, 244);
const GREEN: Rgb = (126, 224, 129);
const GREY: Rgb = (154, 154, 166);
// Bender's steel, Hedonismbot's eyes, grille and laurels, a cigar.
const STEEL: Rgb = (168, 182, 198);
const STEEL_DARK: Rgb = (132, 146, 164);
const YELLOW: Rgb = (250, 222, 72);
const BLUE_EYE: Rgb = (118, 160, 224);
const GRILL: Rgb = (96, 104, 196);
const LEAF: Rgb = (104, 160, 72);
const CIGAR: Rgb = (118, 72, 40);

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

/// The elder: a stooped old scientist — bald and spotted, white tufts over
/// the ears, big round tinted spectacles, a long nose over a frown, a white
/// lab coat open on green scrubs, slippers. The figure of a crew: several
/// agents at one station, one figure.
fn elder(c: &mut Canvas) {
    const SCRUBS: Rgb = (104, 178, 146);
    const SLIPPERS: Rgb = (170, 222, 214);
    const LENS: Rgb = (196, 236, 228);
    // Slippers, then thin green legs under the coat.
    c.shape(ellipse(CX - 22.0, 368.0, 24.0, 11.0), SLIPPERS);
    c.shape(ellipse(CX + 22.0, 368.0, 24.0, 11.0), SLIPPERS);
    c.shape(
        rounded(CX - 30.0, 282.0, 22.0, 84.0, 10.0),
        shade(SCRUBS, 0.85),
    );
    c.shape(
        rounded(CX + 8.0, 282.0, 22.0, 84.0, 10.0),
        shade(SCRUBS, 0.85),
    );
    // The scrubs' top, seen between the coat's open sides.
    c.shape(rounded(CX - 34.0, 176.0, 68.0, 112.0, 16.0), SCRUBS);
    c.shape(
        polygon(&[(CX - 18.0, 176.0), (CX + 18.0, 176.0), (CX, 200.0)]),
        shade(SKIN, 0.95),
    );
    // The lab coat: two long panels down to the knees, lapels folded back.
    c.shape(
        polygon(&[
            (CX - 30.0, 172.0),
            (CX - 58.0, 186.0),
            (CX - 64.0, 318.0),
            (CX - 22.0, 322.0),
            (CX - 14.0, 216.0),
        ]),
        WHITE,
    );
    c.shape(
        polygon(&[
            (CX + 30.0, 172.0),
            (CX + 58.0, 186.0),
            (CX + 64.0, 318.0),
            (CX + 22.0, 322.0),
            (CX + 14.0, 216.0),
        ]),
        WHITE,
    );
    c.shape(
        polygon(&[(CX - 30.0, 172.0), (CX - 12.0, 214.0), (CX - 34.0, 200.0)]),
        shade(WHITE, 0.9),
    );
    c.shape(
        polygon(&[(CX + 30.0, 172.0), (CX + 12.0, 214.0), (CX + 34.0, 200.0)]),
        shade(WHITE, 0.9),
    );
    // Sleeves hanging, bony hands.
    c.shape(rounded(CX - 78.0, 188.0, 24.0, 104.0, 11.0), WHITE);
    c.shape(rounded(CX + 54.0, 188.0, 24.0, 104.0, 11.0), WHITE);
    c.shape(ellipse(CX - 66.0, 298.0, 10.0, 14.0), SKIN);
    c.shape(ellipse(CX + 66.0, 298.0, 10.0, 14.0), SKIN);
    // A thin neck, then the head: tall, bald, a few spots on the crown.
    c.shape(rounded(CX - 12.0, 150.0, 24.0, 30.0, 8.0), SKIN);
    for side in [-1.0, 1.0] {
        c.shape(ellipse(CX + side * 54.0, 112.0, 13.0, 20.0), SKIN);
    }
    c.shape(ellipse(CX, 96.0, 54.0, 66.0), SKIN);
    let spot = shade(SKIN, 0.82);
    c.fill(ellipse(CX - 16.0, 50.0, 7.0, 5.0).as_ref(), spot, 255);
    c.fill(ellipse(CX + 12.0, 44.0, 5.0, 4.0).as_ref(), spot, 255);
    c.fill(ellipse(CX + 26.0, 62.0, 4.0, 3.0).as_ref(), spot, 255);
    // White tufts over the ears.
    for side in [-1.0, 1.0] {
        c.shape(ellipse(CX + side * 50.0, 84.0, 9.0, 13.0), WHITE);
    }
    // Big round tinted spectacles.
    for side in [-1.0, 1.0] {
        let lens = circle(CX + side * 22.0, 100.0, 19.0);
        c.fill(lens.as_ref(), LENS, 230);
        c.stroke(lens.as_ref(), INK, LINE);
        c.fill(
            circle(CX + side * 22.0 - 6.0, 94.0, 4.0).as_ref(),
            WHITE,
            255,
        );
    }
    c.line((CX - 4.0, 98.0), (CX + 4.0, 98.0), INK, 5.0);
    // A long nose, and a frown under it.
    c.shape(ellipse(CX + 2.0, 126.0, 9.0, 15.0), shade(SKIN, 0.93));
    let mut frown = PathBuilder::new();
    frown.move_to(CX - 14.0, 152.0);
    frown.quad_to(CX, 142.0, CX + 14.0, 152.0);
    c.stroke(frown.finish().as_ref(), INK, 5.0);
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

/// A box with a domed top and rounded bottom corners: a robot's head, a
/// bell foot.
#[allow(clippy::many_single_char_names)]
fn domed(x: f32, y: f32, w: f32, h: f32, dome: f32, r: f32) -> Option<Path> {
    let mut pb = PathBuilder::new();
    pb.move_to(x, y + dome);
    pb.quad_to(x, y, x + w / 2.0, y);
    pb.quad_to(x + w, y, x + w, y + dome);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.close();
    pb.finish()
}

/// The seams of a jointed limb: thin ink rings across it.
fn seams(c: &mut Canvas, x: f32, w: f32, ys: &[f32]) {
    for &y in ys {
        c.line((x, y), (x + w, y), INK, 3.0);
    }
}

/// Bender, the builder: a steel-grey bending unit — a domed head with a
/// black visor and two yellow eyes that light up while he works, a toothy
/// grin, an antenna with its bulb, a door on his chest, a cigar in one hand
/// and a wrench raised in the other. The model shows as the badge on his
/// door.
fn bender(c: &mut Canvas, model: Model, active: bool) {
    let eyes = if active { YELLOW } else { INK };
    // The antenna and its bulb, before the head covers its base.
    c.line((CX, 56.0), (CX, 22.0), INK, 6.0);
    if active {
        c.fill(circle(CX, 16.0, 16.0).as_ref(), GREEN, 80);
    }
    c.shape(circle(CX, 16.0, 8.0), if active { GREEN } else { RED });
    // Jointed legs and feet; the body goes over their tops.
    for side in [-1.0, 1.0] {
        let x = CX + side * 26.0 - 11.0;
        c.shape(rounded(x, 290.0, 22.0, 56.0, 10.0), STEEL_DARK);
        seams(c, x, 22.0, &[306.0, 320.0, 334.0]);
        c.shape(rounded(x - 10.0, 340.0, 42.0, 26.0, 12.0), STEEL_DARK);
    }
    // The body: a steel hull with a door, its handle and the badge.
    c.shape(rounded(CX - 56.0, 166.0, 112.0, 132.0, 24.0), STEEL);
    c.shape(
        domed(CX - 32.0, 190.0, 64.0, 78.0, 20.0, 8.0),
        shade(STEEL, 0.92),
    );
    c.fill(circle(CX + 20.0, 236.0, 4.0).as_ref(), INK, 255);
    c.shape(circle(CX - 14.0, 212.0, 8.0), model.hue());
    // The left arm hangs, cigar lit and smoking.
    c.shape(rounded(CX - 86.0, 176.0, 22.0, 92.0, 11.0), STEEL_DARK);
    seams(c, CX - 86.0, 22.0, &[200.0, 222.0, 244.0]);
    c.shape(circle(CX - 75.0, 274.0, 13.0), STEEL_DARK);
    c.line((CX - 80.0, 268.0), (CX - 108.0, 246.0), INK, 15.0);
    c.line((CX - 80.0, 268.0), (CX - 108.0, 246.0), CIGAR, 9.0);
    c.fill(circle(CX - 109.0, 245.0, 5.0).as_ref(), RED, 255);
    for (dx, dy, r) in [
        (-114.0, 228.0, 7.0),
        (-118.0, 212.0, 9.0),
        (-110.0, 194.0, 11.0),
    ] {
        c.fill(circle(CX + dx, dy, r).as_ref(), WHITE, 110);
    }
    // The right arm raises the wrench.
    c.shape(rounded(CX + 64.0, 150.0, 22.0, 90.0, 11.0), STEEL_DARK);
    seams(c, CX + 64.0, 22.0, &[176.0, 198.0, 220.0]);
    c.shape(rounded(CX + 68.0, 72.0, 14.0, 76.0, 6.0), shade(GREY, 1.15));
    c.shape(circle(CX + 75.0, 66.0, 18.0), shade(GREY, 1.15));
    c.fill(rounded(CX + 68.0, 46.0, 14.0, 18.0, 3.0).as_ref(), INK, 255);
    c.shape(circle(CX + 75.0, 146.0, 13.0), STEEL_DARK);
    // The neck, the domed head, the visor, the eyes, the grin.
    c.shape(rounded(CX - 16.0, 150.0, 32.0, 20.0, 6.0), STEEL_DARK);
    c.shape(domed(CX - 40.0, 44.0, 80.0, 112.0, 26.0, 10.0), STEEL);
    c.shape(rounded(CX - 46.0, 84.0, 92.0, 44.0, 16.0), INK);
    for side in [-1.0, 1.0] {
        let ex = CX + side * 19.0;
        c.fill(circle(ex, 106.0, 14.0).as_ref(), eyes, 255);
        if active {
            c.fill(circle(ex, 106.0, 5.0).as_ref(), INK, 255);
        }
    }
    c.shape(rounded(CX - 30.0, 128.0, 60.0, 18.0, 5.0), CREAM);
    for dx in [-20.0, -10.0, 0.0, 10.0, 20.0] {
        c.line((CX + dx, 130.0), (CX + dx, 144.0), INK, 2.5);
    }
}

/// Hedonismbot, the inspector: a gold robot sprawled on a gold chaise
/// longue — laurels on his head, blue eyes under lids that lift while he
/// works, a blue grille for a mouth, a great riveted belly, one arm behind
/// his head and the clipboard held up in the other. The model shows as
/// the clip on his board.
fn hedonismbot(c: &mut Canvas, model: Model, active: bool) {
    // The chaise: a scrolled back on the left, two feet, the seat, the scroll.
    c.shape(rounded(14.0, 196.0, 52.0, 150.0, 22.0), GOLD_DARK);
    for x in [24.0, 60.0] {
        c.shape(rounded(x, 350.0, 16.0, 22.0, 6.0), GOLD_DARK);
    }
    // The arm behind the head, before the head covers it.
    c.shape(rounded(26.0, 120.0, 20.0, 90.0, 10.0), GOLD_DARK);
    seams(c, 26.0, 20.0, &[146.0, 170.0, 194.0]);
    c.shape(circle(44.0, 116.0, 12.0), GOLD_DARK);
    c.shape(rounded(14.0, 296.0, 228.0, 60.0, 20.0), GOLD_DARK);
    c.shape(circle(226.0, 318.0, 16.0), GOLD);
    c.fill(circle(226.0, 318.0, 6.0).as_ref(), INK, 255);
    // The belly, seamed and riveted, on the seat.
    c.shape(ellipse(150.0, 262.0, 80.0, 58.0), GOLD);
    c.line((80.0, 258.0), (222.0, 258.0), GOLD_DARK, 5.0);
    for x in [110.0, 150.0, 190.0] {
        c.fill(circle(x, 272.0, 4.0).as_ref(), INK, 255);
    }
    // The legs hang off the seat, bell feet on the ground.
    for x in [150.0, 196.0] {
        c.shape(rounded(x, 300.0, 20.0, 52.0, 9.0), GOLD_DARK);
        seams(c, x, 20.0, &[318.0, 334.0]);
        c.shape(domed(x - 10.0, 346.0, 40.0, 28.0, 14.0, 8.0), GOLD);
    }
    // The head leans on the chaise's back, laurelled.
    c.shape(domed(56.0, 140.0, 72.0, 112.0, 30.0, 14.0), GOLD);
    for (x, y) in [
        (62.0, 152.0),
        (74.0, 141.0),
        (90.0, 136.0),
        (106.0, 141.0),
        (118.0, 152.0),
    ] {
        c.shape(ellipse(x, y, 9.0, 6.0), LEAF);
    }
    // Blue eyes under lids that droop at rest and lift at work.
    let lid = if active { 180.0 } else { 190.0 };
    for ex in [78.0, 106.0] {
        c.fill(ellipse(ex, 186.0, 12.0, 9.0).as_ref(), BLUE_EYE, 255);
        c.fill(circle(ex, 188.0, 4.0).as_ref(), INK, 255);
        c.fill(
            rounded(ex - 13.0, 174.0, 26.0, lid - 174.0, 4.0).as_ref(),
            GOLD,
            255,
        );
        c.line((ex - 12.0, lid), (ex + 12.0, lid), INK, 3.0);
    }
    // The grille.
    c.shape(rounded(66.0, 208.0, 52.0, 24.0, 6.0), GRILL);
    for dx in [10.0, 20.0, 30.0, 40.0] {
        c.line((66.0 + dx, 211.0), (66.0 + dx, 229.0), INK, 2.5);
    }
    // The other arm holds the clipboard up; its clip is the model's colour.
    c.shape(rounded(176.0, 140.0, 20.0, 96.0, 10.0), GOLD_DARK);
    seams(c, 176.0, 20.0, &[166.0, 190.0, 214.0]);
    c.shape(circle(186.0, 136.0, 12.0), GOLD_DARK);
    c.shape(rounded(166.0, 60.0, 46.0, 70.0, 6.0), CREAM);
    c.fill(
        rounded(180.0, 52.0, 18.0, 12.0, 4.0).as_ref(),
        model.hue(),
        255,
    );
    for (dy, len) in [(0.0, 30.0), (12.0, 24.0), (24.0, 28.0)] {
        c.fill(rounded(174.0, 78.0 + dy, len, 4.0, 2.0).as_ref(), INK, 255);
    }
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
        Figure::Elder => elder(&mut canvas),
        Figure::Robot {
            model,
            wrench,
            active,
        } if wrench => bender(&mut canvas, model, active),
        Figure::Robot { model, active, .. } => hedonismbot(&mut canvas, model, active),
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
            Figure::Elder,
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
    fn bender_s_eyes_light_yellow_at_work_and_go_dark_idle() {
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
        let eye = pixel(&busy, WIDTH / 2 - 10, 106);
        assert_eq!(&eye[..3], &[250, 222, 72], "{eye:?}");
        let dark = pixel(&idle, WIDTH / 2 - 10, 106);
        assert_eq!(&dark[..3], &[42, 27, 46], "{dark:?}");
        assert_ne!(busy, idle);
        let hull = pixel(&busy, WIDTH / 2 + 46, 240);
        assert_eq!(&hull[..3], &[168, 182, 198], "steel grey: {hull:?}");
    }

    #[test]
    fn hedonismbot_is_gold_and_opens_his_eyes_at_work() {
        let busy = paint(Figure::Robot {
            model: Model::Opus,
            wrench: false,
            active: true,
        });
        let idle = paint(Figure::Robot {
            model: Model::Opus,
            wrench: false,
            active: false,
        });
        let eye = pixel(&busy, 85, 186);
        assert_eq!(&eye[..3], &[118, 160, 224], "{eye:?}");
        let lid = pixel(&idle, 85, 186);
        assert_eq!(&lid[..3], &[226, 178, 64], "{lid:?}");
        let belly = pixel(&busy, 150, 290);
        assert_eq!(&belly[..3], &[226, 178, 64], "gold: {belly:?}");
        let bender = paint(Figure::Robot {
            model: Model::Opus,
            wrench: true,
            active: true,
        });
        assert_ne!(busy, bender, "the builder and the inspector differ");
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
