//! The Bevy side: a [`Scene`] becomes entities, the pointer becomes events.
//!
//! One camera, orthographic, looking down from the isometric corner. Every
//! opaque prop is drawn twice: once with the ink-wash [`ToonMaterial`] (a
//! flat colour in three bands of light), once as a slightly larger black
//! shell with its front faces culled — the inverted hull that gives the
//! paper-cutout outline. Smoke and scanner discs, which are see-through, use
//! the engine's own material and no outline.
//!
//! The figures are cards: a quad turned to face the camera, textured with
//! the cut-out [`crate::sprites`] paints for it, once per figure, the first
//! time one is needed. The texture is unlit and its transparent margin is
//! cut away rather than blended, so a card sorts against the props like any
//! solid thing and never needs an outline of its own.
//!
//! A figure — a door, a robot, a sign — is several props with one hotspot;
//! they carry one [`Group`], and while the pointer is over any of them all
//! of them grow a touch, as one. Only the hot props and the opaque props
//! that may stand in front of them are ray cast: the outline shells, the
//! smoke and the belts' stripes are never even considered.
//!
//! Meshes are cached by shape: a unit sphere, cylinder and cone are scaled
//! per prop, the rest (rounded boxes, capsules, cut cones, rings) are built
//! per size, since scaling would distort their rounding.
//!
//! The scene is rebuilt whenever the page pushes a new picture or moves the
//! viewer: every prop entity is despawned and the list is spawned again. A
//! few hundred entities, a few times a minute — cheap, and it keeps this
//! module free of diffing. What must survive a rebuild (where the product
//! was, so it slides rather than jumps) is remembered by key; labels are
//! kept and updated in place, because a UI node respawned has no size for a
//! frame and jumps when it gets one.

// Motion is `base + sin(t) * amp`; written that way it reads as motion. A
// fused multiply-add gains nothing on a few hundred props a frame.
#![allow(clippy::suboptimal_flops)]

use std::collections::{HashMap, HashSet};
use std::f32::consts::PI;

use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::camera::ScalingMode;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::picking::mesh_picking::{MeshPickingCamera, MeshPickingSettings};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, Extent3d, Face, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::text::Justify;

use crate::bridge::{self, Hot, Outbound, View};
use crate::model::Snapshot;
use crate::scene::{self, Anim, Backing, Label, Prop, Rgba, Scene, Shape};
use crate::sprites::{self, Figure};

/// Everything spawned for the current scene — despawned on rebuild.
#[derive(Component)]
struct SceneProp;

/// What a rebuild tears down: every scene entity, with what some remember.
type Old<'w, 's> = Query<
    'w,
    's,
    (Entity, Option<&'static Keyed>, Option<&'static Transform>),
    (With<SceneProp>, Without<Anchored>),
>;

/// The labels already on screen, by slot, with their current text.
type Labels<'w, 's> = Query<'w, 's, (Entity, &'static LabelSlot, &'static Text), With<Anchored>>;

/// What a click or a hover reports.
#[derive(Component)]
struct HotSpot(Hot);

/// A prop that moves, and where it started.
#[derive(Component)]
struct Animated {
    anim: Anim,
    base: Vec3,
    /// Its scale at rest, for the pulses that scale it.
    scale: Vec3,
}

/// A prop whose last position is remembered across rebuilds.
#[derive(Component)]
struct Keyed(String);

/// A UI label hanging from a world point.
#[derive(Component)]
struct Anchored(Vec3);

/// Which label of the scene's list this node shows.
///
/// Labels are not torn down with the props: a UI node that is respawned has
/// no computed size for a frame and jumps when it gets one, and the picture
/// is pushed every few seconds. Nodes are kept by slot and updated in place;
/// only a changed text relayouts, and only that label.
#[derive(Component)]
struct LabelSlot(usize);

/// The figure a hot prop belongs to: every shape of a door, a robot or a
/// sign carries the same one, and they grow together under the pointer.
#[derive(Component)]
struct Group(u32);

/// A prop's scale at rest — what the hover grows it from.
#[derive(Component)]
struct RestScale(Vec3);

/// The hot entities under the pointer right now. A figure is hovered while
/// any of its shapes is, so the pointer crossing from one shape of a figure
/// to the next never lets it shrink in between.
#[derive(Resource, Default)]
struct Hovering(HashSet<Entity>);

/// The picture, the viewpoint, and whether the entities match them.
#[derive(Resource, Default)]
struct Picture {
    snapshot: Snapshot,
    view: View,
    dirty: bool,
}

/// Where keyed props were when the scene was last torn down.
#[derive(Resource, Default)]
struct Memory(HashMap<String, Vec3>);

/// The camera's framing: what it looks at, how tall its view is, and whether
/// the human took the controls.
#[derive(Resource)]
struct Rig {
    center: Vec3,
    height: f32,
    /// The scene's width as the camera sees it, from the last fit.
    width: f32,
    user: bool,
    /// Pixels dragged since the last press — a click after a drag is a drag.
    dragged: f32,
}

impl Default for Rig {
    fn default() -> Self {
        Self {
            center: Vec3::ZERO,
            height: 20.0,
            width: 32.0,
            user: false,
            dragged: 0.0,
        }
    }
}

/// The one typeface every label uses.
///
/// Bevy's default font is ASCII only; the labels carry `·`, `✓`, `–`, `●`.
/// `DejaVu Sans` is embedded (750 KB, Bitstream Vera licence — see
/// `assets/DejaVuSans-LICENSE.txt`) and loaded once at start-up.
#[derive(Resource)]
struct Typeface(Handle<Font>);

const TYPEFACE: &[u8] = include_bytes!("../assets/DejaVuSans.ttf");

