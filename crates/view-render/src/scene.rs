//! From a picture and a viewpoint to a list of things to draw.
//!
//! Pure, and tested without a GPU: a [`Scene`] is props (rounded boxes,
//! capsules, cones, spheres, arches), labels anchored in the world, and a
//! bounding box the camera frames. The props carry an optional [`Hot`], which
//! is what a click or a hover reports to the page; they carry an [`Anim`]
//! when they move.
//!
//! The look is *Cult of the Lamb*'s palette and ink on *Fallout*'s atomic
//! age: nothing has a sharp corner, every shape wears a thick ink outline
//! (drawn by [`crate::app`]), the light falls in three flat bands, the
//! palette is brick red, slate, cream, pavement and gold under a dark plum ink
//! — and the props are chrome and portholes, a vault's gear door, CRT
//! terminals, radar dishes and red beacons. The plant itself is *Futurama*'s
//! Planet Express: a brick hangar beside a tall tower under a red dome, in a
//! town by the water. The figures — followers, robots, the steward, the doctor —
//! are not built from shapes but painted: flat cut-outs ([`crate::sprites`])
//! standing on the ground and always facing the camera, as a paper doll
//! would.
//!
//! The coordinates are the plant's: `x` along the belts, `z` across them,
//! `y` up — one tile is one unit, one storey is one unit. The camera in
//! [`crate::app`] looks at that from the classic isometric corner.

// Layout arithmetic — `x0 + i as f32 * 1.5` — reads as placement; a fused
// multiply-add would gain nothing on a scene built a few times a minute.
#![allow(clippy::suboptimal_flops)]
// Props are placed by index — `i as f32 * 1.5` — and indices here are a few
// dozen at most, far below where an `f32` loses an integer.
#![allow(clippy::cast_precision_loss)]
// `x`, `y`, `z`, `w`, `h`, `d`, `r`: the names a shape has had since geometry.
#![allow(clippy::many_single_char_names)]

use crate::bridge::{Hot, View};
use crate::model::{Employee, Issue, IssueStatus, Kind, Room, RoomStatus, Snapshot, StationState};
use crate::sprites::{Figure, Model};

/// A colour, in sRGB with alpha.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rgba(pub f32, pub f32, pub f32, pub f32);

impl Rgba {
    /// From `#rrggbb` or `#rrggbbaa`; a malformed text is grey rather than a
    /// panic — a colour is never worth stopping the frame for.
    #[must_use]
    pub fn hex(text: &str) -> Self {
        let digits = text.trim_start_matches('#');
        let channel = |at: usize| {
            digits
                .get(at..at + 2)
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                .map_or(0.5, |v| f32::from(v) / 255.0)
        };
        let alpha = if digits.len() >= 8 { channel(6) } else { 1.0 };
        if digits.len() < 6 {
            return Self(0.5, 0.5, 0.5, 1.0);
        }
        Self(channel(0), channel(2), channel(4), alpha)
    }

    /// The same colour, scaled.
    #[must_use]
    pub fn shade(self, k: f32) -> Self {
        Self(
            (self.0 * k).clamp(0.0, 1.0),
            (self.1 * k).clamp(0.0, 1.0),
            (self.2 * k).clamp(0.0, 1.0),
            self.3,
        )
    }

    /// The same colour, with this alpha.
    #[must_use]
    pub const fn alpha(self, a: f32) -> Self {
        Self(self.0, self.1, self.2, a)
    }
}

/// The models' colours — the robes the figures wear, on the chimneys too.
#[must_use]
pub fn model_color(model: Option<&str>) -> Rgba {
    let (r, g, b) = Model::parse(model).hue();
    Rgba(
        f32::from(r) / 255.0,
        f32::from(g) / 255.0,
        f32::from(b) / 255.0,
        1.0,
    )
}

/// Where the camera looks from, as a direction: the isometric corner. The
/// renderer takes it from here, so the cards below and the camera agree.
pub const CAMERA_FROM: [f32; 3] = [1.0, 1.0, 1.0];

/// The screen's up, in plant coordinates: `y` with the camera's direction
/// taken out, normalised — `(-1, 2, -1) / √6`. A painted figure rises along
/// it, so its feet stay on the ground as seen.
pub const CAMERA_UP: [f32; 3] = [-0.408_248_3, 0.816_496_6, -0.408_248_3];

// The palette: a bright, welcoming town — red brick under cream copings,
// slate roofs, pavement and tiles under foot, warm wood, leaves, gold for
// what shines, teal as the accent — drawn in an ink that is dark plum, never
// black. The last block is for text: light on the dark pills, dark where it
// is written straight on the ground.
const GRASS: &str = "#7f9f52";
const GRASS_DARK: &str = "#6b8a44";
const RED: &str = "#c6392f";
const RED_DARK: &str = "#962a2a";
const CREAM: &str = "#f3e7c9";
const BONE: &str = "#e9dcbd";
const PAPER: &str = "#f7efd8";
const WOOD: &str = "#9b6b45";
const WOOD_DARK: &str = "#6a4530";
const STONE: &str = "#aaa693";
const STONE_DARK: &str = "#857f72";
const GOLD: &str = "#e2b240";
const GOLD_DARK: &str = "#b88a28";
const TEAL: &str = "#3fa58c";
const PLUM: &str = "#3b2a40";
const INK: &str = "#2a1b2e";
const ACCENT: &str = "#f2c75c";
// The town around the plant: brick, slate roofs, pavement, asphalt, the
// harbour's water and the warm globes of the street lamps.
const BRICK: &str = "#b5452e";
const BRICK_DARK: &str = "#86372b";
const ROOF: &str = "#3f4953";
const ROOF_DARK: &str = "#353d47";
const PAVEMENT: &str = "#bdb6a8";
const ASPHALT: &str = "#4c5058";
const WATER: &str = "#2e9a98";
const LAMP: &str = "#fff0c0";
const SOIL: &str = "#6e5140";
// Inside the plant: a dark concrete hall, rooms tiled warm when they run,
// greyer while drafted, raw while built; daylight in the factory windows.
const HALL: &str = "#74777b";
const CONCRETE: &str = "#aca598";
const TILE_LIVE: &str = "#cfc1a4";
const TILE_DRAFT: &str = "#b2aa9c";
const TILE_RAW: &str = "#968b7b";
const DAYLIGHT: &str = "#bfe3dc";
const BELT: &str = "#4f5a66";
const OK: &str = "#7ee081";
const WARN: &str = "#ffb547";
const BAD: &str = "#ff6b6b";
const DIM: &str = "#cfc3cc";
const TEXT: &str = "#fff6e5";
// The infirmary's: a white sheet, a pale scrub green.
const WHITE_SHEET: &str = "#fcfaf4";
const SCRUB: &str = "#86bea0";
const SCRIPT: &str = "#3a2a3e";

/// The lines room's layout. Each line is a band across the room: robots
/// and printers behind the belt, the belt, the stage names and the worker
/// in front of it. The spacing is twice the first draft's, so the bands do
/// not touch.
const LINE_X0: f32 = 2.6;
const STATION_SPACING: f32 = 1.8;
const LINE_SPACING: f32 = 7.2;
/// Where the first belt lies: far enough from the back wall that the robots
/// behind it — cards leaning back toward that wall — stay clear of it.
const FIRST_LINE_Z: f32 = 3.2;

/// How a prop moves, evaluated by the renderer every frame from `t` seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anim {
    /// Rises, drifts and grows, like smoke.
    Smoke {
        /// Offsets the phase, so puffs do not move in step.
        seed: f32,
        /// Cycles per second.
        speed: f32,
    },
    /// Slides forward along `z` and sinks, like a printed sheet.
    Sheet {
        /// Offsets the phase.
        seed: f32,
        /// Cycles per second.
        speed: f32,
    },
    /// Slides along `x`, wrapping, like a belt stripe.
    Belt {
        /// The spacing between stripes — the distance after which the belt
        /// looks the same.
        period: f32,
        /// Tiles per second.
        speed: f32,
    },
    /// Bobs up and down.
    Bob {
        /// How high.
        amp: f32,
        /// How fast, in radians per second.
        speed: f32,
        /// Offsets the phase.
        seed: f32,
    },
    /// Breathes: a slow pulse of its size.
    Glow {
        /// Radians per second.
        speed: f32,
    },
    /// Swings along `z`, like an arm's boom.
    Swing {
        /// How far.
        amp: f32,
        /// Radians per second.
        speed: f32,
    },
    /// Slides toward a point on `x` and stays — the product on the belt.
    Slide {
        /// Where it goes.
        to: f32,
    },
}

/// What a prop is shaped like. Every size is a full extent, every shape is
/// centred on the prop's position; the builder below places them by base.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A box with every edge and corner rounded off.
    Rounded {
        /// Along `x`.
        w: f32,
        /// Up.
        h: f32,
        /// Along `z`.
        d: f32,
        /// The radius of the rounding, at most half the smallest side.
        r: f32,
    },
    /// A sharp box — the floors, where rounding would show as a seam.
    Cuboid {
        /// Along `x`.
        w: f32,
        /// Up.
        h: f32,
        /// Along `z`.
        d: f32,
    },
    /// An upright cylinder.
    Cylinder {
        /// The radius.
        r: f32,
        /// The height.
        h: f32,
    },
    /// An upright cone, apex up.
    Cone {
        /// The base radius.
        r: f32,
        /// The height.
        h: f32,
    },
    /// An upright cone with its top cut off.
    Frustum {
        /// The radius at the bottom.
        bottom: f32,
        /// The radius at the top.
        top: f32,
        /// The height.
        h: f32,
    },
    /// An upright capsule: a cylinder with a half-sphere at each end.
    Capsule {
        /// The radius.
        r: f32,
        /// The height, caps included.
        h: f32,
    },
    /// A sphere.
    Sphere {
        /// The radius.
        r: f32,
    },
    /// A ring lying flat; tilted by the prop to stand as an arch.
    Torus {
        /// From the centre to the middle of the tube.
        ring: f32,
        /// The tube's radius.
        tube: f32,
    },
    /// A painted figure: a flat card, `w` by `h`, that always faces the
    /// camera, textured by [`crate::sprites`]. Drawn without an outline —
    /// the ink is in the painting.
    Card {
        /// Across the screen.
        w: f32,
        /// Up the screen.
        h: f32,
        /// Who is painted on it.
        figure: Figure,
    },
}

/// What a surface is painted with, in world space, by the toon shader —
/// the brick of a wall, the slabs of a pavement, water that moves. Plain
/// for most props: a pattern is what tells a wall from a toy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Pattern {
    /// The flat colour alone.
    #[default]
    Plain,
    /// Courses of bricks with darker mortar.
    Brick,
    /// Square slabs with thin grout.
    Pavers,
    /// Boards, for fences and docks.
    Planks,
    /// A metal roof's seams, along `z`, one every tile across `x`.
    Seams,
    /// Water, its crests drifting with time.
    Water,
}

impl Pattern {
    /// The number `toon.wgsl` knows this pattern by.
    #[must_use]
    pub const fn id(self) -> u8 {
        match self {
            Self::Plain => 0,
            Self::Brick => 1,
            Self::Pavers => 2,
            Self::Planks => 3,
            Self::Seams => 4,
            Self::Water => 5,
        }
    }

    /// The size of one tile, in world units — a brick's length, a slab's
    /// side, a board's width, the gap between seams.
    #[must_use]
    pub const fn tile(self) -> f32 {
        match self {
            Self::Plain | Self::Water => 1.0,
            Self::Brick => 0.42,
            Self::Pavers => 0.6,
            Self::Planks => 0.22,
            Self::Seams => 0.34,
        }
    }
}

/// One thing to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct Prop {
    /// Its centre, in plant coordinates.
    pub at: [f32; 3],
    /// Its shape.
    pub shape: Shape,
    /// Its colour.
    pub color: Rgba,
    /// Emissive colour, for lamps and orbs.
    pub emissive: Option<Rgba>,
    /// What its surface is painted with.
    pub pattern: Pattern,
    /// Transparent, for smoke and scanner planes — drawn without an outline.
    pub translucent: bool,
    /// Rotation around `x`, `y`, `z`, in degrees, applied in that order.
    pub tilt: [f32; 3],
    /// What a click or hover does. `None` props are not even hoverable.
    pub hot: Option<Hot>,
    /// How it moves.
    pub anim: Option<Anim>,
    /// A key, so a moving prop keeps its motion across rebuilds.
    pub key: Option<String>,
    /// The figure this prop belongs to — every shape of a door, a robot or
    /// a sign carries the same one, and they grow together under the
    /// pointer. Set by the hotspot, so only hot props have one.
    pub group: Option<u32>,
}

/// What a label is drawn on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Backing {
    /// Nothing: the text alone, written straight on the ground.
    #[default]
    None,
    /// A dark ink pill — a tag hanging in the world.
    Ink,
    /// A soft grey plate, a touch see-through — text written on a board. It
    /// reads as part of the board rather than as a tag over it, and stays
    /// legible where the board, leaning away in the isometric view, is
    /// narrower on screen than the line written on it.
    Slate,
}

/// A label anchored in the world, drawn flat on the screen.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    /// Where it hangs.
    pub at: [f32; 3],
    /// The text.
    pub text: String,
    /// Font size, in CSS pixels.
    pub size: f32,
    /// The colour.
    pub color: Rgba,
    /// What it is drawn on.
    pub backing: Backing,
    /// Bold.
    pub bold: bool,
}

/// Everything to draw, and where it lies.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    /// The props.
    pub props: Vec<Prop>,
    /// The labels.
    pub labels: Vec<Label>,
    /// Lowest corner of what the camera frames.
    pub min: [f32; 3],
    /// Highest corner.
    pub max: [f32; 3],
    /// The floor's colour.
    pub floor: Rgba,
}

/// Builds a scene, remembering its extent.
struct Builder {
    scene: Scene,
    extent_set: bool,
    /// Figures made so far — the next hotspot's group.
    groups: u32,
}

impl Builder {
    fn new(floor: &str) -> Self {
        Self {
            scene: Scene {
                floor: Rgba::hex(floor),
                ..Scene::default()
            },
            extent_set: false,
            groups: 0,
        }
    }

    fn span(&mut self, a: [f32; 3], b: [f32; 3]) {
        if !self.extent_set {
            self.scene.min = a;
            self.scene.max = b;
            self.extent_set = true;
            return;
        }
        for i in 0..3 {
            self.scene.min[i] = self.scene.min[i].min(a[i]).min(b[i]);
            self.scene.max[i] = self.scene.max[i].max(a[i]).max(b[i]);
        }
    }

    /// The one way a prop is added: a shape at a centre, with its half
    /// extents for the camera's framing.
    fn prop(&mut self, shape: Shape, at: [f32; 3], half: [f32; 3], color: Rgba) -> &mut Prop {
        self.span(
            [at[0] - half[0], at[1] - half[1], at[2] - half[2]],
            [at[0] + half[0], at[1] + half[1], at[2] + half[2]],
        );
        self.scene.props.push(Prop {
            at,
            shape,
            color,
            emissive: None,
            pattern: Pattern::Plain,
            translucent: false,
            tilt: [0.0; 3],
            hot: None,
            anim: None,
            key: None,
            group: None,
        });
        let last = self.scene.props.len() - 1;
        &mut self.scene.props[last]
    }

    /// A rounded box with its base at `(x, y, z)`, extending `w` along x,
    /// `h` up, `d` along z — what most of the plant is made of.
    #[allow(clippy::too_many_arguments)] // Each one is a coordinate, a size or a colour; a struct would be a point object built for one call.
    fn rounded(
        &mut self,
        x: f32,
        y: f32,
        z: f32,
        w: f32,
        h: f32,
        d: f32,
        r: f32,
        color: Rgba,
    ) -> &mut Prop {
        let r = r.min(w.min(h).min(d) / 2.0);
        self.prop(
            Shape::Rounded { w, h, d, r },
            [x + w / 2.0, y + h / 2.0, z + d / 2.0],
            [w / 2.0, h / 2.0, d / 2.0],
            color,
        )
    }

    /// A sharp slab — a floor.
    #[allow(clippy::too_many_arguments)] // Each one is a coordinate, a size or a colour; a struct would be a point object built for one call.
    fn slab(&mut self, x: f32, y: f32, z: f32, w: f32, h: f32, d: f32, color: Rgba) -> &mut Prop {
        self.prop(
            Shape::Cuboid { w, h, d },
            [x + w / 2.0, y + h / 2.0, z + d / 2.0],
            [w / 2.0, h / 2.0, d / 2.0],
            color,
        )
    }

