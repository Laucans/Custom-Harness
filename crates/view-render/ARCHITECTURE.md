# `harness-view-render` — architecture

The plant, drawn. A [Bevy](https://bevy.org) scene — real 3D under an
orthographic camera at the isometric corner — compiled to WebAssembly and
bound to the `<canvas>` of `harness-view`'s page. It replaces the hand-written
canvas engine the first version shipped with.

```
harness-view (bin, axum)  ──serves──▶  static/render/{render.js, render_bg.wasm}
                                              ▲
harness-view-render (cdylib → wasm) ──────────┘   scripts/build-render.sh
```

The two crates share **no Rust type**: `harness-view` pulls `tokio` with
process features, which has no place in a browser bundle, so the renderer
mirrors the few fields of the picture it draws in its own `model.rs`, every
one defaulted. A picture from a newer server still draws what this version
knows.

## Who does what

| | the page (`app.js`) | the renderer (this crate) |
| --- | --- | --- |
| data | receives the picture over SSE, pushes it as JSON | parses it, draws it |
| navigation | owns level and room, the URL hash, the breadcrumbs | is told the viewpoint, frames it |
| interaction | opens panes, GitHub, the steward's terminal | reports clicks and hovers as `{kind, hot}` |
| chrome | the HUD, the panes, the tooltip, xterm.js | nothing — it owns the canvas and only the canvas |

The `hot` a click reports is the same shape the canvas engine used
(`go`/`room`, `pane`, `url`, `tip`), so every pane in `app.js` survived the
migration untouched.

## The modules

```
src/lib.rs      the crate's lints and the two halves
src/model.rs    the picture, mirrored with serde (all fields default)
src/scene.rs    PURE: picture + viewpoint → props, labels, hotspots, bounds — the plant's shape, tested natively
src/app.rs      Bevy: three shared meshes, materials cached by colour, camera rig, picking, labels, animation
src/bridge.rs   the inbox the page writes (last picture, last viewpoint) and the events the scene emits
src/web.rs      wasm-bindgen: start / setSnapshot / setView, and the callback to window.harnessRender.onEvent
```

`scene.rs` is where the plant is designed and where it is tested: every
level is built from a fixture and asserted on — which hotspots exist, where
the product sits, what the control room's panel says. `app.rs` only turns
that list into entities, so a change of look is a change in one file and a
test in the same file.

## The look: a paper cult in the atomic age

The art direction is borrowed from *Cult of the Lamb*: flat, bright colours
under thick ink outlines, nothing with a sharp corner, big round heads on
short bodies. The palette is a town's — red brick under cream copings,
slate roofs, pavement and tiles under foot, warm wood, gold for what
shines, teal as the accent — one palette for all three levels, so the
inside reads as the same building: brick back walls with factory windows,
low brick partitions, tiled floors (warm where a room runs, greyer while
drafted, raw while built) on a dark concrete hall, slate-rubber belts. The ink is a dark plum, never black; the only dark surface is
the dusk-teal sky behind the plant, so everything in front of it reads as
lit. The props wear that palette in *Fallout*'s retro-futurism: chrome
rings, rivets and skirts, portholes, radar dishes and red beacons, a vault's
gear door for the entrance, CRT terminals for printers and screens, a
Protectron for a robot, a Vault Boy for the steward. The plant itself is
*Futurama*'s Planet Express — a brick hangar under dark barrel vaults
framed by red arches, beside a tall tapering brick tower with a balcony, a
red ribbed dome and a gold spire, the project's name on a gantry over the
hangar's front. It stands in a town, not a field: paved ground, a street
with kerbs, a dashed line and a zebra crossing, lamps and trees, a brick
neighbour with a water tank, a plank fence, a tulip bed, and on the right
the quay — stone edge, bollards, an outflow pipe, a boat bobbing on water
that moves. In 3D that becomes:

- **Shapes** — `scene.rs` draws with rounded boxes, capsules, cones, cut
  cones, spheres and rings (a gate is an arch of torus over the belt, a roof
  is a row of barrel vaults). The only sharp boxes are floors. The rounded
  box is a custom mesh (`app::rounded_box`): a sphere whose vertices are
  pushed out to the box's corners, so the sphere's normals stay right.
- **Outlines** — every opaque prop has a child: the same mesh grown by a
  constant world thickness, black, unlit, front faces culled. The inverted
  hull reads as an inked line at any zoom and costs one extra draw per prop.
  It is `Pickable::IGNORE`, so a click still lands on the prop.
- **Ink wash** — `ToonMaterial` (`toon.wgsl`) lights a flat colour in three
  bands from a fixed corner, plus a glow for lamps, orbs and screens.
- **Patterns** — a prop may wear a `scene::Pattern` (brick, pavers, planks,
  a roof's seams, water) that the same shader paints in world space, on the
  plane the surface faces most: no texture file, no UV, and a wall of any
  size gets bricks of one size. Water reads the engine's clock and moves.
  The material cache is keyed by colour *and* pattern. Outside, the walls,
  roofs, fence, ground and harbour wear one; inside, the walls are brick
  and the floors tiled, nothing else. Smoke
  and the scanner's disc stay on the engine's material, since they are
  see-through, and wear no outline.
- **Figures** — followers, robots and the steward are not built from shapes
  but painted, the way the game and *Don't Starve* stand paper characters in
  a world. `sprites.rs` draws each one with vector paths (a vault jumpsuit
  in the model's colour with gold belt and trim, a big head with bead eyes
  and pink cheeks under a domed gold hard hat; a Protectron robot — domed
  head, a visor whose slits glow green while it works, a riveted chest
  plate, a wrench or a clipboard in its chrome hand; the steward as the
  Vault Boy — blue jumpsuit, blond quiff, a wink and a thumbs-up) and
  `tiny-skia` rasterises it into a 256×384 RGBA texture at run time, once per
  figure, the first time it is needed — no image file is shipped. The scene
  places a `Shape::Card` for it: a quad that `app.rs` turns with the camera's
  own rotation so it always faces the viewer, standing along the screen's up
  (`scene::CAMERA_UP`) so its feet stay on the ground as seen, over a soft
  translucent disc of shadow. The card's material is unlit and alpha-masked,
  so it sorts against the props like any solid thing and needs no outline:
  the ink is in the painting. Since the painting is pure Rust, the figures
  are tested natively — a transparent margin, thick ink, a suit per model,
  green visor slits on a working robot.

## Rendering choices

- **Orthographic, from (1, 1, 1).** The classic isometric corner, 35° down.
  One tile is one unit, one storey one unit. Pan by dragging, zoom with the
  wheel around the pointer; a new level frames itself. Tonemapping is off.
- **Meshes cached by shape.** A unit sphere, cylinder and cone are scaled per
  prop; rounded boxes, capsules, cut cones and rings are built once per size
  (to the centimetre), since scaling would distort their rounding. Materials
  are cached by colour.
- **Rebuilt, not diffed.** Every new picture or viewpoint despawns the scene
  and spawns it again. A few hundred entities a few times a minute is cheap;
  what must survive — where the product was, so it slides — is remembered by
  key.
- **Labels are UI.** Text is a Bevy UI node pinned every frame to its world
  anchor with `world_to_viewport`; it stays crisp at any zoom and never
  flips. A label has one of three backings (`scene::Backing`): none, for a
  stage name written on the ground; an ink pill, for a tag hanging in the
  world (a name over a figure, a room's number); a grey, see-through plate,
  for text written on a board — the sign, the control room's panel, the
  mock-up. The plate is what keeps a line legible on a board that, leaning
  away in the isometric view, is narrower on screen than the line itself.
- **Picking is the engine's.** `MeshPickingPlugin` with `require_markers`,
  observers on `Pointer` events. A `HotSpot` component on what reacts, a
  blocking, non-hoverable `Pickable` on the opaque props that may stand in
  front of it, and no marker at all on the outline shells, the smoke and the
  belts' stripes — half the meshes are never ray cast. The shapes of one
  figure (a door's frame and leaves, a robot and its shadow) share a
  `Group`, given by the scene's `hot_since`; a hovered figure grows by six
  percent *as one*, in `hover_grow`, from each shape's rest scale — or, for
  a glowing shape, from the pulse `animate` has just set — so nothing
  compounds and no single leaf ever swells on its own.
- **In front means +z (and +x, +y).** The camera looks from (1, 1, 1), so
  the face toward the viewer is the one with the larger coordinate. A door
  leaf on its frame, a card on its board, a screen on its desk, a label on
  a face: each sits at a *larger* `z` (or `y`) than what carries it, or the
  carrier hides it; and two slabs never share a top face where they overlap,
  or they flicker. The scene's tests hold both rules.

## Building it

```
rustup target add wasm32-unknown-unknown
brew install wasm-pack              # fetches a matching wasm-bindgen
scripts/build-render.sh             # wasm-dev: the iteration build → crates/view/static/render/
scripts/build-render.sh wasm-release   # the small bundle, minutes of LTO + wasm-opt
```

Two profiles, because the slow part of a release build is not the
dependencies — cargo caches those under `target/` — but the final link: fat
LTO over the whole of Bevy in one codegen unit, then `wasm-opt`. `wasm-dev`
keeps the dependencies optimised (and cached, once compiled for that
profile) and compiles only this crate fast, with no LTO and no `wasm-opt`:
a change here rebuilds in well under a minute, for a bundle five times
larger (about 90 MB) that the browser then takes several seconds to compile
before the first frame. The page serves whichever bundle was built last, so
build `wasm-release` before anyone opens the plant for real: a slow first
render is almost always the dev bundle.

The bundle is a build artifact and is not committed; the page says what to
run when it is missing. `cargo build --workspace` compiles this crate
natively too (that is what `cargo test` and clippy check), which is why its
Bevy features are chosen per target in `Cargo.toml`.

The script does three things a plain `cargo build` would get wrong on a Mac
with a Homebrew Rust: it puts the rustup toolchain first (Homebrew's `cargo`
has no wasm32 standard library), it sets `DYLD_FALLBACK_LIBRARY_PATH` so
rustup's `rust-lld` finds `libLLVM.dylib`, and it runs `wasm-opt` itself with
`--all-features` (wasm-pack's own call refuses the bulk-memory instructions
rustc now emits; it is disabled in the crate metadata). Result: ~17 MB,
served once per page load from `127.0.0.1`.

**In the browser console**: Bevy's warnings, one `scene:` line per rebuild
(viewpoint, props, labels) and one `press on …` line per click naming the
entity the ray hit — `(hot)` when it is a hotspot. That line is the first
thing to read when a click seems lost.

Both profiles live in the workspace `Cargo.toml`: `wasm-release`
(`opt-level = "z"`, fat LTO, one codegen unit) and `wasm-dev` (`opt-level =
1` here, `3` for every dependency, no LTO, incremental).