/// The ink-wash material: a flat colour lit in three bands, plus a glow.
/// The shader is `toon.wgsl`, beside this file.
#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct ToonMaterial {
    /// The flat colour, linear.
    #[uniform(0)]
    pub color: LinearRgba,
    /// What it emits on top, linear; black for most things.
    #[uniform(1)]
    pub emissive: LinearRgba,
    /// The pattern painted over it: `x` is [`scene::Pattern::id`], `y` its tile
    /// size in world units.
    #[uniform(2)]
    pub pattern: Vec4,
}

impl Material for ToonMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://harness_view_render/toon.wgsl".into()
    }
}

/// Meshes by shape, built once each.
#[derive(Resource, Default)]
struct MeshCache(HashMap<u64, Handle<Mesh>>);

/// Toon materials by colour and pattern, the engine's materials for the see-through, and
/// the painted figures by who is on them.
#[derive(Resource, Default)]
struct Palette {
    /// By colour, glow and pattern.
    toon: HashMap<(u64, u8), Handle<ToonMaterial>>,
    glass: HashMap<u64, Handle<StandardMaterial>>,
    /// Each figure's cut-out, painted once and kept.
    cards: HashMap<Figure, Handle<StandardMaterial>>,
    /// The one ink shell every outline is drawn with.
    ink: Option<Handle<StandardMaterial>>,
}

/// The outline's thickness, in world units — about two pixels at the default
/// framing, the width of an inked line.
const OUTLINE: f32 = 0.06;

/// The isometric corner: 45° around, 35.26° down — the classic. The scene
/// owns the number, because its cards lean on the same direction.
fn camera_direction() -> Vec3 {
    Vec3::from(scene::CAMERA_FROM).normalize()
}

/// The rotation that turns a quad in the `xy` plane to face the camera, its
/// top up the screen: the camera's own.
fn billboard_rotation() -> Quat {
    Transform::from_translation(camera_direction())
        .looking_at(Vec3::ZERO, Vec3::Y)
        .rotation
}

/// The plant plugin: everything this crate adds to an `App`.
pub struct PlantPlugin;

impl Plugin for PlantPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "toon.wgsl");
        app.add_plugins((MeshPickingPlugin, MaterialPlugin::<ToonMaterial>::default()))
            // Only what is marked is ray cast: the hot props and the opaque
            // ones that may stand in front of them, never the outline
            // shells or the see-through — half the meshes.
            .insert_resource(MeshPickingSettings {
                require_markers: true,
                ..default()
            })
            // The sky behind the plant: a dusk teal, the one colour here
            // that is dark, so the grass and the roofs read as lit.
            .insert_resource(ClearColor(Color::srgb(0.165, 0.22, 0.239)))
            .insert_resource(GlobalAmbientLight {
                color: Color::WHITE,
                brightness: 900.0,
                ..default()
            })
            .init_resource::<Picture>()
            .init_resource::<Memory>()
            .init_resource::<Rig>()
            .init_resource::<Palette>()
            .init_resource::<MeshCache>()
            .init_resource::<Hovering>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    take_inbox,
                    rebuild.run_if(|picture: Res<'_, Picture>| picture.dirty),
                    camera_controls,
                    frame,
                    animate,
                    hover_grow,
                    place_labels,
                )
                    .chain(),
            )
            .add_observer(on_click)
            .add_observer(on_press)
            .add_observer(on_over)
            .add_observer(on_out);
    }
}

/// Runs the plant in the canvas named by `selector` (`#scene`).
pub fn run(selector: &str) {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        canvas: Some(selector.to_string()),
                        fit_canvas_to_parent: true,
                        // The page's own shortcuts (Esc closes a pane) must
                        // keep working while the canvas has focus.
                        prevent_default_event_handling: false,
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::log::LogPlugin {
                    // The engine's own warnings, and this crate's pointer
                    // and scene lines, in the browser console.
                    filter: "warn,harness_view_render=info".to_string(),
                    level: bevy::log::Level::INFO,
                    ..default()
                }),
        )
        .add_plugins(PlantPlugin)
        .run();
}