    /// An upright cylinder standing on `y`.
    fn cylinder(&mut self, cx: f32, y: f32, cz: f32, r: f32, h: f32, color: Rgba) -> &mut Prop {
        self.prop(
            Shape::Cylinder { r, h },
            [cx, y + h / 2.0, cz],
            [r, h / 2.0, r],
            color,
        )
    }

    /// An upright cone standing on `y`, apex up.
    fn cone(&mut self, cx: f32, y: f32, cz: f32, r: f32, h: f32, color: Rgba) -> &mut Prop {
        self.prop(
            Shape::Cone { r, h },
            [cx, y + h / 2.0, cz],
            [r, h / 2.0, r],
            color,
        )
    }

    /// A cut cone standing on `y`.
    #[allow(clippy::too_many_arguments)] // Each one is a coordinate, a size or a colour; a struct would be a point object built for one call.
    fn frustum(
        &mut self,
        cx: f32,
        y: f32,
        cz: f32,
        bottom: f32,
        top: f32,
        h: f32,
        color: Rgba,
    ) -> &mut Prop {
        let r = bottom.max(top);
        self.prop(
            Shape::Frustum { bottom, top, h },
            [cx, y + h / 2.0, cz],
            [r, h / 2.0, r],
            color,
        )
    }

    /// An upright capsule standing on `y`, `h` tall caps included.
    fn capsule(&mut self, cx: f32, y: f32, cz: f32, r: f32, h: f32, color: Rgba) -> &mut Prop {
        let h = h.max(2.0 * r);
        self.prop(
            Shape::Capsule { r, h },
            [cx, y + h / 2.0, cz],
            [r, h / 2.0, r],
            color,
        )
    }

    /// A capsule lying along `z`, centred.
    fn barrel(&mut self, cx: f32, cy: f32, cz: f32, r: f32, len: f32, color: Rgba) -> &mut Prop {
        let len = len.max(2.0 * r);
        let prop = self.prop(
            Shape::Capsule { r, h: len },
            [cx, cy, cz],
            [r, r, len / 2.0],
            color,
        );
        prop.tilt = [90.0, 0.0, 0.0];
        prop
    }

    /// A sphere, centred.
    fn sphere(&mut self, cx: f32, cy: f32, cz: f32, r: f32, color: Rgba) -> &mut Prop {
        self.prop(Shape::Sphere { r }, [cx, cy, cz], [r, r, r], color)
    }

    /// A ring standing upright across `z` — an arch over a belt.
    fn arch(&mut self, cx: f32, cy: f32, cz: f32, ring: f32, tube: f32, color: Rgba) -> &mut Prop {
        let out = ring + tube;
        let prop = self.prop(
            Shape::Torus { ring, tube },
            [cx, cy, cz],
            [tube, out, out],
            color,
        );
        prop.tilt = [0.0, 0.0, 90.0];
        prop
    }

    /// A ring lying flat.
    fn ring(&mut self, cx: f32, cy: f32, cz: f32, ring: f32, tube: f32, color: Rgba) -> &mut Prop {
        let out = ring + tube;
        self.prop(
            Shape::Torus { ring, tube },
            [cx, cy, cz],
            [out, tube, out],
            color,
        )
    }

    /// A painted figure with its feet at `(x, z)`: a soft disc of shadow on
    /// the ground, and a card `w` wide and `h` tall that rises along the
    /// camera's up rather than `y`, so it stands on that spot as seen. The
    /// card is the prop returned; the shadow is the one before it.
    fn card(&mut self, x: f32, z: f32, w: f32, h: f32, figure: Figure) -> &mut Prop {
        let shadow = self.cylinder(x, 0.0, z, w * 0.36, 0.02, Rgba::hex(INK).alpha(0.28));
        shadow.translucent = true;
        let [cx, cy, cz] = over(x, z, h / 2.0);
        self.prop(
            Shape::Card { w, h, figure },
            [cx, cy, cz],
            [w / 2.0, h / 2.0, w / 2.0],
            Rgba(1.0, 1.0, 1.0, 1.0),
        )
    }

    fn label(&mut self, x: f32, y: f32, z: f32, text: &str, size: f32, color: &str) -> &mut Label {
        self.scene.labels.push(Label {
            at: [x, y, z],
            text: text.to_string(),
            size,
            color: Rgba::hex(color),
            backing: Backing::None,
            bold: false,
        });
        let at = self.scene.labels.len() - 1;
        &mut self.scene.labels[at]
    }

    /// Where the next prop will land — the start of a figure.
    const fn mark(&self) -> usize {
        self.scene.props.len()
    }

    /// The same hotspot on every prop since `mark`: a figure made of many
    /// shapes clicks as one, and grows as one under the pointer — the call
    /// is what defines a figure, and gives it its group.
    fn hot_since(&mut self, mark: usize, hot: &Hot) {
        let group = self.groups;
        self.groups += 1;
        for prop in self.scene.props.iter_mut().skip(mark) {
            prop.hot = Some(hot.clone());
            prop.group = Some(group);
        }
    }

    /// The same motion on every prop since `mark`.
    fn anim_since(&mut self, mark: usize, anim: Anim) {
        for prop in self.scene.props.iter_mut().skip(mark) {
            prop.anim = Some(anim);
        }
    }

    /// The floor of a room, tiled, with two brick back walls that read as
    /// a corner: a cream coping on top, factory windows letting daylight in.
    fn floor(&mut self, w: f32, d: f32) {
        let color = self.scene.floor;
        self.slab(0.0, -0.06, 0.0, w, 0.06, d, color).pattern = Pattern::Pavers;
        let (wall, brick, bone) = (1.4, Rgba::hex(BRICK), Rgba::hex(BONE));
        self.rounded(0.0, -0.08, -0.3, w, wall, 0.3, 0.06, brick)
            .pattern = Pattern::Brick;
        self.rounded(-0.3, -0.08, 0.0, 0.3, wall, d, 0.06, brick)
            .pattern = Pattern::Brick;
        // The copings' top faces must not overlap in the corner, or they
        // flicker: the side one starts past the back one.
        self.rounded(-0.33, wall - 0.12, -0.33, w + 0.33, 0.1, 0.36, 0.03, bone);
        self.rounded(-0.33, wall - 0.12, 0.04, 0.36, 0.1, d - 0.04, 0.03, bone);
        let windows = |len: f32| {
            (0..)
                .map(|k| 1.2 + k as f32 * 2.4)
                .take_while(move |at| at + 0.6 < len)
        };
        for x in windows(w) {
            self.rounded(x, 0.3, -0.04, 0.7, 0.8, 0.06, 0.03, bone);
            self.rounded(
                x + 0.08,
                0.38,
                -0.01,
                0.54,
                0.64,
                0.06,
                0.03,
                Rgba::hex(DAYLIGHT),
            )
            .emissive = Some(Rgba::hex("#3a5a55"));
        }
        for z in windows(d) {
            self.rounded(-0.04, 0.3, z, 0.06, 0.8, 0.7, 0.03, bone);
            self.rounded(
                -0.01,
                0.38,
                z + 0.08,
                0.06,
                0.64,
                0.54,
                0.03,
                Rgba::hex(DAYLIGHT),
            )
            .emissive = Some(Rgba::hex("#3a5a55"));
        }
    }
}

/// A point `rise` up the screen from feet at `(x, z)` — the centre of a
/// card, or where a figure's label hangs.
fn over(x: f32, z: f32, rise: f32) -> [f32; 3] {
    [
        x + CAMERA_UP[0] * rise,
        CAMERA_UP[1] * rise,
        z + CAMERA_UP[2] * rise,
    ]
}

// ---- props --------------------------------------------------------------

fn pane(json: serde_json::Value, tip_text: &str) -> Hot {
    Hot {
        pane: Some(json),
        tip: Some(tip_text.to_string()),
        ..Hot::default()
    }
}

/// A chimney: an atomic-age stack in the model's colour — a tapered tower
/// ringed in chrome, three fins at its foot, a red beacon on top — and the
/// smoke when a session on that model is open.
#[allow(clippy::too_many_arguments)] // Each one is a coordinate, a size or a colour; a struct would be a point object built for one call.
fn chimney(
    b: &mut Builder,
    x: f32,
    y: f32,
    z: f32,
    h: f32,
    model: &str,
    smoking: bool,
    seed: f32,
    hot: &Hot,
) {
    let m = b.mark();
    let color = model_color(Some(model));
    let chrome = Rgba::hex(STONE);
    b.frustum(x, y, z, 0.46, 0.3, h, color);
    b.ring(x, y + h * 0.55, z, 0.38, 0.05, chrome);
    b.ring(x, y + h, z, 0.31, 0.09, chrome);
    for k in 0..3 {
        let angle = (k as f32).mul_add(120.0, 30.0).to_radians();
        b.rounded(
            x + angle.cos() * 0.5 - 0.07,
            y,
            z + angle.sin() * 0.5 - 0.07,
            0.14,
            h * 0.4,
            0.14,
            0.05,
            Rgba::hex(STONE_DARK),
        );
    }
    let beacon = b.sphere(x, y + h + 0.14, z, 0.1, Rgba::hex(RED));
    beacon.emissive = Some(Rgba::hex(RED));
    beacon.anim = Some(Anim::Glow { speed: 3.0 });
    b.hot_since(m, hot);
    if smoking {
        for i in 0..7 {
            let puff = b.sphere(x, y + h + 0.4, z, 0.22, Rgba(0.93, 0.9, 0.86, 0.5));
            puff.translucent = true;
            puff.anim = Some(Anim::Smoke {
                seed: seed * 3.7 + i as f32 / 7.0,
                speed: 0.28,
            });
        }
    }
}

/// A belt: a chrome frame on riveted feet, a dark rubber track between two
/// gold rails, and moving cream stripes.
fn conveyor(b: &mut Builder, x: f32, z: f32, len: f32, active: bool, key: &str) {
    let track = Rgba::hex(if active { BELT } else { ROOF_DARK });
    b.rounded(x, 0.0, z, len, 0.3, 1.0, 0.1, Rgba::hex(STONE_DARK));
    b.rounded(
        x + 0.02,
        0.14,
        z + 0.06,
        len - 0.04,
        0.18,
        0.88,
        0.06,
        track,
    );
    b.rounded(x, 0.3, z - 0.02, len, 0.06, 0.1, 0.03, Rgba::hex(GOLD_DARK));
    b.rounded(x, 0.3, z + 0.92, len, 0.06, 0.1, 0.03, Rgba::hex(GOLD_DARK));
    // A belt is a few tiles long and never negative: the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let stripes = (len * 2.0).floor().max(1.0) as usize;
    for s in 0..stripes {
        if s % 2 == 0 {
            // A rivet on each rail, every tile.
            for dz in [0.03, 0.97] {
                b.sphere(
                    x + s as f32 * 0.5 + 0.25,
                    0.36,
                    z + dz,
                    0.035,
                    Rgba::hex(STONE),
                );
            }
        }
        let stripe = b.rounded(
            x + s as f32 * 0.5,
            0.31,
            z + 0.1,
            0.1,
            0.03,
            0.8,
            0.015,
            Rgba::hex(CREAM).alpha(0.45),
        );
        stripe.translucent = true;
        if active {
            // Stripes sit every half tile, so shifting them all by the same
            // amount modulo that spacing reads as one endless belt.
            stripe.anim = Some(Anim::Belt {
                period: 0.5,
                speed: 0.9,
            });
            stripe.key = Some(format!("{key}:stripe:{s}"));
        }
    }
}

/// A gate: a gold arch over the belt on two chrome posts, a radar dish and
/// a red bulb on top; a glowing teal disc fills it while it scans.
fn scanner(b: &mut Builder, x: f32, z: f32, active: bool, hot: &Hot) {
    let m = b.mark();
    let chrome = Rgba::hex(STONE);
    b.capsule(x + 0.5, 0.0, z - 0.1, 0.09, 1.0, chrome);
    b.capsule(x + 0.5, 0.0, z + 1.1, 0.09, 1.0, chrome);
    b.arch(x + 0.5, 0.95, z + 0.5, 0.6, 0.085, Rgba::hex(GOLD));
    b.cylinder(x + 0.5, 1.6, z + 0.5, 0.025, 0.4, Rgba::hex(INK));
    let dish = b.cone(x + 0.5, 1.72, z + 0.5, 0.22, 0.12, chrome);
    dish.tilt = [180.0, 0.0, 0.0];
    let bulb = b.sphere(x + 0.5, 2.06, z + 0.5, 0.06, Rgba::hex(RED));
    bulb.emissive = Some(Rgba::hex(RED));
    b.hot_since(m, hot);
    if active {
        let beam = b.cylinder(
            x + 0.5,
            0.94,
            z + 0.5,
            0.56,
            0.02,
            Rgba::hex(TEAL).alpha(0.3),
        );
        beam.tilt = [0.0, 0.0, 90.0];
        beam.translucent = true;
        beam.emissive = Some(Rgba::hex(TEAL));
        beam.anim = Some(Anim::Glow { speed: 7.0 });
        let lamp = b.sphere(x + 0.5, 1.64, z + 0.5, 0.09, Rgba::hex(TEAL));
        lamp.emissive = Some(Rgba::hex(TEAL));
    }
}

/// A robot, painted: `wrench` is Bender, the steel-grey builder; the other
/// is Hedonismbot, the gold inspector with his clipboard. It bobs while it
/// works; the model shows as a badge.
fn robot(
    b: &mut Builder,
    x: f32,
    z: f32,
    wrench: bool,
    active: bool,
    model: Option<&str>,
    hot: &Hot,
) {
    let m = b.mark();
    let figure = Figure::Robot {
        model: Model::parse(model),
        wrench,
        active,
    };
    let card = b.card(x + 0.5, z + 0.5, 1.5, 2.0, figure);
    if active {
        card.anim = Some(Anim::Bob {
            amp: 0.03,
            speed: 6.0,
            seed: x,
        });
    }
    b.hot_since(m, hot);
}

/// A printer: a terminal of the atomic age — a chrome console, a cream CRT
/// with a glowing teal screen, a keyboard — feeding punch cards forward.
fn printer(b: &mut Builder, x: f32, z: f32, active: bool, hot: &Hot) {
    let m = b.mark();
    b.rounded(x + 0.1, 0.0, z + 0.1, 0.8, 0.5, 0.7, 0.1, Rgba::hex(STONE));
    b.rounded(
        x + 0.18,
        0.5,
        z + 0.12,
        0.64,
        0.5,
        0.45,
        0.12,
        Rgba::hex(CREAM),
    );
    // The screen sits proud of the shell, toward the camera.
    let screen = b.rounded(
        x + 0.26,
        0.6,
        z + 0.5,
        0.48,
        0.3,
        0.1,
        0.06,
        Rgba::hex(TEAL),
    );
    screen.emissive = Some(Rgba::hex(TEAL).shade(0.6));
    b.rounded(
        x + 0.22,
        0.5,
        z + 0.6,
        0.56,
        0.06,
        0.26,
        0.03,
        Rgba::hex(STONE_DARK),
    );
    b.cylinder(x + 0.82, 0.98, z + 0.2, 0.02, 0.35, Rgba::hex(INK));
    b.rounded(
        x + 0.25,
        0.3,
        z + 0.8,
        0.5,
        0.06,
        0.04,
        0.02,
        Rgba::hex(INK),
    );
    b.hot_since(m, hot);
    if active {
        let led = b.sphere(x + 0.82, 1.36, z + 0.2, 0.05, Rgba::hex(OK));
        led.emissive = Some(Rgba::hex(OK));
        for i in 0..3 {
            let sheet = b.rounded(
                x + 0.3,
                0.4,
                z + 0.9,
                0.4,
                0.02,
                0.3,
                0.01,
                Rgba::hex(CREAM),
            );
            sheet.anim = Some(Anim::Sheet {
                seed: i as f32 / 3.0,
                speed: 0.55,
            });
        }
    }
}

/// A robotic arm: a chrome mast on a dark foot, two gold piston rings, a
/// gold boom, two bone claws and a red lamp at the joint.
fn arm(b: &mut Builder, x: f32, z: f32, active: bool, hot: &Hot) {
    let m = b.mark();
    let gold = Rgba::hex(GOLD);
    b.frustum(x + 0.5, 0.0, z + 0.5, 0.42, 0.3, 0.3, Rgba::hex(STONE_DARK));
    b.capsule(x + 0.5, 0.25, z + 0.5, 0.12, 1.0, Rgba::hex(STONE));
    b.ring(x + 0.5, 0.55, z + 0.5, 0.14, 0.04, gold);
    b.ring(x + 0.5, 0.85, z + 0.5, 0.14, 0.04, gold);
    let boom = b.mark();
    b.barrel(x + 0.5, 1.08, z + 0.95, 0.1, 0.95, gold.shade(1.1));
    let lamp = b.sphere(x + 0.5, 1.26, z + 0.5, 0.07, Rgba::hex(RED));
    lamp.emissive = Some(Rgba::hex(RED));
    let claw_l = b.cone(x + 0.38, 0.72, z + 1.42, 0.08, 0.3, Rgba::hex(BONE));
    claw_l.tilt = [180.0, 0.0, 0.0];
    let claw_r = b.cone(x + 0.62, 0.72, z + 1.42, 0.08, 0.3, Rgba::hex(BONE));
    claw_r.tilt = [180.0, 0.0, 0.0];
    b.hot_since(m, hot);
    if active {
        b.anim_since(
            boom,
            Anim::Swing {
                amp: 0.3,
                speed: 2.4,
            },
        );
    }
}

/// The product on the belt: a rounded crate.
fn crate_box(
    b: &mut Builder,
    x: f32,
    y: f32,
    z: f32,
    color: &str,
    key: Option<&str>,
    slide_to: Option<f32>,
) {
    let body = b.rounded(x, y, z, 0.5, 0.45, 0.5, 0.1, Rgba::hex(color));
    if let Some(key) = key {
        body.key = Some(key.to_string());
    }
    if let Some(to) = slide_to {
        body.anim = Some(Anim::Slide { to });
    }
}

/// A follower: a painted cut-out in the model's colour, bobbing at work,
/// their name over their head.
fn person(b: &mut Builder, x: f32, z: f32, model: Option<&str>, name: &str, hot: &Hot) {
    let m = b.mark();
    let (fx, fz) = (x + 0.5, z + 0.5);
    let figure = Figure::Follower {
        model: Model::parse(model),
    };
    let card = b.card(fx, fz, 1.2, 1.8, figure);
    card.anim = Some(Anim::Bob {
        amp: 0.05,
        speed: 3.2,
        seed: x + z,
    });
    b.hot_since(m, hot);
    let [lx, ly, lz] = over(fx, fz, 2.05);
    b.label(lx, ly, lz, name, 11.0, TEXT).backing = Backing::Ink;
}

/// A crew: several agents at one station, as one figure — the elder —
/// named by their issues; a click opens the pane that lists them all.
fn elder(b: &mut Builder, x: f32, z: f32, name: &str, hot: &Hot) {
    let m = b.mark();
    let (fx, fz) = (x + 0.5, z + 0.5);
    let card = b.card(fx, fz, 1.2, 1.8, Figure::Elder);
    card.anim = Some(Anim::Bob {
        amp: 0.03,
        speed: 1.6,
        seed: x + z,
    });
    b.hot_since(m, hot);
    let [lx, ly, lz] = over(fx, fz, 2.05);
    b.label(lx, ly, lz, name, 11.0, TEXT).backing = Backing::Ink;
}

/// The steward: a painted cut-out — hooded, bearded, red-eyed, a staff with
/// a pale orb — taller than anyone, by the door.
fn wizard(b: &mut Builder, x: f32, z: f32, hot: &Hot) {
    let m = b.mark();
    let (fx, fz) = (x + 0.5, z + 0.5);
    b.card(fx, fz, 1.6, 2.6, Figure::Steward);
    b.hot_since(m, hot);
    let [lx, ly, lz] = over(fx, fz, 2.85);
    b.label(lx, ly, lz, "the steward", 11.0, TEXT).backing = Backing::Ink;
}

/// The doctor: a painted cut-out — the coral, lobster-faced physician in a
/// white coat, head mirror on, a blue book under the claw.
fn physician(b: &mut Builder, x: f32, z: f32, hot: &Hot) {
    let m = b.mark();
    let (fx, fz) = (x + 0.5, z + 0.5);
    b.card(fx, fz, 1.6, 2.6, Figure::Doctor);
    b.hot_since(m, hot);
    let [lx, ly, lz] = over(fx, fz, 2.85);
    b.label(lx, ly, lz, "the doctor", 11.0, TEXT).backing = Backing::Ink;
}

/// The janitor: leaning on a push broom at the tower's foot by
/// the water, a mop bucket beside them. A click opens their yard: what the
/// workspace weighs, what a sweep would free.
fn caretaker(b: &mut Builder, x: f32, z: f32) {
    let hot = pane(
        serde_json::json!({"kind": "janitor"}),
        "The janitor\nWeighs what the workspace keeps on disk and sweeps what is no longer useful — no model, just rules.",
    );
    let m = b.mark();
    let (fx, fz) = (x + 0.5, z + 0.5);
    b.card(fx, fz, 1.5, 2.4, Figure::Janitor);
    // The mop bucket: a grey pail on its castors, a mop's handle up out of it.
    b.cylinder(fx + 0.85, 0.0, fz + 0.1, 0.26, 0.34, Rgba::hex(STONE));
    b.ring(fx + 0.85, 0.34, fz + 0.1, 0.26, 0.04, Rgba::hex(STONE_DARK));
    b.capsule(fx + 0.9, 0.3, fz + 0.05, 0.035, 0.9, Rgba::hex(WOOD));
    b.hot_since(m, &hot);
    let [lx, ly, lz] = over(fx, fz, 2.65);
    b.label(lx, ly, lz, "the janitor", 11.0, TEXT).backing = Backing::Ink;
}

/// What a click on the doctor, the bed or the cabinet opens.
fn doctor_hot(tip_text: &str) -> Hot {
    pane(serde_json::json!({"kind": "doctor"}), tip_text)
}

/// A medicine cabinet: cream, a red cross proud of its door.
fn cabinet(b: &mut Builder, x: f32, z: f32, w: f32, h: f32, hot: &Hot) {
    let m = b.mark();
    let d = 0.5;
    b.rounded(x, 0.0, z, w, h, d, 0.08, Rgba::hex(CREAM));
    let (cx, cy) = (x + w / 2.0, h * 0.6);
    let arm = w * 0.5;
    let bar = w * 0.16;
    // The cross sits proud of the door, toward the camera.
    b.rounded(
        cx - bar / 2.0,
        cy - arm / 2.0,
        z + d,
        bar,
        arm,
        0.04,
        0.02,
        Rgba::hex(RED),
    );
    b.rounded(
        cx - arm / 2.0,
        cy - bar / 2.0,
        z + d,
        arm,
        bar,
        0.04,
        0.02,
        Rgba::hex(RED),
    );
    b.hot_since(m, hot);
}

/// An examination bed: a white mattress on four chrome legs, a teal pillow,
/// a pale blanket folded over the foot.
fn bed(b: &mut Builder, x: f32, z: f32, len: f32, hot: &Hot) {
    let m = b.mark();
    let (wide, top) = (1.2, 0.35);
    let chrome = Rgba::hex(STONE);
    for (lx, lz) in [
        (0.12, 0.12),
        (len - 0.12, 0.12),
        (0.12, wide - 0.12),
        (len - 0.12, wide - 0.12),
    ] {
        b.cylinder(x + lx, 0.0, z + lz, 0.05, top, chrome);
    }
    b.rounded(x, top, z, len, 0.28, wide, 0.1, Rgba::hex(WHITE_SHEET));
    b.rounded(
        x + 0.15,
        top + 0.28,
        z + 0.25,
        0.55,
        0.14,
        0.7,
        0.06,
        Rgba::hex(TEAL),
    );
    b.rounded(
        x + len * 0.5,
        top + 0.28,
        z + 0.08,
        len * 0.45,
        0.07,
        wide - 0.16,
        0.03,
        Rgba::hex(SCRUB),
    );
    b.hot_since(m, hot);
}

/// A sign: a retro billboard — a chrome frame on one chrome pole, the board
/// in it, two gold bulbs over it.
#[allow(clippy::too_many_arguments)] // Each one is a coordinate, a size or a colour; a struct would be a point object built for one call.
fn sign(b: &mut Builder, x: f32, z: f32, w: f32, title: &str, sub: &str, face: &str, hot: &Hot) {
    let m = b.mark();
    let chrome = Rgba::hex(STONE);
    b.cylinder(x + w / 2.0, 0.0, z + 0.5, 0.07, 1.1, chrome);
    b.rounded(x - 0.08, 0.97, z + 0.38, w + 0.16, 1.16, 0.12, 0.05, chrome);
    b.rounded(x, 1.05, z + 0.42, w, 1.0, 0.14, 0.06, Rgba::hex(face));
    for bx in [x + 0.3, x + w - 0.3] {
        let bulb = b.sphere(bx, 2.22, z + 0.5, 0.08, Rgba::hex(GOLD));
        bulb.emissive = Some(Rgba::hex(GOLD).shade(0.7));
    }
    b.hot_since(m, hot);
    // On the face of the board — `z + 0.56`, the side the camera sees —
    // which spans 1.05..2.05 in height. A label anchored behind the face
    // lands up and left of it on screen.
    let face = z + 0.57;
    let head = b.label(x + w / 2.0, 1.78, face, title, 13.0, INK);
    head.bold = true;
    head.backing = Backing::Slate;
    b.label(x + w / 2.0, 1.36, face, sub, 11.0, SCRIPT).backing = Backing::Slate;
}

/// The door: a vault's gear door — a gold cog ring standing in the wall, a
/// round steel leaf that glows gold when the plant is at work, a chrome hub.
fn door(b: &mut Builder, x: f32, z: f32, lit: bool, hot: &Hot) {
    let m = b.mark();
    let (cx, cy) = (x + 0.65, 0.95);
    // The camera looks from +z: ring, leaf and hub stand proud of the wall
    // in that order, each a step toward the viewer.
    let ring = b.ring(cx, cy - 0.12, z, 0.78, 0.12, Rgba::hex(GOLD));
    ring.tilt = [90.0, 0.0, 0.0];
    for k in 0..8 {
        let angle = (k as f32 * 45.0).to_radians();
        b.rounded(
            cx + angle.cos() * 0.86 - 0.09,
            cy + angle.sin() * 0.86 - 0.09,
            z - 0.04,
            0.18,
            0.18,
            0.14,
            0.04,
            Rgba::hex(GOLD_DARK),
        );
    }
    let leaf = b.cylinder(
        cx,
        cy - 0.06,
        z + 0.06,
        0.72,
        0.12,
        Rgba::hex(if lit { "#f3cf6b" } else { PLUM }),
    );
    leaf.tilt = [90.0, 0.0, 0.0];
    if lit {
        leaf.emissive = Some(Rgba::hex("#9c7a28"));
        leaf.anim = Some(Anim::Glow { speed: 2.0 });
    }
    let hub = b.cylinder(cx, cy - 0.06, z + 0.16, 0.26, 0.12, Rgba::hex(STONE_DARK));
    hub.tilt = [90.0, 0.0, 0.0];
    b.hot_since(m, hot);
    let enter = b.label(cx, 2.05, z + 0.2, "ENTER", 11.0, ACCENT);
    enter.bold = true;
    enter.backing = Backing::Ink;
}

/// A red-and-cream striped barrier on two chrome posts capped in gold.
fn barrier(b: &mut Builder, x: f32, z: f32, len: f32) {
    let chrome = Rgba::hex(STONE);
    b.cylinder(x + 0.05, 0.0, z + 0.45, 0.06, 0.72, chrome);
    b.cylinder(x + len - 0.05, 0.0, z + 0.45, 0.06, 0.72, chrome);
    b.sphere(x + 0.05, 0.78, z + 0.45, 0.08, Rgba::hex(GOLD));
    b.sphere(x + len - 0.05, 0.78, z + 0.45, 0.08, Rgba::hex(GOLD));
    b.rounded(x, 0.45, z + 0.38, len, 0.26, 0.14, 0.07, Rgba::hex(RED));
    // A barrier is a few tiles long and never negative: the cast is exact.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let stripes = (len / 0.4).floor() as usize;
    for s in 0..stripes {
        b.rounded(
            x + s as f32 * 0.4 + 0.1,
            0.45,
            z + 0.37,
            0.15,
            0.26,
            0.16,
            0.05,
            Rgba::hex(CREAM),
        );
    }
}

/// A traffic cone, red with a cream band.
fn cone(b: &mut Builder, x: f32, z: f32) {
    let red = Rgba::hex(RED);
    b.rounded(x + 0.2, 0.0, z + 0.2, 0.6, 0.06, 0.6, 0.03, red);
    b.cone(x + 0.5, 0.06, z + 0.5, 0.26, 0.7, red);
    b.frustum(x + 0.5, 0.32, z + 0.5, 0.17, 0.14, 0.1, Rgba::hex(CREAM));
}

/// A screen: a cream CRT terminal on a chrome stand, its glass glowing in a
/// colour, a dial and an antenna. `y` is the surface it stands on: a desk's
/// top, or `0.0` for the floor — a stand placed at the floor inside a desk
/// shows as a screen half-buried in it.
fn screen(b: &mut Builder, x: f32, z: f32, y: f32, color: &str, hot: &Hot) {
    let m = b.mark();
    let chrome = Rgba::hex(STONE);
    b.cylinder(x + 0.55, y, z + 0.55, 0.26, 0.05, Rgba::hex(STONE_DARK));
    b.cylinder(x + 0.55, y + 0.05, z + 0.55, 0.07, 0.35, chrome);
    b.rounded(
        x + 0.05,
        y + 0.4,
        z + 0.3,
        1.0,
        0.8,
        0.5,
        0.16,
        Rgba::hex(CREAM),
    );
    // The glass sits proud of the shell, or the shell's face hides it.
    let glass = b.rounded(
        x + 0.14,
        y + 0.5,
        z + 0.74,
        0.82,
        0.58,
        0.1,
        0.1,
        Rgba::hex(color),
    );
    glass.emissive = Some(Rgba::hex(color));
    glass.anim = Some(Anim::Glow { speed: 3.0 });
    let dial = b.cylinder(x + 0.88, y + 0.46, z + 0.72, 0.05, 0.06, chrome);
    dial.tilt = [90.0, 0.0, 0.0];
    b.cylinder(x + 0.9, y + 1.2, z + 0.5, 0.015, 0.3, Rgba::hex(INK));
    b.hot_since(m, hot);
}

/// A shelf: a chrome rack — a dark back, two steel boards — with crates on it.
fn shelf(
    b: &mut Builder,
    x: f32,
    z: f32,
    title: &str,
    crates: &[(String, String, Option<Hot>)],
    fallback: &Hot,
) {
    let chrome = Rgba::hex(STONE);
    b.rounded(x, 0.0, z - 0.12, 3.6, 2.1, 0.14, 0.1, Rgba::hex(STONE_DARK));
    b.rounded(x, 0.0, z, 3.6, 0.18, 1.2, 0.08, chrome);
    b.rounded(x, 1.3, z, 3.6, 0.14, 1.2, 0.07, chrome);
    b.cylinder(x + 0.08, 0.0, z + 1.1, 0.04, 2.1, chrome);
    b.cylinder(x + 3.52, 0.0, z + 1.1, 0.04, 2.1, chrome);
    b.label(x + 1.8, 2.45, z - 0.05, title, 11.0, TEXT).backing = Backing::Ink;
    if crates.is_empty() {
        b.label(x + 1.8, 0.7, z + 0.6, "empty shelf", 11.0, SCRIPT);
    }
    for (i, (name, color, hot)) in crates.iter().take(8).enumerate() {
        let cx = x + 0.25 + (i % 4) as f32 * 0.82;
        let cy = if i < 4 { 0.18 } else { 1.44 };
        let m = b.mark();
        crate_box(b, cx, cy, z + 0.35, color, None, None);
        b.hot_since(m, hot.as_ref().unwrap_or(fallback));
        b.label(cx + 0.25, cy + 0.85, z + 0.6, name, 10.0, TEXT)
            .backing = Backing::Ink;
    }
}

// ---- A · the plant from outside --------------------------------------------

fn board_counts(snap: &Snapshot) -> String {
    let Some(board) = &snap.board else {
        return "no GitHub board".to_string();
    };
    let (mut todo, mut done) = (0, 0);
    for m in board.milestones.iter().filter(|m| m.issue.state == "open") {
        for t in &m.tasks {
            if matches!(t.status, IssueStatus::Done | IssueStatus::Delivered) {
                done += 1;
            } else {
                todo += 1;
            }
        }
    }
    format!("{todo} to do · {done} done")
}

fn employee_hot(e: &Employee) -> Hot {
    pane(
        serde_json::json!({"kind": "employee", "id": e.id}),
        &format!(
            "{}\n{}{} · round {}\nlast write {} ago\n{}",
            e.name,
            e.stage.as_deref().unwrap_or("—"),
            e.model
                .as_ref()
                .map_or_else(String::new, |m| format!(" · {m}")),
            e.round.as_deref().unwrap_or("—"),
            e.age_secs.map_or_else(|| "?".to_string(), age),
            e.last_line
        ),
    )
}