fn setup(
    mut commands: Commands<'_, '_>,
    mut fonts: ResMut<'_, Assets<Font>>,
    mut materials: ResMut<'_, Assets<StandardMaterial>>,
    mut palette: ResMut<'_, Palette>,
) {
    commands.insert_resource(Typeface(fonts.add(Font::from_bytes(TYPEFACE.to_vec()))));
    palette.ink = Some(materials.add(StandardMaterial {
        // The scene's ink: a dark plum, as a pen draws it, never black.
        base_color: Color::srgb(0.165, 0.106, 0.18),
        unlit: true,
        // The inverted hull: only the back of the shell shows, around the
        // prop, as an outline.
        cull_mode: Some(Face::Front),
        ..default()
    }));
    commands.spawn((
        Camera3d::default(),
        MeshPickingCamera,
        Projection::from(OrthographicProjection {
            scaling_mode: ScalingMode::FixedVertical {
                viewport_height: 20.0,
            },
            // The camera sits 80 units out; nothing lies behind it.
            near: 0.1,
            far: 500.0,
            ..OrthographicProjection::default_3d()
        }),
        Tonemapping::None,
        Transform::from_translation(camera_direction() * 80.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    // For the see-through props only: the toon material carries its own light.
    commands.spawn((
        DirectionalLight {
            illuminance: 5500.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(4.0, 9.0, 2.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    bridge::emit(&Outbound::Ready);
}

/// Reads what the page pushed since the last frame.
fn take_inbox(mut picture: ResMut<'_, Picture>, mut rig: ResMut<'_, Rig>) {
    let inbox = bridge::take();
    if let Some(json) = inbox.snapshot {
        match Snapshot::parse(&json) {
            Ok(snapshot) => {
                picture.snapshot = snapshot;
                picture.dirty = true;
            }
            Err(e) => warn!("a picture the renderer could not read: {e}"),
        }
    }
    if let Some(view) = inbox.view {
        if view != picture.view {
            // A new place: the camera frames it afresh.
            rig.user = false;
        }
        picture.view = view;
        picture.dirty = true;
    }
}

const fn color(rgba: Rgba) -> Color {
    Color::srgba(rgba.0, rgba.1, rgba.2, rgba.3)
}

/// A colour as a cache key: eight bits a channel.
fn quantize(c: Rgba) -> u64 {
    // Clamped to 0..=255 before the cast, so nothing truncates.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    [c.0, c.1, c.2, c.3].iter().fold(0_u64, |acc, v| {
        (acc << 8) | u64::from((v * 255.0).round().clamp(0.0, 255.0) as u8)
    })
}

/// The engine's material for a see-through prop, made once per colour.
fn glass_material(
    palette: &mut Palette,
    materials: &mut Assets<StandardMaterial>,
    prop: &Prop,
) -> Handle<StandardMaterial> {
    let key = quantize(prop.color) ^ prop.emissive.map_or(0, |e| quantize(e).rotate_left(32));
    palette
        .glass
        .entry(key)
        .or_insert_with(|| {
            let mut mat = StandardMaterial::from(color(prop.color));
            mat.perceptual_roughness = 0.9;
            mat.metallic = 0.0;
            mat.reflectance = 0.2;
            mat.alpha_mode = AlphaMode::Blend;
            if let Some(e) = prop.emissive {
                mat.emissive = LinearRgba::from(color(e)) * 1.6;
            }
            materials.add(mat)
        })
        .clone()
}

/// The ink-wash material for an opaque prop, made once per colour.
fn toon_material(
    palette: &mut Palette,
    materials: &mut Assets<ToonMaterial>,
    prop: &Prop,
) -> Handle<ToonMaterial> {
    let key = (
        quantize(prop.color) ^ prop.emissive.map_or(0, |e| quantize(e).rotate_left(32)),
        prop.pattern.id(),
    );
    palette
        .toon
        .entry(key)
        .or_insert_with(|| {
            materials.add(ToonMaterial {
                color: LinearRgba::from(color(prop.color)),
                emissive: prop
                    .emissive
                    .map_or(LinearRgba::BLACK, |e| LinearRgba::from(color(e)) * 0.9),
                pattern: Vec4::new(f32::from(prop.pattern.id()), prop.pattern.tile(), 0.0, 0.0),
            })
        })
        .clone()
}

/// A shape's size as a cache key, to the centimetre.
///
/// Four sizes of fourteen bits each (up to 163 units) under a four-bit tag:
/// the fields never reach the tag, so two shapes of different kinds can
/// never share a key.
fn mesh_key(shape: Shape) -> u64 {
    // Sizes are a few units at most and positive: `* 100` fits fourteen bits
    // and truncation to the centimetre is the point.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let q = |v: f32| u64::from((v * 100.0).round().clamp(0.0, 16383.0) as u16);
    let pack = |tag: u64, a: f32, b: f32, c: f32, d: f32| {
        tag << 60 | q(a) << 42 | q(b) << 28 | q(c) << 14 | q(d)
    };
    match shape {
        Shape::Rounded { w, h, d, r } => pack(1, w, h, d, r),
        Shape::Capsule { r, h } => pack(2, r, h, 0.0, 0.0),
        Shape::Frustum { bottom, top, h } => pack(3, bottom, top, h, 0.0),
        Shape::Torus { ring, tube } => pack(4, ring, tube, 0.0, 0.0),
        Shape::Cuboid { .. } => pack(5, 0.0, 0.0, 0.0, 0.0),
        Shape::Cylinder { .. } => pack(6, 0.0, 0.0, 0.0, 0.0),
        Shape::Cone { .. } => pack(7, 0.0, 0.0, 0.0, 0.0),
        Shape::Sphere { .. } => pack(8, 0.0, 0.0, 0.0, 0.0),
        Shape::Card { .. } => pack(9, 0.0, 0.0, 0.0, 0.0),
    }
}

/// The mesh for a shape, and the scale that puts it at size: unit meshes
/// for what scales cleanly, a mesh per size for what does not.
fn mesh_for(
    cache: &mut MeshCache,
    meshes: &mut Assets<Mesh>,
    shape: Shape,
) -> (Handle<Mesh>, Vec3) {
    let scale = match shape {
        Shape::Cuboid { w, h, d } => Vec3::new(w, h, d),
        Shape::Cylinder { r, h } | Shape::Cone { r, h } => Vec3::new(r, h, r),
        Shape::Sphere { r } => Vec3::splat(r),
        Shape::Card { w, h, .. } => Vec3::new(w, h, 1.0),
        Shape::Rounded { .. }
        | Shape::Capsule { .. }
        | Shape::Frustum { .. }
        | Shape::Torus { .. } => Vec3::ONE,
    };
    let handle = cache
        .0
        .entry(mesh_key(shape))
        .or_insert_with(|| match shape {
            Shape::Rounded { w, h, d, r } => meshes.add(rounded_box(w, h, d, r)),
            Shape::Cuboid { .. } => meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            Shape::Card { .. } => meshes.add(Rectangle::new(1.0, 1.0)),
            Shape::Cylinder { .. } => meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(28)),
            Shape::Cone { .. } => meshes.add(Cone::new(1.0, 1.0).mesh().resolution(28)),
            Shape::Frustum { bottom, top, h } => meshes.add(
                ConicalFrustum {
                    radius_top: top,
                    radius_bottom: bottom,
                    height: h,
                }
                .mesh()
                .resolution(28),
            ),
            Shape::Capsule { r, h } => meshes.add(
                Capsule3d::new(r, (h - 2.0 * r).max(0.0))
                    .mesh()
                    .latitudes(12)
                    .longitudes(24),
            ),
            Shape::Sphere { .. } => meshes.add(Sphere::new(1.0).mesh().uv(24, 16)),
            Shape::Torus { ring, tube } => meshes.add(
                Torus {
                    minor_radius: tube,
                    major_radius: ring,
                }
                .mesh()
                .minor_resolution(14)
                .major_resolution(36),
            ),
        })
        .clone();
    (handle, scale)
}

/// The world half extents of a shape — what an outline has to grow past.
fn half_extents(shape: Shape) -> Vec3 {
    match shape {
        Shape::Rounded { w, h, d, .. } | Shape::Cuboid { w, h, d } => Vec3::new(w, h, d) / 2.0,
        Shape::Cylinder { r, h } | Shape::Cone { r, h } | Shape::Capsule { r, h } => {
            Vec3::new(r, h / 2.0, r)
        }
        Shape::Frustum { bottom, top, h } => Vec3::new(bottom.max(top), h / 2.0, bottom.max(top)),
        Shape::Sphere { r } => Vec3::splat(r),
        Shape::Torus { ring, tube } => Vec3::new(ring + tube, tube, ring + tube),
        Shape::Card { w, h, .. } => Vec3::new(w / 2.0, h / 2.0, w / 2.0),
    }
}

/// A box with rounded edges and corners, as a mesh.
///
/// A sphere's vertices pushed out to the box's corners: each vertex of a
/// sphere of radius `r` moves by the box's inner half extents in the
/// direction of its normal's signs. The patches between become the flat
/// faces, the sphere's own normals stay right everywhere, and no vertex
/// lands on a sign boundary because the grid is offset by half a step.
// `w`, `h`, `d`, `r`, `n`: geometry's own names. The grid indices are a few
// dozen, far below where an `f32` loses an integer.
#[allow(clippy::many_single_char_names, clippy::cast_precision_loss)]
#[must_use]
pub fn rounded_box(w: f32, h: f32, d: f32, r: f32) -> Mesh {
    const LON: usize = 16;
    const LAT: usize = 11;
    let r = r.min(w.min(h).min(d) / 2.0).max(0.001);
    let inner = Vec3::new(w / 2.0 - r, h / 2.0 - r, d / 2.0 - r).max(Vec3::ZERO);
    let mut positions = Vec::with_capacity((LAT + 1) * (LON + 1));
    let mut normals = Vec::with_capacity((LAT + 1) * (LON + 1));
    let mut uvs = Vec::with_capacity((LAT + 1) * (LON + 1));
    for i in 0..=LAT {
        let theta = PI * (i as f32 + 0.5) / (LAT as f32 + 1.0);
        for j in 0..=LON {
            let phi = 2.0 * PI * (j as f32 + 0.5) / LON as f32;
            let n = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            let corner = Vec3::new(
                inner.x.copysign(n.x),
                inner.y.copysign(n.y),
                inner.z.copysign(n.z),
            );
            let p = corner + n * r;
            positions.push([p.x, p.y, p.z]);
            normals.push([n.x, n.y, n.z]);
            uvs.push([j as f32 / LON as f32, i as f32 / LAT as f32]);
        }
    }
    // The poles: one vertex each, so the caps close.
    let top = positions.len();
    positions.push([0.0, h / 2.0, 0.0]);
    normals.push([0.0, 1.0, 0.0]);
    uvs.push([0.5, 0.0]);
    let bottom = positions.len();
    positions.push([0.0, -h / 2.0, 0.0]);
    normals.push([0.0, -1.0, 0.0]);
    uvs.push([0.5, 1.0]);
    let mut indices: Vec<u32> = Vec::new();
    let at = |i: usize, j: usize| u32::try_from(i * (LON + 1) + j).unwrap_or(0);
    for i in 0..LAT {
        for j in 0..LON {
            // Counter-clockwise seen from outside, which is what the engine
            // keeps and the outline's inverted hull culls.
            indices.extend_from_slice(&[at(i, j), at(i, j + 1), at(i + 1, j)]);
            indices.extend_from_slice(&[at(i, j + 1), at(i + 1, j + 1), at(i + 1, j)]);
        }
    }
    let (top, bottom) = (
        u32::try_from(top).unwrap_or(0),
        u32::try_from(bottom).unwrap_or(0),
    );
    for j in 0..LON {
        indices.extend_from_slice(&[top, at(0, j + 1), at(0, j)]);
        indices.extend_from_slice(&[bottom, at(LAT, j), at(LAT, j + 1)]);
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// The transform a prop wants: its centre, its tilt, and the scale that
/// puts the cached mesh at size.
fn placement(at: Vec3, tilt: [f32; 3], scale: Vec3) -> Transform {
    Transform {
        translation: at,
        rotation: Quat::from_euler(
            EulerRot::XYZ,
            tilt[0].to_radians(),
            tilt[1].to_radians(),
            tilt[2].to_radians(),
        ),
        scale,
    }
}

#[allow(clippy::too_many_arguments)] // A system's parameters are its dependencies, and this one spawns everything.
fn rebuild(
    mut commands: Commands<'_, '_>,
    mut picture: ResMut<'_, Picture>,
    mut memory: ResMut<'_, Memory>,
    mut rig: ResMut<'_, Rig>,
    mut palette: ResMut<'_, Palette>,
    mut cache: ResMut<'_, MeshCache>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut toons: ResMut<'_, Assets<ToonMaterial>>,
    mut glasses: ResMut<'_, Assets<StandardMaterial>>,
    mut images: ResMut<'_, Assets<Image>>,
    mut hovering: ResMut<'_, Hovering>,
    typeface: Res<'_, Typeface>,
    old: Old<'_, '_>,
    labels: Labels<'_, '_>,
) {
    picture.dirty = false;
    // Whatever was under the pointer is about to be despawned; the page's
    // tooltip goes with it.
    if !hovering.0.is_empty() {
        hovering.0.clear();
        bridge::emit(&Outbound::Leave);
    }
    for (entity, keyed, transform) in &old {
        if let (Some(Keyed(key)), Some(transform)) = (keyed, transform) {
            memory.0.insert(key.clone(), transform.translation);
        }
        commands.entity(entity).despawn();
    }
    let scene = scene::build(&picture.snapshot, &picture.view);
    info!(
        "scene: {:?}, {} props, {} labels",
        picture.view,
        scene.props.len(),
        scene.labels.len()
    );
    let mut assets = Assets3 {
        palette: &mut palette,
        cache: &mut cache,
        meshes: &mut meshes,
        toons: &mut toons,
        glasses: &mut glasses,
        images: &mut images,
    };
    for prop in &scene.props {
        spawn_prop(&mut commands, &mut assets, &memory, prop);
    }
    let mut slots: HashMap<usize, (Entity, String)> = labels
        .iter()
        .map(|(entity, slot, text)| (slot.0, (entity, text.0.clone())))
        .collect();
    for (i, label) in scene.labels.iter().enumerate() {
        match slots.remove(&i) {
            Some((entity, text)) => {
                update_label(&mut commands, entity, &typeface, label, text != label.text);
            }
            None => spawn_label(&mut commands, &typeface, i, label),
        }
    }
    for (entity, _) in slots.into_values() {
        commands.entity(entity).despawn();
    }
    if !rig.user {
        fit(&mut rig, &scene);
    }
}

/// The asset stores a spawn draws from, gathered so a prop takes one argument.
struct Assets3<'a> {
    palette: &'a mut Palette,
    cache: &'a mut MeshCache,
    meshes: &'a mut Assets<Mesh>,
    toons: &'a mut Assets<ToonMaterial>,
    glasses: &'a mut Assets<StandardMaterial>,
    images: &'a mut Assets<Image>,
}

/// The material of a painted figure: its cut-out, rasterised the first time
/// the figure is asked for and kept, on a quad that shows it as painted —
/// unlit, both faces, the transparent margin cut away at half alpha.
fn card_material(assets: &mut Assets3<'_>, figure: Figure) -> Handle<StandardMaterial> {
    let Assets3 {
        palette,
        glasses,
        images,
        ..
    } = assets;
    palette
        .cards
        .entry(figure)
        .or_insert_with(|| {
            let sprite = sprites::paint(figure);
            let image = images.add(Image::new(
                Extent3d {
                    width: sprite.width,
                    height: sprite.height,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                sprite.rgba,
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            ));
            glasses.add(StandardMaterial {
                base_color_texture: Some(image),
                alpha_mode: AlphaMode::Mask(0.5),
                unlit: true,
                double_sided: true,
                cull_mode: None,
                ..default()
            })
        })
        .clone()
}

fn spawn_prop(
    commands: &mut Commands<'_, '_>,
    assets: &mut Assets3<'_>,
    memory: &Memory,
    prop: &Prop,
) {
    let (mesh, scale) = mesh_for(assets.cache, assets.meshes, prop.shape);
    // A sliding prop resumes from where it was, so a rebuild never makes
    // it jump back.
    let remembered = match (&prop.key, prop.anim) {
        (Some(key), Some(Anim::Slide { .. })) => memory.0.get(key).copied(),
        _ => None,
    };
    let at = remembered.unwrap_or_else(|| Vec3::from(prop.at));
    let mut transform = placement(at, prop.tilt, scale);
    if matches!(prop.shape, Shape::Card { .. }) {
        transform.rotation = billboard_rotation();
    }
    let mut entity = commands.spawn((SceneProp, Mesh3d(mesh.clone()), transform));
    if let Shape::Card { figure, .. } = prop.shape {
        entity.insert(MeshMaterial3d(card_material(assets, figure)));
    } else if prop.translucent {
        entity.insert(MeshMaterial3d(glass_material(
            assets.palette,
            assets.glasses,
            prop,
        )));
    } else {
        entity.insert(MeshMaterial3d(toon_material(
            assets.palette,
            assets.toons,
            prop,
        )));
        if let Some(ink) = assets.palette.ink.clone() {
            // The outline: the same mesh, grown by a constant thickness in
            // every direction, ink, back faces only. Unmarked, so the ray
            // cast never considers it.
            let grow =
                Vec3::ONE + Vec3::splat(OUTLINE) / half_extents(prop.shape).max(Vec3::splat(0.01));
            entity.with_children(|parent| {
                parent.spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(ink),
                    Transform::from_scale(grow.min(Vec3::splat(2.5))),
                ));
            });
        }
    }
    match (&prop.hot, prop.translucent) {
        (Some(hot), _) => {
            entity.insert((HotSpot(hot.clone()), Pickable::default()));
        }
        // Solid and mute: it hides what is behind it from the pointer as it
        // does from the eye.
        (None, false) => {
            entity.insert(Pickable {
                should_block_lower: true,
                is_hoverable: false,
            });
        }
        // Smoke, stripes, discs: the pointer sees through them, unmarked.
        (None, true) => {}
    }
    if let Some(group) = prop.group {
        entity.insert((Group(group), RestScale(scale)));
    }
    if let Some(anim) = prop.anim {
        entity.insert(Animated {
            anim,
            base: Vec3::from(prop.at),
            scale,
        });
    }
    if let Some(key) = &prop.key {
        entity.insert(Keyed(key.clone()));
    }
}

fn text_font(typeface: &Typeface, label: &Label) -> TextFont {
    TextFont {
        font: FontSource::Handle(typeface.0.clone()),
        font_size: FontSize::Px(if label.bold {
            label.size * 1.08
        } else {
            label.size
        }),
        ..default()
    }
}

/// The dark pill behind a tag hanging in the world: the scene's ink, a
/// little see-through.
const INK_PILL: BackgroundColor = BackgroundColor(Color::srgba(0.165, 0.106, 0.18, 0.86));

/// The grey plate behind text written on a board: a touch see-through, so
/// the board shows through and the text reads as part of it.
const SLATE_PLATE: BackgroundColor = BackgroundColor(Color::srgba(0.47, 0.45, 0.5, 0.6));

/// The background a label's backing asks for, if any.
const fn backing_color(backing: Backing) -> Option<BackgroundColor> {
    match backing {
        Backing::None => None,
        Backing::Ink => Some(INK_PILL),
        Backing::Slate => Some(SLATE_PLATE),
    }
}

fn spawn_label(commands: &mut Commands<'_, '_>, typeface: &Typeface, slot: usize, label: &Label) {
    let mut entity = commands.spawn((
        SceneProp,
        LabelSlot(slot),
        Anchored(Vec3::from(label.at)),
        Text::new(label.text.clone()),
        text_font(typeface, label),
        TextColor(color(label.color)),
        // A label of two lines — a line's title over its run count — is
        // centred, as a one-line label is by construction.
        TextLayout::justify(Justify::Center),
        Node {
            position_type: PositionType::Absolute,
            padding: UiRect::axes(Val::Px(6.0), Val::Px(2.0)),
            border_radius: BorderRadius::all(Val::Px(7.0)),
            ..default()
        },
        Pickable::IGNORE,
        Visibility::Hidden,
    ));
    if let Some(backing) = backing_color(label.backing) {
        entity.insert(backing);
    }
}

/// Moves and recolours a label already on screen; relayouts it only when its
/// text changed, so an unchanged label never flickers.
fn update_label(
    commands: &mut Commands<'_, '_>,
    entity: Entity,
    typeface: &Typeface,
    label: &Label,
    text_changed: bool,
) {
    let mut node = commands.entity(entity);
    node.insert((
        Anchored(Vec3::from(label.at)),
        TextColor(color(label.color)),
    ));
    if text_changed {
        node.insert((Text::new(label.text.clone()), text_font(typeface, label)));
    }
    match backing_color(label.backing) {
        Some(backing) => {
            node.insert(backing);
        }
        None => {
            node.remove::<BackgroundColor>();
        }
    }
}

/// Frames the scene's bounding box: the camera looks at its centre, and the
/// view is tall enough for every corner at the current aspect ratio.
fn fit(rig: &mut Rig, scene: &Scene) {
    let min = Vec3::from(scene.min);
    let max = Vec3::from(scene.max);
    rig.center = (min + max) / 2.0;
    let rotation = Transform::from_translation(camera_direction())
        .looking_at(Vec3::ZERO, Vec3::Y)
        .rotation
        .inverse();
    let mut lo = Vec2::splat(f32::MAX);
    let mut hi = Vec2::splat(f32::MIN);
    for i in 0..8 {
        let corner = Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y + 2.5 },
            if i & 4 == 0 { min.z } else { max.z },
        );
        let seen = rotation * (corner - rig.center);
        lo = lo.min(seen.truncate());
        hi = hi.max(seen.truncate());
    }
    let extent = hi - lo;
    rig.width = extent.x;
    rig.height = extent.y.max(extent.x / rig_aspect()) * 1.12 + 1.0;
}

/// The aspect ratio the framing assumes; refined by [`frame`] once the window
/// is known.
const fn rig_aspect() -> f32 {
    1.6
}

/// Applies the rig to the camera every frame, correcting the framing for the
/// window's real aspect ratio.
fn frame(
    rig: Res<'_, Rig>,
    windows: Query<'_, '_, &Window>,
    mut camera: Query<'_, '_, (&mut Transform, &mut Projection), With<Camera3d>>,
) {
    let Ok((mut transform, mut projection)) = camera.single_mut() else {
        return;
    };
    let aspect = windows
        .single()
        .map_or(rig_aspect(), |w| (w.width() / w.height().max(1.0)).max(0.2));
    let mut height = rig.height;
    if !rig.user {
        // The fit assumed a default aspect; a narrow pane needs more height
        // to show the same width.
        height = height.max(rig.width / aspect * 1.12 + 1.0);
    }
    *transform = Transform::from_translation(rig.center + camera_direction() * 80.0)
        .looking_at(rig.center, Vec3::Y);
    if let Projection::Orthographic(ortho) = &mut *projection {
        ortho.scaling_mode = ScalingMode::FixedVertical {
            viewport_height: height,
        };
    }
}

/// Drag to pan, wheel to zoom — around the pointer, so what is under it
/// stays under it.
fn camera_controls(
    mut rig: ResMut<'_, Rig>,
    buttons: Res<'_, ButtonInput<MouseButton>>,
    motion: Res<'_, AccumulatedMouseMotion>,
    scroll: Res<'_, AccumulatedMouseScroll>,
    windows: Query<'_, '_, &Window>,
    camera: Query<'_, '_, &Transform, With<Camera3d>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok(camera) = camera.single() else {
        return;
    };
    let per_pixel = rig.height / window.height().max(1.0);
    if buttons.just_pressed(MouseButton::Left) {
        rig.dragged = 0.0;
    }
    if buttons.pressed(MouseButton::Left) && motion.delta != Vec2::ZERO {
        rig.dragged += motion.delta.length();
        if rig.dragged > 3.0 {
            rig.user = true;
            let right = camera.right().as_vec3();
            let up = camera.up().as_vec3();
            rig.center -= right * motion.delta.x * per_pixel;
            rig.center += up * motion.delta.y * per_pixel;
        }
    }
    if scroll.delta.y != 0.0 {
        let lines = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        };
        let factor = (lines * 0.12).exp();
        let new_height = (rig.height * factor).clamp(3.0, 120.0);
        if let Some(cursor) = window.cursor_position() {
            // Keep the world point under the cursor fixed while zooming.
            let offset = cursor - Vec2::new(window.width(), window.height()) / 2.0;
            let right = camera.right().as_vec3();
            let up = camera.up().as_vec3();
            let world_offset = right * offset.x * per_pixel - up * offset.y * per_pixel;
            let height = rig.height;
            rig.center += world_offset * (1.0 - new_height / height);
        }
        rig.height = new_height;
        rig.user = true;
    }
}

/// Moves every animated prop from its base, by the clock.
fn animate(time: Res<'_, Time>, mut props: Query<'_, '_, (&Animated, &mut Transform)>) {
    let t = time.elapsed_secs();
    let dt = time.delta_secs();
    for (animated, mut transform) in &mut props {
        let base = animated.base;
        match animated.anim {
            Anim::Smoke { seed, speed } => {
                let ph = (t * speed + seed).rem_euclid(1.0);
                transform.translation =
                    base + Vec3::new((ph * 5.0 + seed).sin() * 0.35 * ph, ph * 2.6, 0.0);
                transform.scale = Vec3::splat(0.14 + ph * 0.55);
            }
            Anim::Sheet { seed, speed } => {
                let ph = (t * speed + seed).rem_euclid(1.0);
                transform.translation = base + Vec3::new(0.0, -ph * 0.3, ph * 0.9);
            }
            Anim::Belt { period, speed } => {
                let shift = (t * speed).rem_euclid(period);
                transform.translation.x = base.x + shift;
            }
            Anim::Bob { amp, speed, seed } => {
                transform.translation.y = base.y + (t * speed + seed).sin().abs() * amp;
            }
            Anim::Glow { speed } => {
                transform.scale = animated.scale * (1.0 + 0.08 * (t * speed).sin());
            }
            Anim::Swing { amp, speed } => {
                transform.translation.z = base.z + (t * speed).sin() * amp;
            }
            Anim::Slide { to } => {
                let x = transform.translation.x;
                transform.translation.x = x + (to - x) * (dt * 4.0).min(1.0);
            }
        }
    }
}

/// Pins every label to its world anchor, centred, or hides it when the
/// anchor is off screen.
fn place_labels(
    camera: Query<'_, '_, (&Camera, &GlobalTransform), With<Camera3d>>,
    mut labels: Query<'_, '_, (&Anchored, &mut Node, &mut Visibility, Option<&ComputedNode>)>,
) {
    let Ok((camera, transform)) = camera.single() else {
        return;
    };
    for (anchor, mut node, mut visibility, computed) in &mut labels {
        match camera.world_to_viewport(transform, anchor.0) {
            Ok(at) => {
                // A node laid out for the first time has no size yet; placing
                // it with a guess is what makes a label jump. One hidden frame
                // instead.
                let Some(size) = computed
                    .map(|c| c.size() * c.inverse_scale_factor())
                    .filter(|size| size.x > 0.0)
                else {
                    *visibility = Visibility::Hidden;
                    continue;
                };
                node.left = Val::Px(at.x - size.x / 2.0);
                node.top = Val::Px(at.y - size.y / 2.0);
                *visibility = Visibility::Inherited;
            }
            Err(_) => *visibility = Visibility::Hidden,
        }
    }
}

/// Every press, hot or not — the line to read in the browser console when a
/// click seems lost: which entity the engine's ray hit, and where.
fn on_press(
    press: On<'_, '_, Pointer<Press>>,
    hots: Query<'_, '_, &HotSpot>,
    transforms: Query<'_, '_, &Transform>,
) {
    info!(
        "press on {:?} (prop at {:?}) hit {:?} at {:?}{}",
        press.entity,
        transforms.get(press.entity).map(|t| t.translation).ok(),
        press.event.hit.position,
        press.pointer_location.position,
        if hots.contains(press.entity) {
            " (hot)"
        } else {
            ""
        }
    );
}

fn on_click(click: On<'_, '_, Pointer<Click>>, rig: Res<'_, Rig>, hots: Query<'_, '_, &HotSpot>) {
    if click.event.button != PointerButton::Primary || rig.dragged > 3.0 {
        return;
    }
    if let Ok(HotSpot(hot)) = hots.get(click.entity) {
        info!("click: {hot:?}");
        bridge::emit(&Outbound::Click { hot: hot.clone() });
    }
}

fn on_over(
    over: On<'_, '_, Pointer<Over>>,
    mut hovering: ResMut<'_, Hovering>,
    hots: Query<'_, '_, &HotSpot>,
) {
    if let Ok(HotSpot(hot)) = hots.get(over.entity) {
        hovering.0.insert(over.entity);
        let at = over.pointer_location.position;
        bridge::emit(&Outbound::Hover {
            tip: hot.tip.clone(),
            x: at.x,
            y: at.y,
        });
    }
}

fn on_out(out: On<'_, '_, Pointer<Out>>, mut hovering: ResMut<'_, Hovering>) {
    if hovering.0.remove(&out.entity) && hovering.0.is_empty() {
        bridge::emit(&Outbound::Leave);
    }
}

/// Grows every shape of a hovered figure a touch, as one. Runs after
/// [`animate`], which sets a glowing prop's scale afresh each frame: that
/// fresh pulse is what the growth multiplies, and a resting prop grows from
/// its rest — never from last frame's value, so nothing compounds.
fn hover_grow(
    hovering: Res<'_, Hovering>,
    groups: Query<'_, '_, &Group>,
    mut props: Query<'_, '_, (&Group, &RestScale, Option<&Animated>, &mut Transform)>,
) {
    let hot: HashSet<u32> = hovering
        .0
        .iter()
        .filter_map(|entity| groups.get(*entity).ok())
        .map(|group| group.0)
        .collect();
    for (group, rest, animated, mut transform) in &mut props {
        let pulsing = animated.is_some_and(|a| matches!(a.anim, Anim::Glow { .. }));
        let base = if pulsing { transform.scale } else { rest.0 };
        let target = hover_scale(base, hot.contains(&group.0));
        if transform.scale != target {
            transform.scale = target;
        }
    }
}

/// A figure under the pointer grows by six percent.
fn hover_scale(base: Vec3, hovered: bool) -> Vec3 {
    if hovered { base * 1.06 } else { base }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn positions(mesh: &Mesh) -> Vec<[f32; 3]> {
        match mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
            Some(bevy::mesh::VertexAttributeValues::Float32x3(v)) => v.clone(),
            _ => panic!("positions are float3"),
        }
    }

    #[test]
    fn a_rounded_box_stays_inside_its_box_and_reaches_its_faces() {
        let mesh = rounded_box(2.0, 1.0, 0.5, 0.1);
        let pts = positions(&mesh);
        assert!(pts.len() > 100);
        for p in &pts {
            assert!(
                p[0].abs() <= 1.0 + 1e-5 && p[1].abs() <= 0.5 + 1e-5 && p[2].abs() <= 0.25 + 1e-5,
                "{p:?}"
            );
        }
        let reach = |axis: usize| pts.iter().map(|p| p[axis].abs()).fold(0.0_f32, f32::max);
        assert!(
            (reach(0) - 1.0).abs() < 0.02,
            "touches the x faces: {}",
            reach(0)
        );
        assert!(
            (reach(1) - 0.5).abs() < 0.02,
            "touches the y faces: {}",
            reach(1)
        );
        assert!(
            (reach(2) - 0.25).abs() < 0.02,
            "touches the z faces: {}",
            reach(2)
        );
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("u32 indices")
        };
        assert_eq!(indices.len() % 3, 0);
        assert!(indices.iter().all(|&i| (i as usize) < pts.len()));
    }

    #[test]
    fn every_face_of_a_rounded_box_points_outward() {
        // The engine culls faces wound clockwise; a mesh wound the wrong way
        // vanishes and leaves only its black outline shell behind.
        let mesh = rounded_box(1.0, 0.6, 0.8, 0.1);
        let pts = positions(&mesh);
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("u32 indices")
        };
        let mut inward = 0;
        for tri in indices.chunks(3) {
            let v = |k: usize| Vec3::from(pts[tri[k] as usize]);
            let (a, b, c) = (v(0), v(1), v(2));
            let normal = (b - a).cross(c - a);
            if normal.dot(a + b + c) < 0.0 {
                inward += 1;
            }
        }
        assert_eq!(
            inward,
            0,
            "{inward} of {} triangles face inward",
            indices.len() / 3
        );
    }

    #[test]
    fn a_rounding_too_large_for_the_box_is_clamped_to_a_capsule_like_shape() {
        let mesh = rounded_box(0.2, 1.0, 0.2, 0.5);
        for p in positions(&mesh) {
            assert!(
                p[0].abs() <= 0.1 + 1e-5 && p[2].abs() <= 0.1 + 1e-5,
                "{p:?}"
            );
        }
    }

    #[test]
    fn mesh_keys_tell_sizes_apart_and_unit_shapes_share_one() {
        assert_ne!(
            mesh_key(Shape::Rounded {
                w: 1.0,
                h: 1.0,
                d: 1.0,
                r: 0.1
            }),
            mesh_key(Shape::Rounded {
                w: 1.0,
                h: 1.0,
                d: 1.5,
                r: 0.1
            })
        );
        assert_eq!(
            mesh_key(Shape::Sphere { r: 0.2 }),
            mesh_key(Shape::Sphere { r: 3.0 })
        );
        assert_eq!(
            mesh_key(Shape::Cylinder { r: 0.2, h: 1.0 }),
            mesh_key(Shape::Cylinder { r: 1.0, h: 2.0 })
        );
        assert_ne!(
            mesh_key(Shape::Cylinder { r: 1.0, h: 1.0 }),
            mesh_key(Shape::Cone { r: 1.0, h: 1.0 })
        );
        assert_eq!(
            mesh_key(Shape::Card {
                w: 1.0,
                h: 1.5,
                figure: Figure::Steward
            }),
            mesh_key(Shape::Card {
                w: 1.2,
                h: 1.6,
                figure: Figure::Follower {
                    model: sprites::Model::Opus
                }
            }),
            "every card is the one quad, scaled"
        );
    }

    #[test]
    fn a_card_faces_the_camera_and_stands_up_the_screen() {
        let rotation = billboard_rotation();
        let normal = rotation * Vec3::Z;
        assert!(
            (normal - camera_direction()).length() < 1e-5,
            "the quad's normal points at the camera: {normal}"
        );
        let up = rotation * Vec3::Y;
        assert!(
            (up - Vec3::from(scene::CAMERA_UP)).length() < 1e-5,
            "the quad's top is the screen's up: {up}"
        );
    }

    #[test]
    fn a_hovered_figure_grows_from_its_rest_and_an_idle_one_is_at_rest() {
        let rest = Vec3::new(1.2, 1.8, 1.0);
        assert_eq!(hover_scale(rest, false), rest);
        let grown = hover_scale(rest, true);
        assert!((grown.x / rest.x - 1.06).abs() < 1e-6, "{grown}");
        assert!((grown.y / rest.y - 1.06).abs() < 1e-6, "{grown}");
        // Applied again from the rest, not from the grown value: no compounding.
        assert_eq!(hover_scale(rest, true), grown);
    }

    #[test]
    fn the_outline_grows_by_a_constant_thickness_whatever_the_size() {
        let big = Vec3::ONE + Vec3::splat(OUTLINE) / half_extents(Shape::Sphere { r: 2.0 });
        let small = Vec3::ONE + Vec3::splat(OUTLINE) / half_extents(Shape::Sphere { r: 0.1 });
        assert!((big.x - (1.0 + OUTLINE / 2.0)).abs() < 1e-4, "{big}");
        assert!((small.x - (1.0 + OUTLINE / 0.1)).abs() < 1e-4, "{small}");
        assert!(
            small.x > big.x,
            "a small prop grows more, in its own units, for the same line"
        );
        let slab = half_extents(Shape::Rounded {
            w: 4.0,
            h: 0.1,
            d: 2.0,
            r: 0.05,
        });
        assert_eq!(slab, Vec3::new(2.0, 0.05, 1.0));
    }
}