/// A horizontal pipe along `x`, from `x0` to `x1`, its axis at `(y, z)`.
fn pipe(b: &mut Builder, x0: f32, x1: f32, y: f32, z: f32, r: f32) {
    let len = x1 - x0;
    let run = b.cylinder(
        f32::midpoint(x0, x1),
        y - len / 2.0,
        z,
        r,
        len,
        Rgba::hex(STONE),
    );
    run.tilt = [0.0, 0.0, 90.0];
}

/// A street lamp with its feet at `(x, z)`: a dark post and a warm globe.
fn lamp(b: &mut Builder, x: f32, z: f32) {
    b.cylinder(x, 0.0, z, 0.13, 0.22, Rgba::hex(STONE_DARK));
    b.cylinder(x, 0.0, z, 0.05, 2.4, Rgba::hex(PLUM));
    b.cylinder(x, 2.36, z, 0.12, 0.08, Rgba::hex(PLUM));
    let globe = b.sphere(x, 2.6, z, 0.2, Rgba::hex(LAMP));
    globe.emissive = Some(Rgba::hex("#a8873a"));
}

/// A round tree with its foot at `(x, z)`.
fn tree(b: &mut Builder, x: f32, z: f32) {
    b.cylinder(x, 0.0, z, 0.3, 0.12, Rgba::hex(SOIL));
    b.cylinder(x, 0.0, z, 0.1, 1.2, Rgba::hex(WOOD_DARK));
    b.sphere(x, 1.55, z, 0.62, Rgba::hex(GRASS));
    b.sphere(x + 0.3, 1.25, z + 0.25, 0.4, Rgba::hex(GRASS_DARK));
}

/// The harbour along the plant's right side: the quay's stone edge with its
/// bollards, and a little boat bobbing at its mooring.
fn harbour(b: &mut Builder, quay: f32, depth: f32) {
    b.rounded(
        quay - 0.25,
        -0.5,
        0.0,
        0.35,
        0.54,
        depth,
        0.06,
        Rgba::hex(STONE),
    );
    for z in [1.6, 4.4, 9.6, 14.2] {
        b.capsule(quay - 0.08, 0.04, z, 0.11, 0.36, Rgba::hex(PLUM));
    }
    let m = b.mark();
    b.rounded(quay + 1.5, -0.46, 2.4, 1.3, 0.42, 3.2, 0.2, Rgba::hex(BONE));
    b.rounded(
        quay + 1.46,
        -0.16,
        2.36,
        1.38,
        0.1,
        3.28,
        0.05,
        Rgba::hex(RED),
    );
    b.rounded(
        quay + 1.75,
        -0.06,
        3.4,
        0.8,
        0.5,
        1.0,
        0.12,
        Rgba::hex(TEAL),
    );
    b.capsule(quay + 2.15, 0.4, 3.9, 0.07, 0.4, Rgba::hex(STONE_DARK));
    b.anim_since(
        m,
        Anim::Bob {
            amp: 0.04,
            speed: 1.3,
            seed: 0.0,
        },
    );
}

/// The street across the plant's front, from the left edge to the quay:
/// asphalt between two kerbs, a dashed gold line down the middle and a
/// zebra crossing by the door.
fn street(b: &mut Builder, end: f32, z: f32) {
    let width = 2.8;
    b.slab(0.0, 0.0, z, end, 0.012, width, Rgba::hex(ASPHALT));
    for kz in [z - 0.12, z + width - 0.02] {
        b.rounded(0.0, 0.0, kz, end, 0.07, 0.14, 0.03, Rgba::hex(STONE));
    }
    // Dashes from past the crossing to short of the quay.
    let dashes = (0..)
        .map(|k| 3.2 + k as f32 * 1.2)
        .take_while(|x| x + 0.6 < end - 0.4);
    for x in dashes {
        b.rounded(
            x,
            0.012,
            z + width / 2.0 - 0.03,
            0.6,
            0.012,
            0.06,
            0.005,
            Rgba::hex(GOLD),
        );
    }
    for k in 0..6 {
        b.rounded(
            0.9,
            0.012,
            z + 0.2 + k as f32 * 0.42,
            1.4,
            0.012,
            0.24,
            0.005,
            Rgba::hex(PAPER),
        );
    }
}

/// A bed of tulips in a stone planter, its back corner at `(fx, fz)`.
fn tulip_bed(b: &mut Builder, fx: f32, fz: f32) {
    let (fw, fd) = (1.8, 5.4);
    b.rounded(fx, 0.0, fz, fw, 0.25, fd, 0.06, Rgba::hex(STONE));
    b.rounded(
        fx + 0.1,
        0.0,
        fz + 0.1,
        fw - 0.2,
        0.27,
        fd - 0.2,
        0.05,
        Rgba::hex(SOIL),
    );
    let blooms = [RED, "#e88aa8", GOLD, "#e88aa8"];
    for row in 0..6 {
        for col in 0..2 {
            let (x, z) = (fx + 0.5 + col as f32 * 0.8, fz + 0.5 + row as f32 * 0.85);
            b.capsule(x, 0.27, z, 0.025, 0.3, Rgba::hex(GRASS_DARK));
            b.sphere(
                x,
                0.6,
                z,
                0.12,
                Rgba::hex(blooms[(row + col) % blooms.len()]),
            );
        }
    }
}

/// What stands around the plant: a brick neighbour in the back corner with
/// its rooftop water tank, a plank fence along the back, a bed of tulips
/// on the left, trees and lamps along the pavement.
fn neighbourhood(b: &mut Builder, quay: f32, street_z: f32) {
    // The neighbour: three storeys of brick, windows on the two faces the
    // camera sees, a cornice and a wooden water tank on cone legs.
    let (nx, nz, nw, nd, nh) = (0.3, 0.4, 2.3, 2.1, 4.4);
    b.rounded(nx, 0.0, nz, nw, nh, nd, 0.1, Rgba::hex(BRICK_DARK))
        .pattern = Pattern::Brick;
    b.rounded(
        nx - 0.06,
        nh - 0.22,
        nz - 0.06,
        nw + 0.12,
        0.22,
        nd + 0.12,
        0.06,
        Rgba::hex(BONE),
    );
    for row in 0..3 {
        let y = 0.7 + row as f32 * 1.25;
        for col in 0..2 {
            let off = 0.45 + col as f32 * 0.95;
            b.rounded(nx + off, y, nz + nd, 0.5, 0.75, 0.06, 0.03, Rgba::hex(BONE));
            b.rounded(
                nx + off + 0.07,
                y + 0.07,
                nz + nd + 0.03,
                0.36,
                0.61,
                0.06,
                0.03,
                Rgba::hex(PLUM),
            );
            b.rounded(
                nx + nw,
                y,
                nz + off - 0.1,
                0.06,
                0.75,
                0.5,
                0.03,
                Rgba::hex(BONE),
            );
            b.rounded(
                nx + nw + 0.03,
                y + 0.07,
                nz + off - 0.03,
                0.06,
                0.61,
                0.36,
                0.03,
                Rgba::hex(PLUM),
            );
        }
    }
    let (wx, wz) = (nx + 0.9, nz + 0.9);
    for (dx, dz) in [(-0.3, -0.3), (0.3, -0.3), (-0.3, 0.3), (0.3, 0.3)] {
        b.cylinder(wx + dx, nh, wz + dz, 0.03, 0.5, Rgba::hex(WOOD_DARK));
    }
    b.cylinder(wx, nh + 0.5, wz, 0.45, 0.7, Rgba::hex(WOOD))
        .pattern = Pattern::Planks;
    b.cone(wx, nh + 1.2, wz, 0.52, 0.35, Rgba::hex(WOOD_DARK));
    // The fence along the back, posts every two tiles.
    b.rounded(
        nx + nw + 0.2,
        0.0,
        0.15,
        quay - nx - nw - 0.6,
        0.9,
        0.1,
        0.04,
        Rgba::hex(WOOD),
    )
    .pattern = Pattern::Planks;
    let posts = (0..)
        .map(|k| nx + nw + 0.3 + k as f32 * 2.0)
        .take_while(|x| *x < quay - 0.4);
    for px in posts {
        b.rounded(px, 0.0, 0.1, 0.14, 1.05, 0.16, 0.04, Rgba::hex(WOOD_DARK));
    }
    tulip_bed(b, 0.5, 3.2);
    tree(b, 0.9, street_z - 0.75);
    tree(b, quay - 1.1, 1.3);
    // One lamp on the plant's side, by the quay, so none stands in front
    // of the door or the board; two across the street, at its ends, so none
    // stands in front of the workers waiting on the asphalt.
    lamp(b, quay - 1.2, street_z - 0.45);
    lamp(b, 4.0, street_z + 3.15);
    lamp(b, quay - 1.4, street_z + 3.15);
}

#[allow(clippy::too_many_lines)] // One level is one list of props; cutting it in two hides the layout.
fn factory(snap: &Snapshot) -> Scene {
    let mut b = Builder::new(PAVEMENT);
    // Paved ground up to the quay, the harbour a step down beyond it, and a
    // street across the front that ends at the water.
    let (gw, gd, quay) = (21.0, 15.0, 17.0);
    let ground = b.scene.floor;
    b.slab(0.0, -0.06, 0.0, quay, 0.06, gd, ground).pattern = Pattern::Pavers;
    b.slab(quay, -0.42, 0.0, gw - quay, 0.08, gd, Rgba::hex(WATER))
        .pattern = Pattern::Water;
    let street_z = 10.9;
    harbour(&mut b, quay, gd);
    street(&mut b, quay - 0.25, street_z);
    neighbourhood(&mut b, quay, street_z);
    // Planet Express: a brick hangar under dark barrel vaults framed by red
    // arches, the tall tapering tower at its side — a balcony, a red dome,
    // a gold spire — and the project's name on a gantry over the front.
    let (bx, bz, bw, bd, bh) = (3.0, 2.5, 9.2, 6.5, 3.0);
    b.rounded(bx, 0.0, bz, bw, bh, bd, 0.2, Rgba::hex(BRICK))
        .pattern = Pattern::Brick;
    // A cream cornice, and a darker brick plinth at the foot.
    b.rounded(
        bx - 0.08,
        bh - 0.3,
        bz - 0.08,
        bw + 0.16,
        0.28,
        bd + 0.16,
        0.12,
        Rgba::hex(BONE),
    );
    b.rounded(
        bx - 0.06,
        0.0,
        bz - 0.06,
        bw + 0.12,
        0.4,
        bd + 0.12,
        0.1,
        Rgba::hex(BRICK_DARK),
    );
    // Drain pipes down the front corners.
    // A drain pipe down the front's right corner; the left one is the door's.
    b.cylinder(
        bx + bw - 0.3,
        0.0,
        bz + bd + 0.12,
        0.07,
        bh - 0.2,
        Rgba::hex(STONE),
    );
    // A flat, seamed roof. Low vaults cover only its front half, so the
    // back half stays clear for the chimneys.
    b.rounded(
        bx + 0.1,
        bh - 0.02,
        bz + 0.1,
        bw - 0.2,
        0.06,
        bd - 0.2,
        0.02,
        Rgba::hex(ROOF_DARK),
    )
    .pattern = Pattern::Seams;
    let vault_z = bz + bd - 1.9;
    for i in 0..4 {
        let cx = bx + 1.3 + i as f32 * 2.2;
        let roof = Rgba::hex(if i % 2 == 0 { ROOF } else { ROOF_DARK });
        b.barrel(cx, bh, vault_z, 0.5, 2.6, roof).pattern = Pattern::Seams;
        // The red arch framing each vault's front end, its lower half
        // inside the wall.
        let arch = b.ring(cx, bh, vault_z + 0.8, 0.5, 0.09, Rgba::hex(RED));
        arch.tilt = [90.0, 0.0, 0.0];
    }
    let glow = !snap.factory.idle;
    let pane_color = Rgba::hex(if glow { "#f3cf6b" } else { PLUM });
    // The tower: a brick drum narrowing as it rises, two cream bands, three
    // portholes facing the viewer; a balcony with its railing, a band, the
    // red dome with its ribs, and a gold spire with a glowing ball.
    let (tx, tz, tr, th) = (bx + bw + 1.5, bz + bd / 2.0 - 0.3, 1.5, 6.4);
    let radius_at = |y: f32| tr + 0.12 - 0.24 * y / th;
    b.frustum(
        tx,
        0.0,
        tz,
        radius_at(0.0) + 0.1,
        radius_at(0.45) + 0.1,
        0.45,
        Rgba::hex(BRICK_DARK),
    );
    b.frustum(
        tx,
        0.0,
        tz,
        radius_at(0.0),
        radius_at(th),
        th,
        Rgba::hex(BRICK),
    )
    .pattern = Pattern::Brick;
    for y in [2.3, 4.6] {
        b.frustum(
            tx,
            y,
            tz,
            radius_at(y) + 0.05,
            radius_at(y + 0.3) + 0.05,
            0.3,
            Rgba::hex(BONE),
        );
    }
    b.cylinder(tx, th - 0.1, tz, tr + 0.4, 0.18, Rgba::hex(STONE));
    b.ring(tx, th + 0.5, tz, tr + 0.33, 0.05, Rgba::hex(STONE));
    for k in 0..12 {
        let a = (k as f32 * 30.0).to_radians();
        b.cylinder(
            tx + a.cos() * (tr + 0.33),
            th + 0.08,
            tz + a.sin() * (tr + 0.33),
            0.03,
            0.42,
            Rgba::hex(STONE),
        );
    }
    b.cylinder(tx, th + 0.08, tz, tr - 0.1, 0.4, Rgba::hex("#4f7a73"));
    let dome = tr - 0.22;
    b.sphere(tx, th + 0.48, tz, dome, Rgba::hex(RED));
    for k in 0..4 {
        let rib = b.ring(tx, th + 0.48, tz, dome, 0.04, Rgba::hex(RED_DARK));
        rib.tilt = [0.0, k as f32 * 45.0, 90.0];
    }
    b.cylinder(tx, th + 0.4 + dome, tz, 0.22, 0.25, Rgba::hex(STONE));
    b.cone(tx, th + 0.6 + dome, tz, 0.12, 1.3, Rgba::hex(GOLD));
    b.ring(tx, th + 0.9 + dome, tz, 0.16, 0.03, Rgba::hex(GOLD_DARK));
    let ball = b.sphere(tx, th + 2.0 + dome, tz, 0.22, Rgba::hex(GOLD));
    ball.emissive = Some(Rgba::hex(GOLD).shade(0.5));
    ball.anim = Some(Anim::Glow { speed: 2.6 });
    let toward = std::f32::consts::FRAC_1_SQRT_2;
    for k in 0..3 {
        let wy = 1.3 + k as f32 * 1.75;
        let r = radius_at(wy);
        let rim = b.ring(
            tx + (r + 0.02) * toward,
            wy,
            tz + (r + 0.02) * toward,
            0.3,
            0.06,
            Rgba::hex(BONE),
        );
        rim.tilt = [90.0, 0.0, -45.0];
        let pane = b.cylinder(
            tx + (r + 0.03) * toward,
            wy - 0.04,
            tz + (r + 0.03) * toward,
            0.27,
            0.08,
            pane_color,
        );
        pane.tilt = [90.0, 0.0, -45.0];
        if glow {
            pane.emissive = Some(Rgba::hex("#9c7a28"));
        }
    }
    // The outflow: a pipe from the tower's foot to the quay, down into the
    // water.
    pipe(&mut b, tx + 1.0, quay + 0.2, 0.32, tz + 1.05, 0.12);
    b.cylinder(quay + 0.2, -0.4, tz + 1.05, 0.14, 0.86, Rgba::hex(STONE));
    b.ring(
        quay + 0.2,
        0.32,
        tz + 1.05,
        0.15,
        0.04,
        Rgba::hex(STONE_DARK),
    );
    // The janitor, at the tower's foot, between it and the quay.
    caretaker(&mut b, quay - 2.4, tz + 2.2);
    // The bridge from the hangar to the tower.
    b.rounded(
        bx + bw - 0.2,
        1.4,
        tz - 0.9,
        1.9,
        1.3,
        1.8,
        0.15,
        Rgba::hex(BRICK),
    )
    .pattern = Pattern::Brick;
    b.rounded(
        bx + bw + 0.1,
        2.55,
        tz - 0.98,
        1.3,
        0.2,
        1.96,
        0.06,
        Rgba::hex(BONE),
    );
    // A radar dish on the roof's back corner.
    let (dx, dz) = (bx + bw - 0.8, bz + 0.8);
    b.cylinder(dx, bh, dz, 0.04, 0.9, Rgba::hex(STONE));
    let dish = b.cone(dx, bh + 0.9, dz, 0.45, 0.22, Rgba::hex(STONE));
    dish.tilt = [180.0, 0.0, 0.0];
    for i in 0..5 {
        // Windows along the hangar's front, past the door: a cream frame, a
        // pane that glows when the plant is at work, and a cross of
        // mullions — each a step proud of the one behind, toward `+z`.
        let (wx, wy, wz) = (bx + 2.95 + i as f32 * 1.3, 1.25, bz + bd);
        b.rounded(
            wx - 0.36,
            wy,
            wz - 0.02,
            0.72,
            1.0,
            0.08,
            0.05,
            Rgba::hex(BONE),
        );
        let pane = b.rounded(wx - 0.28, wy + 0.08, wz, 0.56, 0.84, 0.08, 0.03, pane_color);
        if glow {
            pane.emissive = Some(Rgba::hex("#9c7a28"));
        }
        b.rounded(
            wx - 0.03,
            wy + 0.08,
            wz + 0.03,
            0.06,
            0.84,
            0.08,
            0.02,
            Rgba::hex(BONE),
        );
        b.rounded(
            wx - 0.28,
            wy + 0.47,
            wz + 0.04,
            0.56,
            0.06,
            0.08,
            0.02,
            Rgba::hex(BONE),
        );
        // A sill under each.
        b.rounded(
            wx - 0.42,
            wy - 0.08,
            wz - 0.02,
            0.84,
            0.08,
            0.16,
            0.03,
            Rgba::hex(BONE),
        );
    }
    // The gantry: two chrome posts on the roof's front edge and the board
    // between them, the name on its face.
    let gz = bz + bd - 0.5;
    for px in [bx + 1.2, bx + bw - 1.2] {
        b.cylinder(px, bh, gz, 0.07, 2.6, Rgba::hex(STONE));
    }
    b.rounded(
        bx + 1.0,
        bh + 1.4,
        gz - 0.1,
        bw - 2.0,
        1.3,
        0.16,
        0.08,
        Rgba::hex(STONE),
    );
    b.rounded(
        bx + 1.1,
        bh + 1.5,
        gz - 0.06,
        bw - 2.2,
        1.1,
        0.16,
        0.06,
        Rgba::hex(RED_DARK),
    );
    for (i, ch) in snap.factory.chimneys.iter().enumerate() {
        // On the roof's back half, behind the vaults and clear of the
        // tower that stands in front of the hangar's right end as the
        // camera sees it.
        let x = bx + bw - 3.3 - i as f32 * 1.9;
        let z = bz + 1.4;
        let h = 2.1 + (i % 2) as f32 * 0.35;
        let hot = pane(
            serde_json::json!({"kind": "chimney", "model": ch.model}),
            &format!(
                "{} chimney\n{} · ${:.2} spent on its stages",
                ch.model,
                if ch.smoking {
                    format!("{} session(s) open", ch.runs)
                } else {
                    "cold".to_string()
                },
                ch.usd
            ),
        );
        chimney(&mut b, x, bh, z, h, &ch.model, ch.smoking, i as f32, &hot);
        b.label(x, bh + h + 1.0, z, &ch.model, 11.0, "#ffffff")
            .backing = Backing::Ink;
    }
    let name = b.label(
        bx + bw / 2.0,
        bh + 2.3,
        gz + 0.12,
        &snap.project.name.to_uppercase(),
        22.0,
        TEXT,
    );
    name.bold = true;
    name.backing = Backing::Ink;
    b.label(
        bx + bw / 2.0,
        bh + 1.75,
        gz + 0.12,
        if snap.project.slug.is_empty() {
            "local checkout"
        } else {
            &snap.project.slug
        },
        11.0,
        DIM,
    )
    .backing = Backing::Ink;
    let go_inside = Hot {
        go: Some("B".to_string()),
        tip: Some("Enter the plant".to_string()),
        ..Hot::default()
    };
    door(
        &mut b,
        bx + 0.5,
        bz + bd + 0.08,
        !snap.factory.idle,
        &go_inside,
    );
    let board_hot = pane(
        serde_json::json!({"kind": "board"}),
        "What is to do, what is done",
    );
    sign(
        &mut b,
        bx + bw - 3.2,
        bz + bd + 1.3,
        3.0,
        "BOARD",
        &board_counts(snap),
        PAPER,
        &board_hot,
    );
    let steward_hot = pane(
        serde_json::json!({"kind": "steward"}),
        "The steward\nA Claude Code terminal in the harness checkout — ask them to start the plant, stop it, read the board.",
    );
    wizard(&mut b, bx - 0.9, bz + bd + 1.4, &steward_hot);
    // The workers stand out on the street, in front of the sign rather than
    // hidden behind it: three to a column across the asphalt, the columns
    // marching toward the quay.
    for (i, e) in snap.employees.iter().enumerate() {
        let x = bx + 5.3 + (i / 3) as f32 * 1.5;
        let z = street_z + 0.6 + (i % 3) as f32 * 0.8;
        person(&mut b, x, z, e.model.as_deref(), &e.name, &employee_hot(e));
    }
    let watch = if snap.factory.watching {
        "● watch polling"
    } else {
        "○ watch off"
    };
    b.label(
        bx + 2.2,
        0.2,
        bz + bd + 4.6,
        watch,
        12.0,
        if snap.factory.watching { OK } else { DIM },
    )
    .backing = Backing::Ink;
    let sub = snap.factory.in_flight.as_ref().map_or_else(
        || {
            snap.factory.saw.as_ref().map_or_else(
                || "quiet".to_string(),
                |s| {
                    format!(
                        "router: {}",
                        s.split(',').take(2).collect::<Vec<_>>().join(",")
                    )
                },
            )
        },
        |f| format!("running {}", f.workflow),
    );
    b.label(bx + 2.2, 0.2, bz + bd + 5.3, &sub, 11.0, SCRIPT);
    b.span([0.0, 0.0, 0.0], [gw, 0.0, gd]);
    b.scene
}

// ---- B · inside -------------------------------------------------------------

fn room_hot(room: &Room) -> Hot {
    Hot {
        go: Some("C".to_string()),
        room: Some(room.key.clone()),
        tip: Some(format!("{} · {}\n{}", room.id, room.name, room.blurb)),
        ..Hot::default()
    }
}

const fn card_color(status: IssueStatus) -> &'static str {
    match status {
        IssueStatus::Ready => "#f2d37a",
        IssueStatus::Todo => CREAM,
        IssueStatus::Blocked => "#cfc6b8",
        IssueStatus::Human => "#f0b2a8",
        IssueStatus::Delivered => "#bfe3c0",
        IssueStatus::Done => "#8ed19a",
    }
}

/// Open tasks of the open milestones, the ones next up first.
fn office_cards(snap: &Snapshot) -> Vec<&Issue> {
    let order = |s: IssueStatus| match s {
        IssueStatus::Ready => 0,
        IssueStatus::Todo => 1,
        IssueStatus::Blocked => 2,
        IssueStatus::Human => 3,
        IssueStatus::Delivered => 4,
        IssueStatus::Done => 5,
    };
    let mut cards: Vec<&Issue> = snap
        .board
        .iter()
        .flat_map(|b| b.milestones.iter())
        .filter(|m| m.issue.state == "open")
        .flat_map(|m| m.tasks.iter())
        .filter(|t| t.status != IssueStatus::Done)
        .collect();
    cards.sort_by_key(|t| (order(t.status), t.number));
    cards
}

#[allow(clippy::too_many_lines)] // One level is one list of props; cutting it in two hides the layout.
fn room_props(b: &mut Builder, snap: &Snapshot, room: &Room, x0: f32, z0: f32, rw: f32) {
    match room.key.as_str() {
        "lines" => {
            for (k, line) in snap.lines.iter().enumerate() {
                let z = z0 + 0.35 + k as f32 * 0.78;
                let (x, len) = (x0 + 1.4, rw - 1.9);
                b.rounded(
                    x,
                    0.0,
                    z,
                    len,
                    0.2,
                    0.5,
                    0.09,
                    Rgba::hex(if line.active { WOOD } else { WOOD_DARK }),
                );
                let active = line
                    .stations
                    .iter()
                    .position(|s| s.state == StationState::Active);
                let done = line.stations.iter().rposition(|s| {
                    matches!(
                        s.state,
                        StationState::Done | StationState::Skipped | StationState::Failed
                    )
                });
                if let Some(at) = active.or(done) {
                    let cx =
                        x + 0.1 + at as f32 / (line.stations.len().max(2) - 1) as f32 * (len - 0.5);
                    b.rounded(
                        cx,
                        0.2,
                        z + 0.08,
                        0.35,
                        0.3,
                        0.35,
                        0.08,
                        Rgba::hex(if line.active { GOLD } else { BONE }),
                    );
                }
            }
        }
        "office" => {
            b.rounded(
                x0 + 0.6,
                0.6,
                z0 - 0.1,
                rw - 1.2,
                1.5,
                0.1,
                0.1,
                Rgba::hex(PAPER),
            );
            for (i, card) in office_cards(snap).iter().take(12).enumerate() {
                let x = x0 + 0.9 + (i % 6) as f32 * 0.95;
                let y = 1.55 - (i / 6) as f32 * 0.65;
                b.rounded(
                    x,
                    y,
                    // Proud of the board's face at `z0`, toward the camera.
                    z0,
                    0.7,
                    0.45,
                    0.04,
                    0.04,
                    Rgba::hex(card_color(card.status)),
                );
            }
            b.rounded(
                x0 + 1.5,
                0.0,
                z0 + 2.6,
                2.6,
                0.8,
                1.1,
                0.12,
                Rgba::hex(WOOD),
            );
            screen(b, x0 + 2.3, z0 + 2.65, 0.8, TEAL, &Hot::default());
            b.capsule(x0 + 2.75, 0.0, z0 + 4.35, 0.3, 0.55, Rgba::hex(STONE_DARK));
            // The product's corner, moved in from the old value room: the
            // mock-up on its easel, the data floppy.
            b.cylinder(x0 + 5.0, 0.0, z0 + 2.5, 0.05, 0.9, Rgba::hex(WOOD));
            b.cylinder(x0 + 6.3, 0.0, z0 + 2.5, 0.05, 0.9, Rgba::hex(WOOD));
            b.rounded(
                x0 + 4.8,
                0.7,
                z0 + 2.4,
                1.7,
                1.1,
                0.1,
                0.1,
                Rgba::hex(CREAM),
            );
            b.cylinder(x0 + 5.65, 0.0, z0 + 4.4, 0.45, 0.14, Rgba::hex(TEAL));
        }
        "store" => {
            let v = &snap.versions;
            let mut items: Vec<(String, &str)> = vec![
                ("main".to_string(), GOLD),
                (v.integration.name.clone(), TEAL),
            ];
            items.extend(v.milestones.iter().map(|m| (m.name.clone(), "#b78cf0")));
            for (i, (_, color)) in items.iter().take(6).enumerate() {
                let x = x0 + 0.6 + (i % 3) as f32 * 2.1;
                let z = z0 + 0.5 + (i / 3) as f32 * 2.6;
                b.rounded(x, 0.0, z, 1.6, 0.15, 1.0, 0.06, Rgba::hex(WOOD));
                crate_box(b, x + 0.5, 0.15, z + 0.25, color, None, None);
            }
        }
        "infirmary" => {
            let hot = doctor_hot(
                "The doctor\nA Claude Code that examines the plant — the check-up is asked the moment they sit down.",
            );
            bed(b, x0 + 0.7, z0 + 1.0, 2.2, &hot);
            cabinet(b, x0 + 4.9, z0 + 0.1, 1.1, 1.6, &hot);
            physician(b, x0 + 4.4, z0 + 3.3, &hot);
        }
        "control" => {
            b.rounded(
                x0 + 1.2,
                0.0,
                z0 + 3.2,
                4.5,
                0.7,
                1.0,
                0.14,
                Rgba::hex(WOOD_DARK),
            );
            for i in 0..3 {
                screen(
                    b,
                    x0 + 1.25 + i as f32 * 1.45,
                    z0 + 3.2,
                    0.7,
                    if i == 1 { WARN } else { TEAL },
                    &Hot::default(),
                );
            }
        }
        _ => {
            barrier(b, x0 + 1.0, z0 + 1.5, 4.5);
            cone(b, x0 + 2.0, z0 + 3.5);
            cone(b, x0 + 4.5, z0 + 4.2);
        }
    }
}

fn interior(snap: &Snapshot) -> Scene {
    let mut b = Builder::new(HALL);
    let (rw, rd, gap) = (7.0, 6.0, 1.2);
    let (w, d) = (3.0 * rw + 4.0 * gap, 2.0 * rd + 3.0 * gap);
    b.floor(w, d);
    for (i, room) in snap.rooms.iter().enumerate() {
        let (col, row) = ((i % 3) as f32, (i / 3) as f32);
        let x0 = gap + col * (rw + gap);
        let z0 = gap + row * (rd + gap);
        let color = match room.status {
            RoomStatus::Construction => TILE_RAW,
            RoomStatus::Draft => TILE_DRAFT,
            RoomStatus::Live => TILE_LIVE,
        };
        let hot = room_hot(room);
        let m = b.mark();
        b.slab(x0, -0.02, z0, rw, 0.04, rd, Rgba::hex(color))
            .pattern = Pattern::Pavers;
        // Low brick partitions on the room's two back sides.
        b.rounded(x0, 0.0, z0 - 0.15, rw, 0.6, 0.15, 0.04, Rgba::hex(BRICK))
            .pattern = Pattern::Brick;
        b.rounded(x0 - 0.15, 0.0, z0, 0.15, 0.6, rd, 0.04, Rgba::hex(BRICK))
            .pattern = Pattern::Brick;
        b.hot_since(m, &hot);
        room_props(&mut b, snap, room, x0, z0, rw);
        let active = room.key == "lines" && !snap.employees.is_empty();
        let color = if active {
            ACCENT
        } else if room.status == RoomStatus::Construction {
            WARN
        } else {
            TEXT
        };
        let label = b.label(
            x0 + rw / 2.0,
            0.1,
            z0 + rd + 0.4,
            &format!("{} · {}", room.id, room.name),
            12.0,
            color,
        );
        label.backing = Backing::Ink;
        label.bold = true;
    }
    let steward_hot = pane(
        serde_json::json!({"kind": "steward"}),
        "The steward\nA Claude Code terminal in the harness checkout.",
    );
    wizard(&mut b, gap + rw + 0.1, gap + 1.4, &steward_hot);
    for (i, e) in snap.employees.iter().enumerate() {
        let k = snap
            .lines
            .iter()
            .position(|l| l.id == e.workflow)
            .unwrap_or(0);
        // Side by side, not stacked: a tenth of a tile apart they read as one.
        person(
            &mut b,
            gap + 0.2 + (i % 4) as f32 * 0.9,
            gap + 0.3 + k as f32 * 0.78,
            e.model.as_deref(),
            &e.name,
            &employee_hot(e),
        );
    }
    b.scene
}

// ---- C · a room ---------------------------------------------------------------

const fn kind_tip(kind: Kind) -> &'static str {
    match kind {
        Kind::Scanner => "gate · a deterministic check",
        Kind::Builder => "LLM that writes",
        Kind::Inspector => "LLM that reads and judges",
        Kind::Printer => "gathers the context",
        Kind::Arm => "acts on the code or on GitHub",
    }
}

const fn state_name(state: StationState) -> &'static str {
    match state {
        StationState::Idle => "idle",
        StationState::Active => "active",
        StationState::Done => "done",
        StationState::Skipped => "skipped",
        StationState::Failed => "failed",
    }
}

#[allow(clippy::too_many_lines)] // One level is one list of props; cutting it in two hides the layout.
fn lines_room(snap: &Snapshot) -> Scene {
    let mut b = Builder::new(CONCRETE);
    let (sp, row, x0) = (STATION_SPACING, LINE_SPACING, LINE_X0);
    let max_n = snap
        .lines
        .iter()
        .map(|l| l.stations.len())
        .max()
        .unwrap_or(1)
        .max(1);
    // Deep enough for the last band and the walk in front of its belt.
    let (w, d) = (
        x0 + max_n as f32 * sp + 1.5,
        FIRST_LINE_Z + (snap.lines.len().max(1) - 1) as f32 * row + 3.0,
    );
    b.floor(w, d);
    for (i, line) in snap.lines.iter().enumerate() {
        let z = FIRST_LINE_Z + i as f32 * row;
        let len = line.stations.len() as f32 * sp + 0.5;
        let line_hot = pane(
            serde_json::json!({"kind": "line", "id": line.id}),
            &format!("{}\n{}", line.title, line.trigger),
        );
        let runs = format!(
            "{} run{}{}",
            line.runs,
            if line.runs == 1 { "" } else { "s" },
            line.last_run
                .as_ref()
                .and_then(|r| r.age_secs)
                .map_or_else(String::new, |a| format!(" · {} ago", age(a)))
        );
        // Over the line's head block: the title with the run count under it,
        // in one pill — two pills overlap once the room is deep enough for
        // the camera to pull back.
        let title = b.label(
            1.15,
            0.9,
            z + 0.5,
            &format!("{}\n{runs}", line.title),
            11.0,
            if line.active { ACCENT } else { TEXT },
        );
        title.backing = Backing::Ink;
        title.bold = true;
        let m = b.mark();
        b.rounded(0.0, 0.0, z, 2.3, 0.35, 1.0, 0.12, Rgba::hex(WOOD_DARK));
        b.hot_since(m, &line_hot);
        conveyor(&mut b, x0, z, len, line.active, &line.id);
        for (k, st) in line.stations.iter().enumerate() {
            let x = x0 + 0.3 + k as f32 * sp;
            let act = st.state == StationState::Active;
            let hot = pane(
                serde_json::json!({"kind": "station", "line": line.id, "id": st.id}),
                &format!(
                    "{}\n{}{}\n{}",
                    st.label,
                    kind_tip(st.kind),
                    st.model
                        .as_ref()
                        .map_or_else(String::new, |m| format!(" · {m}")),
                    state_name(st.state)
                ),
            );
            // What became of the station, as a mark before its stage name —
            // a separate pill over the belt collided with the line's title.
            let mark = match st.state {
                StationState::Done => "✓ ",
                StationState::Skipped => "– ",
                StationState::Failed => "✗ ",
                StationState::Idle | StationState::Active => "",
            };
            if st.kind == Kind::Scanner {
                scanner(&mut b, x, z, act, &hot);
                if !mark.is_empty() {
                    // A gate has no stage name to carry the mark: over the arch.
                    let label = b.label(x + 0.5, 2.1, z + 0.5, mark.trim_end(), 12.0, DIM);
                    label.backing = Backing::Ink;
                    label.bold = true;
                }
            } else {
                let zb = z - 1.65;
                match st.kind {
                    Kind::Builder => robot(&mut b, x, zb, true, act, st.model.as_deref(), &hot),
                    Kind::Inspector => robot(&mut b, x, zb, false, act, st.model.as_deref(), &hot),
                    Kind::Printer => printer(&mut b, x, zb, act, &hot),
                    Kind::Arm | Kind::Scanner => arm(&mut b, x, zb, act, &hot),
                }
                b.label(
                    x + 0.5,
                    -0.1,
                    z + 1.3,
                    &format!("{mark}{}", st.stage.as_deref().unwrap_or(&st.label)),
                    10.0,
                    if act { RED_DARK } else { SCRIPT },
                );
            }
        }
        let active_idx = line
            .stations
            .iter()
            .position(|s| s.state == StationState::Active);
        let done_idx = line
            .stations
            .iter()
            .rposition(|s| matches!(s.state, StationState::Done | StationState::Skipped));
        if let Some(target) = active_idx.or(done_idx) {
            let tx = x0 + 0.3 + target as f32 * sp + 0.25;
            crate_box(
                &mut b,
                tx,
                0.3,
                z + 0.25,
                if line.active { GOLD } else { BONE },
                Some(&format!("{}:product", line.id)),
                Some(tx),
            );
        }
        // Everyone at work on this line, by station: a parallel watch puts
        // several lanes on one line, and many of them may stand at the same
        // station. A crew of one is its own figure and opens its own pane; a
        // crew of several is one figure with one label — fanned out, their
        // labels stacked into an unreadable column — and opens the pane that
        // lets you pick one.
        let crew: Vec<&Employee> = snap
            .employees
            .iter()
            .filter(|e| e.workflow == line.id)
            .collect();
        for (k, station) in line.stations.iter().enumerate() {
            let here: Vec<&Employee> = crew
                .iter()
                .copied()
                .filter(|e| {
                    e.station.as_ref() == Some(&station.id) || (e.station.is_none() && k == 0)
                })
                .collect();
            let Some(first) = here.first() else {
                continue;
            };
            // In front of the belt, between their station and the next, so
            // their card covers neither stage name.
            let x = x0 + 0.3 + k as f32 * sp + 0.8;
            if here.len() == 1 {
                person(
                    &mut b,
                    x,
                    z + 1.3,
                    first.model.as_deref(),
                    &first.name,
                    &employee_hot(first),
                );
                continue;
            }
            let hot = pane(
                serde_json::json!({"kind": "crew", "line": line.id, "station": station.id}),
                &format!(
                    "{} at work here — click to pick one:\n{}",
                    here.len(),
                    here.iter()
                        .map(|m| m.name.as_str())
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            );
            elder(&mut b, x, z + 1.3, &crew_label(&line.title, &here), &hot);
        }
    }
    b.scene
}

/// One label for a crew of several: its members' issues (`#20 #21`) — or,
/// when no name carries one, how many on which line.
fn crew_label(line_title: &str, crew: &[&Employee]) -> String {
    let issues: Vec<&str> = crew
        .iter()
        .filter_map(|e| e.name.split_whitespace().find(|w| w.starts_with('#')))
        .collect();
    if issues.is_empty() {
        format!("{} × {line_title}", crew.len())
    } else {
        issues.join(" ")
    }
}

fn age(secs: u64) -> String {
    match secs {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h {}m", s / 3600, (s % 3600) / 60),
        s => format!("{}d", s / 86_400),
    }
}

fn office_room(snap: &Snapshot) -> Scene {
    let mut b = Builder::new(CONCRETE);
    b.floor(16.0, 9.0);
    b.rounded(0.6, 0.7, -0.22, 12.4, 2.6, 0.14, 0.14, Rgba::hex(PAPER));
    b.label(
        6.8,
        3.8,
        -0.07,
        "ISSUE BOARD · open tasks of the open milestones",
        11.0,
        TEXT,
    )
    .backing = Backing::Ink;
    let cards = office_cards(snap);
    if cards.is_empty() {
        b.label(
            6.8,
            2.0,
            -0.07,
            if snap.board.is_some() {
                "nothing open"
            } else {
                "no GitHub board"
            },
            12.0,
            SCRIPT,
        )
        .backing = Backing::Slate;
    }
    for (i, card) in cards.iter().take(24).enumerate() {
        let x = 0.9 + (i % 8) as f32 * 1.5;
        let y = 2.65 - (i / 8) as f32 * 0.75;
        let hot = pane(
            serde_json::json!({"kind": "issue", "number": card.number}),
            &format!(
                "#{} {}\n{}",
                card.number,
                card.title,
                issue_status_name(card.status)
            ),
        );
        let m = b.mark();
        b.rounded(
            x,
            y,
            // Proud of the board's face at -0.08, toward the camera; the pin
            // proud of the card.
            -0.08,
            1.25,
            0.6,
            0.05,
            0.05,
            Rgba::hex(card_color(card.status)),
        );
        b.sphere(x + 0.62, y + 0.6, -0.01, 0.05, Rgba::hex(RED));
        b.hot_since(m, &hot);
        b.label(
            x + 0.62,
            y + 0.3,
            // On the card's face, or the number drifts up and left of it.
            -0.02,
            &format!("#{}", card.number),
            10.0,
            "#111827",
        )
        .bold = true;
    }
    let desk_hot = pane(
        serde_json::json!({"kind": "placeholder", "title": "Talk to Claude Code", "text": "Here you will build the need with Claude Code: pick an issue on the board, discuss it, let the refinement write the SPEC. Not wired yet — the office shows the issues read-only for now."}),
        "Claude Code desk · coming",
    );
    let m = b.mark();
    b.rounded(3.5, 0.0, 4.0, 3.2, 0.8, 1.2, 0.14, Rgba::hex(WOOD));
    b.hot_since(m, &desk_hot);
    screen(&mut b, 4.4, 4.05, 0.8, TEAL, &desk_hot);
    b.capsule(5.05, 0.0, 6.05, 0.32, 0.55, Rgba::hex(STONE_DARK));
    let board_hot = pane(serde_json::json!({"kind": "board"}), "The board, as a list");
    let m = b.mark();
    b.rounded(8.5, 0.0, 4.0, 3.2, 0.8, 1.2, 0.14, Rgba::hex(WOOD));
    b.hot_since(m, &board_hot);
    b.label(10.1, 1.1, 4.6, "BOARD", 11.0, TEXT).backing = Backing::Ink;
    product_corner(&mut b);
    b.scene
}

/// The product's corner of the office — what the old value room held: the
/// mock-up on its easel, the feature list on a screen, the data floppy.
fn product_corner(b: &mut Builder) {
    let mock = pane(
        serde_json::json!({"kind": "placeholder", "title": "The mock-up", "text": "The interface, drawn as micro-frontends: click a part, read the feature behind it. Nothing to show yet — the product's mock-up has no source the view can read."}),
        "The mock-up of the interface",
    );
    let m = b.mark();
    b.cylinder(1.15, 0.0, 6.35, 0.06, 1.15, Rgba::hex(WOOD));
    b.cylinder(3.25, 0.0, 6.35, 0.06, 1.15, Rgba::hex(WOOD));
    b.rounded(0.8, 1.0, 6.2, 2.8, 2.0, 0.14, 0.14, Rgba::hex(CREAM));
    b.hot_since(m, &mock);
    let head = b.label(2.2, 2.0, 6.35, "MOCK-UP", 12.0, INK);
    head.bold = true;
    head.backing = Backing::Slate;
    b.label(2.2, 1.6, 6.35, "micro-frontends · coming", 10.0, SCRIPT)
        .backing = Backing::Slate;
    let features = pane(serde_json::json!({"kind": "features"}), "The feature list");
    // On the row of the other two desks, to their right.
    let m = b.mark();
    b.rounded(12.5, 0.0, 4.0, 2.6, 0.8, 1.2, 0.14, Rgba::hex(WOOD));
    b.hot_since(m, &features);
    screen(b, 13.3, 4.05, 0.8, "#a78bfa", &features);
    let data = pane(
        serde_json::json!({"kind": "placeholder", "title": "Data sources", "text": "Where the product reads from and writes to. Not declared anywhere the view can read yet."}),
        "The data sources",
    );
    let m = b.mark();
    b.cylinder(11.0, 0.0, 7.0, 0.65, 0.16, Rgba::hex(TEAL));
    b.cylinder(11.0, 0.16, 7.0, 0.22, 0.05, Rgba::hex(BONE));
    b.hot_since(m, &data);
    b.label(11.0, 0.7, 7.0, "DATA", 10.0, TEXT).backing = Backing::Ink;
}

const fn issue_status_name(status: IssueStatus) -> &'static str {
    match status {
        IssueStatus::Done => "done",
        IssueStatus::Delivered => "delivered",
        IssueStatus::Human => "human",
        IssueStatus::Blocked => "blocked",
        IssueStatus::Ready => "ready",
        IssueStatus::Todo => "todo",
    }
}

fn url_hot(url: &str, tip_text: &str) -> Option<Hot> {
    (!url.is_empty()).then(|| Hot {
        url: Some(url.to_string()),
        tip: Some(format!("{tip_text}\nopens GitHub ↗")),
        ..Hot::default()
    })
}

fn store_room(snap: &Snapshot) -> Scene {
    let mut b = Builder::new(CONCRETE);
    b.floor(15.0, 9.0);
    let v = &snap.versions;
    let versions_pane = pane(
        serde_json::json!({"kind": "versions"}),
        "Every version on sale",
    );
    shelf(
        &mut b,
        0.8,
        0.3,
        "RELEASE",
        &[(
            v.main.name.clone(),
            GOLD.to_string(),
            url_hot(&v.main.url, &format!("{}\n{}", v.main.name, v.main.label)),
        )],
        &versions_pane,
    );
    shelf(
        &mut b,
        5.6,
        0.3,
        "AGENTS",
        &[(
            v.integration.name.clone(),
            TEAL.to_string(),
            url_hot(
                &v.integration.url,
                &format!("{}\n{}", v.integration.name, v.integration.label),
            ),
        )],
        &versions_pane,
    );
    let milestones: Vec<(String, String, Option<Hot>)> = v
        .milestones
        .iter()
        .map(|m| {
            let short = m
                .name
                .strip_prefix("milestone/")
                .and_then(|rest| rest.split('-').next())
                .map_or_else(|| m.name.clone(), |n| format!("#{n}"));
            (
                short,
                "#b78cf0".to_string(),
                url_hot(&m.url, &format!("{}\n{}", m.name, m.label)),
            )
        })
        .collect();
    shelf(&mut b, 10.4, 0.3, "MILESTONES", &milestones, &versions_pane);
    let m = b.mark();
    b.rounded(4.5, 0.0, 5.5, 6.0, 1.0, 1.1, 0.2, Rgba::hex(RED));
    b.rounded(4.4, 0.95, 5.4, 6.2, 0.12, 1.3, 0.06, Rgba::hex(CREAM));
    b.hot_since(m, &versions_pane);
    b.label(7.5, 1.4, 6.0, "COUNTER · every version", 11.0, TEXT)
        .backing = Backing::Ink;
    b.scene
}

fn infirmary_room(_snap: &Snapshot) -> Scene {
    let mut b = Builder::new(CONCRETE);
    b.floor(14.0, 9.0);
    // The chart on the back wall: what last stopped the plant, as the
    // control room's stops screen.
    let chart = pane(
        serde_json::json!({"kind": "dashboards", "focus": "errors"}),
        "The patient's chart\nWhy the harness last stopped — the stops dashboard.",
    );
    let m = b.mark();
    b.rounded(0.8, 0.7, -0.22, 5.4, 2.4, 0.14, 0.14, Rgba::hex(PAPER));
    b.hot_since(m, &chart);
    let head = b.label(3.5, 2.6, -0.07, "PATIENT CHART", 12.0, INK);
    head.bold = true;
    head.backing = Backing::Slate;
    b.label(
        3.5,
        2.15,
        -0.07,
        "the last stops · errors.tsv",
        10.0,
        SCRIPT,
    )
    .backing = Backing::Slate;
    b.label(3.5, 1.5, -0.07, "click to read", 10.0, SCRIPT)
        .backing = Backing::Slate;
    let doctor = doctor_hot(
        "The doctor\nA Claude Code in the harness checkout that examines the plant: pulse, chart, `harness doctor --dry-run`, the workspaces — asked the moment they sit down.",
    );
    bed(
        &mut b,
        1.0,
        4.0,
        3.0,
        &doctor_hot(
            "The examination bed\nLie the plant down: the doctor reads its chart and says what is wrong.",
        ),
    );
    let m = b.mark();
    b.rounded(5.6, 0.0, 4.4, 3.2, 0.8, 1.2, 0.14, Rgba::hex(WOOD));
    b.rounded(7.9, 0.8, 4.8, 0.55, 0.03, 0.75, 0.02, Rgba::hex(CREAM));
    b.hot_since(m, &doctor);
    screen(&mut b, 6.3, 4.45, 0.8, OK, &doctor);
    b.capsule(7.2, 0.0, 6.4, 0.32, 0.55, Rgba::hex(STONE_DARK));
    cabinet(
        &mut b,
        7.6,
        0.3,
        1.6,
        2.2,
        &doctor_hot(
            "The remedies\n`harness doctor`: the repair of what a failed run left behind. The doctor opens it only on your go.",
        ),
    );
    physician(&mut b, 10.4, 5.2, &doctor);
    sign(
        &mut b,
        10.4,
        0.8,
        3.0,
        "INFIRMARY",
        "harness doctor · check-up",
        GOLD,
        &chart,
    );
    b.scene
}

fn kfmt(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1e6)
    } else if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

fn control_room(snap: &Snapshot) -> Scene {
    let mut b = Builder::new(CONCRETE);
    b.floor(14.0, 9.0);
    let c = &snap.costs;
    let costs_pane = pane(
        serde_json::json!({"kind": "dashboards", "focus": "costs"}),
        "Spending — the full dashboard",
    );
    let m = b.mark();
    b.rounded(2.5, 0.8, -0.2, 9.0, 2.6, 0.14, 0.14, Rgba::hex(PLUM));
    b.hot_since(m, &costs_pane);
    // Written on the panel: grey plates rather than ink pills, the lines
    // centred on the panel's middle height. The panel leans away on screen,
    // so a long line never fits inside it; the plate carries it.
    let total = b.label(
        7.0,
        2.85,
        -0.05,
        &format!("${:.2} · {} sessions", c.total_usd, c.sessions),
        18.0,
        ACCENT,
    );
    total.bold = true;
    total.backing = Backing::Slate;
    b.label(
        7.0,
        2.3,
        -0.05,
        &format!(
            "{} in · {} out · cache {} read / {} write",
            kfmt(c.tokens.input + c.tokens.cache_read),
            kfmt(c.tokens.output),
            kfmt(c.tokens.cache_read),
            kfmt(c.tokens.cache_write)
        ),
        11.0,
        DIM,
    )
    .backing = Backing::Slate;
    if let Some(q) = &snap.quota {
        for (i, w) in q.windows.iter().enumerate() {
            // Utilization is 0..=1, so this is 0..=100.
            #[allow(clippy::cast_possible_truncation)]
            let pct = (w.utilization * 100.0).round() as i64;
            let color = if pct > 90 {
                BAD
            } else if pct > 70 {
                WARN
            } else {
                OK
            };
            b.label(
                7.0,
                1.85 - i as f32 * 0.4,
                -0.05,
                &format!("{} {pct}% used", w.name),
                11.0,
                color,
            )
            .backing = Backing::Slate;
        }
    }
    let screens = [
        ("costs", TEAL, "Spending by day, stage and task"),
        ("quota", WARN, "Rate-limit windows"),
        ("errors", BAD, "The last stops"),
        ("journal", OK, "The watch journal"),
    ];
    for (i, (focus, color, tip_text)) in screens.iter().enumerate() {
        let hot = pane(
            serde_json::json!({"kind": "dashboards", "focus": focus}),
            tip_text,
        );
        screen(&mut b, 2.6 + i as f32 * 2.4, 4.4, 0.7, color, &hot);
    }
    b.rounded(2.4, 0.0, 4.4, 9.8, 0.7, 1.0, 0.16, Rgba::hex(WOOD_DARK));
    b.scene
}

fn construction_room(_snap: &Snapshot) -> Scene {
    let mut b = Builder::new(TILE_RAW);
    b.floor(12.0, 8.0);
    barrier(&mut b, 1.5, 2.5, 5.0);
    barrier(&mut b, 6.5, 3.5, 4.0);
    cone(&mut b, 3.0, 5.0);
    cone(&mut b, 7.5, 5.5);
    let hot = pane(
        serde_json::json!({"kind": "placeholder", "title": "Room 6", "text": "Nothing is planned for this room yet."}),
        "Nothing here yet",
    );
    sign(
        &mut b,
        4.5,
        0.8,
        3.4,
        "UNDER CONSTRUCTION",
        "room 6 · nothing here yet",
        GOLD,
        &hot,
    );
    b.scene
}

/// Everything to draw for this picture, from this viewpoint.
#[must_use]
pub fn build(snap: &Snapshot, view: &View) -> Scene {
    match view {
        View::A => factory(snap),
        View::B => interior(snap),
        View::C { room } => match room.as_str() {
            "lines" => lines_room(snap),
            "office" => office_room(snap),
            "store" => store_room(snap),
            "infirmary" => infirmary_room(snap),
            "control" => control_room(snap),
            _ => construction_room(snap),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PICTURE: &str = r#"{
      "project": {"name": "dnd_helper", "slug": "Laucans/dnd_helper", "url": "https://github.com/Laucans/dnd_helper"},
      "factory": {"chimneys": [{"model": "opus", "smoking": true, "runs": 1, "usd": 9.8}, {"model": "sonnet", "smoking": false, "runs": 0, "usd": 45.7}],
                  "watching": false, "idle": false, "saw": "1 roadmap, 6 milestone(s)", "in_flight": null},
      "employees": [{"id": "agent-loop/20261007-142103", "name": "Dev loop · #66", "workflow": "agent-loop",
                     "station": "technical-refinement", "stage": "technical-refinement", "model": "opus", "round": "1/3",
                     "age_secs": 40, "last_line": "→ Bash: cargo test", "active": true}],
      "lines": [{"id": "agent-loop", "title": "Dev loop", "trigger": "harness:ready on a task", "runs": 287, "active": true,
                 "last_run": {"age_secs": 40},
                 "stations": [
                   {"id": "pick", "label": "pick the task", "kind": "printer", "stage": "pick", "state": "done"},
                   {"id": "technical-refinement.pre", "label": "requires", "kind": "scanner", "state": "idle"},
                   {"id": "technical-refinement", "label": "technical refinement", "kind": "builder", "stage": "technical-refinement", "model": "opus", "state": "active"},
                   {"id": "technical-refinement.post", "label": "must achieve", "kind": "scanner", "state": "idle"},
                   {"id": "code", "label": "code", "kind": "builder", "stage": "code", "model": "sonnet", "state": "idle"},
                   {"id": "deliver", "label": "mark delivered", "kind": "arm", "stage": "deliver", "state": "idle"}]},
                {"id": "split", "title": "Split", "trigger": "harness:ready on a milestone", "runs": 1, "active": false,
                 "stations": [{"id": "slice", "label": "slice", "kind": "inspector", "stage": "slice", "model": "sonnet", "state": "skipped"}]}],
      "rooms": [{"id": 1, "key": "lines", "name": "Assembly lines", "blurb": "b", "status": "live"},
                {"id": 2, "key": "office", "name": "Architecture office", "blurb": "b", "status": "draft"},
                {"id": 3, "key": "store", "name": "Distribution", "blurb": "b", "status": "live"},
                {"id": 4, "key": "infirmary", "name": "Infirmary", "blurb": "b", "status": "live"},
                {"id": 5, "key": "control", "name": "Control room", "blurb": "b", "status": "live"},
                {"id": 6, "key": "construction", "name": "Under construction", "blurb": "b", "status": "construction"}],
      "board": {"milestones": [{"issue": {"number": 17, "title": "Dernier", "state": "open"},
                                "tasks": [{"number": 61, "title": "a", "state": "open", "status": "delivered"},
                                          {"number": 66, "title": "b", "state": "open", "status": "ready"},
                                          {"number": 67, "title": "c", "state": "open", "status": "human"},
                                          {"number": 62, "title": "d", "state": "closed", "status": "done"}]}]},
      "versions": {"main": {"name": "main", "url": "https://github.com/x/tree/main", "label": "the current version"},
                   "integration": {"name": "main_agent", "url": "https://github.com/x/tree/main_agent", "label": "agents"},
                   "milestones": [{"name": "milestone/17-dernier", "url": "https://github.com/x/tree/milestone/17-dernier", "label": "Dernier"}]},
      "costs": {"total_usd": 58.09, "sessions": 316, "tokens": {"input": 1000, "output": 1000000, "cache_read": 107000000, "cache_write": 4100000}},
      "quota": {"windows": [{"name": "five_hour", "utilization": 0.58}, {"name": "seven_day", "utilization": 0.9}]}
    }"#;

    fn picture() -> Snapshot {
        Snapshot::parse(PICTURE).expect("the fixture parses")
    }

    fn hots(scene: &Scene) -> Vec<&Hot> {
        scene.props.iter().filter_map(|p| p.hot.as_ref()).collect()
    }

    fn pane_kind(hot: &Hot) -> Option<&str> {
        hot.pane
            .as_ref()
            .and_then(|p| p.get("kind"))
            .and_then(|k| k.as_str())
    }

    fn panes(scene: &Scene, kind: &str) -> usize {
        hots(scene)
            .iter()
            .filter(|h| pane_kind(h) == Some(kind))
            .count()
    }

    fn shapes(scene: &Scene) -> Vec<&'static str> {
        scene
            .props
            .iter()
            .map(|p| match p.shape {
                Shape::Rounded { .. } => "rounded",
                Shape::Cuboid { .. } => "cuboid",
                Shape::Cylinder { .. } => "cylinder",
                Shape::Cone { .. } => "cone",
                Shape::Frustum { .. } => "frustum",
                Shape::Capsule { .. } => "capsule",
                Shape::Sphere { .. } => "sphere",
                Shape::Torus { .. } => "torus",
                Shape::Card { .. } => "card",
            })
            .collect()
    }

    fn cards(scene: &Scene) -> Vec<Figure> {
        scene
            .props
            .iter()
            .filter_map(|p| match p.shape {
                Shape::Card { figure, .. } => Some(figure),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_figure_s_shapes_share_one_group_and_figures_differ() {
        let scene = build(&picture(), &View::A);
        let door: Vec<u32> = scene
            .props
            .iter()
            .filter(|p| p.hot.as_ref().and_then(|h| h.go.as_deref()) == Some("B"))
            .filter_map(|p| p.group)
            .collect();
        assert_eq!(door.len(), 11, "ring, eight teeth, leaf and hub");
        assert!(
            door.iter().all(|g| *g == door[0]),
            "the cog grows as one: {door:?}"
        );
        let steward: Vec<u32> = scene
            .props
            .iter()
            .filter(|p| p.hot.as_ref().and_then(pane_kind) == Some("steward"))
            .filter_map(|p| p.group)
            .collect();
        assert_eq!(steward.len(), 2, "card and shadow");
        assert_eq!(steward[0], steward[1], "card and shadow grow as one");
        assert_ne!(steward[0], door[0], "but not with the door");
        assert!(
            scene
                .props
                .iter()
                .all(|p| p.hot.is_some() == p.group.is_some()),
            "every hot prop belongs to a figure, and only those"
        );
    }

    #[test]
    fn no_two_slabs_share_a_top_face_where_they_overlap() {
        // Two coplanar faces flicker against each other; the road once lay
        // flush with the ground.
        for view in [
            View::A,
            View::B,
            View::C {
                room: "lines".to_string(),
            },
        ] {
            let scene = build(&picture(), &view);
            let slabs: Vec<([f32; 3], f32, f32, f32)> = scene
                .props
                .iter()
                .filter_map(|p| match p.shape {
                    Shape::Cuboid { w, h, d } => Some((p.at, w, h, d)),
                    _ => None,
                })
                .collect();
            for (i, one) in slabs.iter().enumerate() {
                for other in slabs.iter().skip(i + 1) {
                    let overlap = (one.0[0] - other.0[0]).abs() < f32::midpoint(one.1, other.1)
                        && (one.0[2] - other.0[2]).abs() < f32::midpoint(one.3, other.3);
                    let top = one.0[1] + one.2 / 2.0;
                    let other_top = other.0[1] + other.2 / 2.0;
                    assert!(
                        !overlap || (top - other_top).abs() > 0.005,
                        "{view:?}: two slabs share a top at y={top}: {one:?} {other:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_screens_stand_on_their_desks_not_in_them() {
        // A screen's stand is the one wide, flat cylinder in these rooms,
        // and its base sits on the desk's top.
        for (room, desk_top) in [("office", 0.8), ("infirmary", 0.8), ("control", 0.7)] {
            let scene = build(
                &picture(),
                &View::C {
                    room: room.to_string(),
                },
            );
            let bases: Vec<f32> = scene
                .props
                .iter()
                .filter_map(|p| match p.shape {
                    Shape::Cylinder { r, h } if (r - 0.26).abs() < 1e-6 => Some(p.at[1] - h / 2.0),
                    _ => None,
                })
                .collect();
            assert!(!bases.is_empty(), "{room}: a screen");
            for base in bases {
                assert!(
                    (base - desk_top).abs() < 1e-5,
                    "{room}: a stand at {base}, the desk top at {desk_top}"
                );
            }
        }
    }

    #[test]
    fn the_lines_room_gives_each_line_a_band_and_keeps_the_robots_off_the_wall() {
        let scene = build(
            &picture(),
            &View::C {
                room: "lines".to_string(),
            },
        );
        // Two lines: deep enough for both bands and the walk in front.
        assert!(
            scene.max[2] >= FIRST_LINE_Z + LINE_SPACING + 2.5,
            "{}",
            scene.max[2]
        );
        for prop in &scene.props {
            if let Shape::Card { h, .. } = prop.shape {
                let top_z = prop.at[2] + CAMERA_UP[2] * h / 2.0;
                assert!(top_z > 0.3, "{prop:?} leans into the back wall");
            }
        }
        let mut heads: Vec<f32> = scene
            .props
            .iter()
            .filter(|p| {
                matches!(p.shape, Shape::Rounded { w, h, d, .. }
                    if (w - 2.3).abs() < 1e-6 && (h - 0.35).abs() < 1e-6 && (d - 1.0).abs() < 1e-6)
            })
            .map(|p| p.at[2])
            .collect();
        heads.sort_by(f32::total_cmp);
        assert_eq!(heads.len(), 2, "one head block per line");
        assert!(
            (heads[1] - heads[0] - LINE_SPACING).abs() < 1e-5,
            "the lines are a full band apart: {heads:?}"
        );
    }

    #[test]
    fn text_written_on_a_board_sits_on_a_grey_plate_and_a_tag_on_ink() {
        let backing = |scene: &Scene, text: &str| {
            scene
                .labels
                .iter()
                .find(|l| l.text == text)
                .map(|l| l.backing)
        };
        let outside = build(&picture(), &View::A);
        assert_eq!(
            backing(&outside, "BOARD"),
            Some(Backing::Slate),
            "on the sign"
        );
        assert_eq!(
            backing(&outside, "the steward"),
            Some(Backing::Ink),
            "a tag over a figure"
        );
        let control = build(
            &picture(),
            &View::C {
                room: "control".to_string(),
            },
        );
        assert_eq!(
            backing(&control, "$58.09 · 316 sessions"),
            Some(Backing::Slate)
        );
        assert_eq!(
            backing(&control, "seven_day 90% used"),
            Some(Backing::Slate)
        );
        let lines = build(
            &picture(),
            &View::C {
                room: "lines".to_string(),
            },
        );
        assert_eq!(
            backing(&lines, "✓ pick"),
            Some(Backing::None),
            "a stage name written on the ground"
        );
    }

    #[test]
    fn two_workers_at_one_station_are_one_figure_that_opens_the_crew_pane() {
        let mut snap = picture();
        let mut second = snap.employees[0].clone();
        second.id = "agent-loop/20261007-150000".to_string();
        second.name = "Dev loop · #67".to_string();
        snap.employees.push(second);
        let scene = build(
            &snap,
            &View::C {
                room: "lines".to_string(),
            },
        );
        let cards = |figure: Figure| {
            scene
                .props
                .iter()
                .filter(|p| matches!(p.shape, Shape::Card { figure: f, .. } if f == figure))
                .count()
        };
        assert_eq!(cards(Figure::Elder), 1, "one figure, the elder");
        assert!(
            scene.props.iter().all(|p| !matches!(
                p.shape,
                Shape::Card {
                    figure: Figure::Follower { .. },
                    ..
                }
            )),
            "no follower stands for a crew member"
        );
        assert!(
            scene.labels.iter().any(|l| l.text == "#66 #67"),
            "one label: each member's issue"
        );
        assert_eq!(
            panes(&scene, "crew"),
            2,
            "the card and its shadow open the crew pane"
        );
        assert_eq!(
            panes(&scene, "employee"),
            0,
            "a crew of two never opens one member blind"
        );
        let single = build(
            &picture(),
            &View::C {
                room: "lines".to_string(),
            },
        );
        assert_eq!(
            panes(&single, "employee"),
            2,
            "alone, the worker opens their own pane"
        );
    }

    #[test]
    fn the_camera_s_up_is_perpendicular_to_its_direction_and_unit_long() {
        let dot: f32 = (0..3).map(|i| CAMERA_FROM[i] * CAMERA_UP[i]).sum();
        assert!(dot.abs() < 1e-6, "{dot}");
        let len = CAMERA_UP.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((len - 1.0).abs() < 1e-6, "{len}");
        // Up is up: a card's centre lies above its feet.
        let [_, y, _] = over(0.0, 0.0, 1.0);
        assert!(y > 0.0, "{y}");
    }

    #[test]
    fn the_figures_are_painted_cards_standing_on_a_shadow() {
        let snap = picture();
        let outside = build(&snap, &View::A);
        let figures = cards(&outside);
        assert!(figures.contains(&Figure::Steward), "{figures:?}");
        assert!(
            figures.contains(&Figure::Follower { model: Model::Opus }),
            "{figures:?}"
        );
        let lines = build(
            &snap,
            &View::C {
                room: "lines".to_string(),
            },
        );
        let robots = cards(&lines);
        assert!(
            robots.contains(&Figure::Robot {
                model: Model::Opus,
                wrench: true,
                active: true
            }),
            "technical refinement writes, on opus, now: {robots:?}"
        );
        assert!(
            robots.contains(&Figure::Robot {
                model: Model::Sonnet,
                wrench: false,
                active: false
            }),
            "slice reads, on sonnet, idle: {robots:?}"
        );
        for prop in lines
            .props
            .iter()
            .filter(|p| matches!(p.shape, Shape::Card { .. }))
        {
            let Shape::Card { h, .. } = prop.shape else {
                unreachable!()
            };
            // The card's centre is half its height up the screen from its
            // feet, and a shadow lies on the ground where the feet are.
            assert!(
                (prop.at[1] - CAMERA_UP[1] * h / 2.0).abs() < 1e-5,
                "{prop:?}"
            );
            let foot_x = prop.at[0] - CAMERA_UP[0] * h / 2.0;
            let shadow = lines.props.iter().any(|s| {
                s.translucent
                    && matches!(s.shape, Shape::Cylinder { .. })
                    && (s.at[0] - foot_x).abs() < 1e-4
                    && s.hot == prop.hot
            });
            assert!(shadow, "a shadow under {prop:?}");
            assert!(prop.hot.is_some(), "every figure opens its pane");
        }
    }

    #[test]
    fn a_colour_reads_from_hex_and_a_bad_one_is_grey_not_a_panic() {
        let c = Rgba::hex("#ff8000");
        assert!(
            (c.0 - 1.0).abs() < 1e-6
                && (c.1 - 128.0 / 255.0).abs() < 1e-6
                && c.2 == 0.0
                && (c.3 - 1.0).abs() < 1e-6
        );
        assert!((Rgba::hex("#5ad1e644").3 - 68.0 / 255.0).abs() < 1e-6);
        assert_eq!(Rgba::hex("oops"), Rgba(0.5, 0.5, 0.5, 1.0));
        assert!((Rgba::hex("#ffffff").shade(0.5).0 - 0.5).abs() < 1e-6);
        assert!((Rgba::hex("#ffffff").alpha(0.3).3 - 0.3).abs() < 1e-6);
    }

    #[test]
    fn nothing_but_the_floors_has_a_sharp_corner() {
        for view in [
            View::A,
            View::B,
            View::C {
                room: "lines".to_string(),
            },
            View::C {
                room: "store".to_string(),
            },
        ] {
            let scene = build(&picture(), &view);
            let sharp = scene
                .props
                .iter()
                .filter(|p| matches!(p.shape, Shape::Cuboid { .. }))
                .count();
            assert!(
                sharp <= 12,
                "{view:?}: {sharp} sharp boxes — only floors and roads may be"
            );
            let kinds = shapes(&scene);
            let round = kinds.iter().filter(|k| **k != "cuboid").count();
            assert!(
                round * 4 > kinds.len() * 3,
                "{view:?}: most shapes are rounded"
            );
        }
    }

    #[test]
    fn a_rounding_never_exceeds_what_the_shape_can_take() {
        let scene = build(
            &picture(),
            &View::C {
                room: "lines".to_string(),
            },
        );
        for prop in &scene.props {
            if let Shape::Rounded { w, h, d, r } = prop.shape {
                assert!(r <= w.min(h).min(d) / 2.0 + 1e-6, "{prop:?}");
                assert!(r > 0.0);
            }
        }
    }

    #[test]
    fn outside_there_is_a_chimney_per_model_a_door_a_sign_a_steward_and_the_workers() {
        let scene = build(&picture(), &View::A);
        let chimneys: std::collections::BTreeSet<&str> = hots(&scene)
            .into_iter()
            .filter(|h| pane_kind(h) == Some("chimney"))
            .filter_map(|h| h.pane.as_ref()?.get("model")?.as_str())
            .collect();
        assert_eq!(chimneys.into_iter().collect::<Vec<_>>(), ["opus", "sonnet"]);
        assert!(panes(&scene, "board") >= 3, "the sign's posts and board");
        assert_eq!(panes(&scene, "steward"), 2, "the card and its shadow");
        assert_eq!(panes(&scene, "employee"), 2, "one worker: card and shadow");
        // The worker stands out on the street, in front of the board — not
        // hidden between it and the wall.
        let nearest = |kind: &str| {
            scene
                .props
                .iter()
                .filter(|p| p.hot.as_ref().is_some_and(|h| pane_kind(h) == Some(kind)))
                .map(|p| p.at[2])
                .fold(f32::MIN, f32::max)
        };
        assert!(nearest("employee") > nearest("board") + 0.5);
        let door = hots(&scene)
            .into_iter()
            .filter(|h| h.go.as_deref() == Some("B"))
            .count();
        assert_eq!(door, 11, "ring, eight teeth, leaf and hub");
        let smoke = scene
            .props
            .iter()
            .filter(|p| matches!(p.anim, Some(Anim::Smoke { .. })))
            .count();
        assert_eq!(smoke, 7, "only the opus chimney smokes");
        assert!(
            scene.props.iter().filter(|p| p.translucent).all(|p| {
                matches!(p.anim, Some(Anim::Smoke { .. }))
                    || matches!(p.shape, Shape::Cylinder { .. })
            }),
            "outside, only smoke and the figures' shadows are see-through"
        );
        assert!(scene.labels.iter().any(|l| l.text == "DND_HELPER"));
        assert!(scene.max[0] >= 19.0 && scene.max[2] >= 15.0);
        let barrels = scene
            .props
            .iter()
            .filter(|p| matches!(p.shape, Shape::Capsule { .. }) && p.tilt[0] > 89.0)
            .count();
        assert!(barrels >= 4, "the roof is barrel vaults");
    }

    #[test]
    fn the_janitor_leans_on_his_broom_outside_by_the_tower_and_the_water() {
        let scene = build(&picture(), &View::A);
        assert!(
            cards(&scene).contains(&Figure::Janitor),
            "they stand outside"
        );
        assert!(
            panes(&scene, "janitor") >= 3,
            "card, shadow and bucket call him"
        );
        let at: Vec<[f32; 3]> = scene
            .props
            .iter()
            .filter(|p| p.hot.as_ref().and_then(pane_kind) == Some("janitor"))
            .map(|p| p.at)
            .collect();
        // Between the tower (x 12–15) and the quay (x 17), in front of the
        // tower's foot rather than behind it.
        assert!(
            at.iter().all(|p| p[0] > 14.0 && p[0] < 17.0 && p[2] > 7.0),
            "{at:?}"
        );
        let groups: std::collections::BTreeSet<u32> = scene
            .props
            .iter()
            .filter(|p| p.hot.as_ref().and_then(pane_kind) == Some("janitor"))
            .filter_map(|p| p.group)
            .collect();
        assert_eq!(groups.len(), 1, "they and their bucket grow as one");
        for view in [
            View::B,
            View::C {
                room: "lines".to_string(),
            },
        ] {
            assert!(!cards(&build(&picture(), &view)).contains(&Figure::Janitor));
        }
    }

    #[test]
    fn the_plant_is_brick_on_a_paved_street_by_moving_water() {
        let scene = build(&picture(), &View::A);
        let worn = |pattern: Pattern| scene.props.iter().filter(|p| p.pattern == pattern).count();
        assert!(
            worn(Pattern::Brick) >= 4,
            "hangar, tower, bridge, neighbour"
        );
        assert!(worn(Pattern::Seams) >= 4, "every vault shows its seams");
        assert!(worn(Pattern::Planks) >= 1, "the fence");
        assert_eq!(worn(Pattern::Water), 1, "one harbour");
        let floor = |pattern: Pattern| {
            scene
                .props
                .iter()
                .find(|p| p.pattern == pattern && matches!(p.shape, Shape::Cuboid { .. }))
                .map(|p| p.at)
        };
        let (Some(ground), Some(water)) = (floor(Pattern::Pavers), floor(Pattern::Water)) else {
            panic!("the ground is paved and the harbour is water");
        };
        assert!(
            water[1] < ground[1] - 0.2,
            "the water lies a step below the quay"
        );
        assert!(
            water[0] > ground[0],
            "the harbour is past the plant, toward +x"
        );
        assert!(
            scene
                .props
                .iter()
                .any(|p| p.at[0] > 17.0 && matches!(p.anim, Some(Anim::Bob { .. }))),
            "a boat bobs at the quay"
        );
    }

    #[test]
    fn inside_wears_the_outside_s_brick_and_tiles() {
        for view in [
            View::B,
            View::C {
                room: "lines".to_string(),
            },
            View::C {
                room: "office".to_string(),
            },
        ] {
            let scene = build(&picture(), &view);
            let floor = scene
                .props
                .iter()
                .find(|p| matches!(p.shape, Shape::Cuboid { .. }));
            assert_eq!(
                floor.map(|p| p.pattern),
                Some(Pattern::Pavers),
                "{view:?}: the floor is tiled"
            );
            assert!(
                scene
                    .props
                    .iter()
                    .filter(|p| p.pattern == Pattern::Brick)
                    .count()
                    >= 2,
                "{view:?}: the back walls are brick"
            );
            assert!(
                scene.props.iter().all(|p| matches!(
                    p.pattern,
                    Pattern::Plain | Pattern::Pavers | Pattern::Brick
                )),
                "{view:?}: no water or roof indoors"
            );
            assert!(
                scene.props.iter().all(|p| p.color != Rgba::hex(GRASS)),
                "{view:?}: no grass indoors"
            );
        }
    }

    #[test]
    fn every_pattern_has_its_own_id_and_a_positive_tile() {
        let all = [
            Pattern::Plain,
            Pattern::Brick,
            Pattern::Pavers,
            Pattern::Planks,
            Pattern::Seams,
            Pattern::Water,
        ];
        let ids: std::collections::BTreeSet<u8> = all.iter().map(|p| p.id()).collect();
        assert_eq!(ids.len(), all.len());
        assert_eq!(Pattern::default().id(), 0, "plain is what the shader skips");
        assert!(all.iter().all(|p| p.tile() > 0.0));
    }

    #[test]
    fn inside_each_room_is_one_hotspot_that_enters_it() {
        let scene = build(&picture(), &View::B);
        for key in [
            "lines",
            "office",
            "store",
            "infirmary",
            "control",
            "construction",
        ] {
            let n = hots(&scene)
                .into_iter()
                .filter(|h| h.go.as_deref() == Some("C") && h.room.as_deref() == Some(key))
                .count();
            assert_eq!(n, 3, "{key}: floor and two walls");
        }
        assert!(
            scene
                .labels
                .iter()
                .any(|l| l.text == "6 · Under construction")
        );
        assert_eq!(panes(&scene, "steward"), 2);
    }

    #[test]
    fn the_lines_room_puts_the_product_and_the_worker_at_the_active_station() {
        let snap = picture();
        let scene = build(
            &snap,
            &View::C {
                room: "lines".to_string(),
            },
        );
        let product = scene
            .props
            .iter()
            .find(|p| p.key.as_deref() == Some("agent-loop:product"))
            .expect("product");
        let Some(Anim::Slide { to }) = product.anim else {
            panic!("the product slides")
        };
        // Station index 2 (technical-refinement) is active: x0 + 0.3 + 2*sp + 0.25.
        let expected = LINE_X0 + 0.3 + 2.0 * STATION_SPACING + 0.25;
        assert!((to - expected).abs() < 1e-5, "{to}");
        assert_eq!(panes(&scene, "employee"), 2);
        assert_eq!(panes(&scene, "line"), 2);
        assert!(
            scene.labels.iter().any(|l| l.text == "✓ pick"),
            "pick is done, marked before its name"
        );
        assert!(
            scene.labels.iter().any(|l| l.text == "– slice"),
            "slice is skipped, marked before its name"
        );
        let stations = panes(&scene, "station");
        assert_eq!(
            stations, 32,
            "six per gate, two per robot (card and shadow), six for the printer, eight for the arm"
        );
        let belt_stripes = scene
            .props
            .iter()
            .filter(|p| matches!(p.anim, Some(Anim::Belt { .. })))
            .count();
        assert!(belt_stripes > 0, "the active line's belt moves");
        let arches = scene
            .props
            .iter()
            .filter(|p| matches!(p.shape, Shape::Torus { .. }) && p.tilt[2] > 89.0)
            .count();
        assert!(arches >= 2, "gates are arches");
        let scanning = scene
            .props
            .iter()
            .filter(|p| {
                p.translucent && matches!(p.shape, Shape::Cylinder { .. }) && p.tilt[2] > 89.0
            })
            .count();
        assert_eq!(scanning, 0, "no gate is scanning in this round");
    }

    #[test]
    fn the_infirmary_has_the_doctor_and_the_office_took_the_product_corner() {
        let snap = picture();
        let infirmary = build(
            &snap,
            &View::C {
                room: "infirmary".to_string(),
            },
        );
        assert!(
            cards(&infirmary).contains(&Figure::Doctor),
            "the doctor stands in"
        );
        assert!(
            panes(&infirmary, "doctor") > 2,
            "the doctor, the bed, the desk and the cabinet all call them"
        );
        assert_eq!(
            panes(&infirmary, "features"),
            0,
            "the feature list moved out"
        );
        let office = build(
            &snap,
            &View::C {
                room: "office".to_string(),
            },
        );
        assert!(
            panes(&office, "features") > 0,
            "the feature list is in the office"
        );
        let titles: Vec<&str> = hots(&office)
            .iter()
            .filter_map(|h| h.pane.as_ref()?.get("title")?.as_str())
            .collect();
        assert!(titles.contains(&"The mock-up"), "{titles:?}");
        assert!(titles.contains(&"Data sources"), "{titles:?}");
        assert!(titles.contains(&"Talk to Claude Code"), "{titles:?}");
        let inside = build(&snap, &View::B);
        assert!(
            cards(&inside).contains(&Figure::Doctor),
            "and in the hall's room 4"
        );
        assert!(panes(&inside, "doctor") >= 2);
    }

    #[test]
    fn the_office_pins_the_open_tasks_next_up_first_and_skips_the_done() {
        let scene = build(
            &picture(),
            &View::C {
                room: "office".to_string(),
            },
        );
        let mut numbers: Vec<u64> = hots(&scene)
            .into_iter()
            .filter_map(|h| h.pane.as_ref())
            .filter(|p| p.get("kind").and_then(|k| k.as_str()) == Some("issue"))
            .filter_map(|p| p.get("number").and_then(serde_json::Value::as_u64))
            .collect();
        numbers.dedup();
        assert_eq!(
            numbers,
            [66, 67, 61],
            "ready, human, delivered — never the done one"
        );
    }

    #[test]
    fn the_store_shelves_link_to_github_and_the_counter_opens_the_pane() {
        let scene = build(
            &picture(),
            &View::C {
                room: "store".to_string(),
            },
        );
        let urls: Vec<&str> = hots(&scene)
            .into_iter()
            .filter_map(|h| h.url.as_deref())
            .collect();
        assert_eq!(
            urls,
            [
                "https://github.com/x/tree/main",
                "https://github.com/x/tree/main_agent",
                "https://github.com/x/tree/milestone/17-dernier"
            ]
        );
        assert!(scene.labels.iter().any(|l| l.text == "#17"));
        assert!(panes(&scene, "versions") >= 1);
    }

    #[test]
    fn the_control_room_reads_the_numbers_and_the_quota() {
        let scene = build(
            &picture(),
            &View::C {
                room: "control".to_string(),
            },
        );
        assert!(
            scene
                .labels
                .iter()
                .any(|l| l.text == "$58.09 · 316 sessions")
        );
        assert!(
            scene
                .labels
                .iter()
                .any(|l| l.text == "seven_day 90% used" && l.color == Rgba::hex(WARN))
        );
        assert!(scene.labels.iter().any(|l| l.text.starts_with("107.0M in")));
        assert!(
            panes(&scene, "dashboards") >= 5,
            "the panel and four screens"
        );
    }

    #[test]
    fn an_unknown_room_is_the_construction_site_and_an_empty_picture_still_draws() {
        let scene = build(
            &Snapshot::default(),
            &View::C {
                room: "nope".to_string(),
            },
        );
        assert!(scene.labels.iter().any(|l| l.text == "UNDER CONSTRUCTION"));
        assert!(shapes(&scene).contains(&"cone"), "traffic cones are cones");
        let outside = build(&Snapshot::default(), &View::A);
        assert!(outside.labels.iter().any(|l| l.text == "no GitHub board"));
        assert_ne!(outside.props, [] as [Prop; 0]);
        let inside = build(&Snapshot::default(), &View::B);
        assert_eq!(
            panes(&inside, "steward"),
            2,
            "the steward is always in the hall"
        );
    }

    #[test]
    fn ages_read_like_the_page_s() {
        assert_eq!(age(5), "5s");
        assert_eq!(age(125), "2m");
        assert_eq!(age(3700), "1h 1m");
        assert_eq!(age(200_000), "2d");
        assert_eq!(kfmt(107_000_000), "107.0M");
        assert_eq!(kfmt(4_100), "4k");
    }
}
