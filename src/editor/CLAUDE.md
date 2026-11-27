# Notes: Map editing

Part of the repo's notes for Claude Code (conventions in `/repo/CLAUDE.md`). Loaded when working in `src/editor/`; keep it up to date.

## Map editing (agreed with the user, 2026-11-18)

The map is ours: `map/gerach.osm` (imported from OSM once, edited by the user in JOSM,
committed; `upload='never'`) is the only input of the bake besides the lidar. `bake
import` adds new areas without touching what's there (deleted elements stay in the file
as `action='delete'`). Tags we add go into the map file and `KEEP` in
`tools/bake/src/osm.rs`. The plan, in this order:

1. Map file (done).
2. Surfaces (done): `physics/surface.rs`, grip share and rolling resistance factor per
   `Surface`; road kinds from OSM `surface`, untagged tracks by `tracktype` (the user
   checked: OSM's tags are right, keep the rules); else by land use. `Terrain::flat` is
   paved, so driving tests that would leave a road use it (`flat_road` has one road).
3. Roads as their own geometry (done): bake `roads.rs` (spline centrelines 1 m apart,
   crown, terrain cut and filled, carved under bridge decks) and `profile.rs` (network
   least squares, block SOR over chains of roads running on into each other). Game:
   `Terrain::ground(x, z, y)` blends overlapping roads (weights fade at road ends;
   roads sharing a point, `Road::meets`, also out to `JOIN_REACH` (2 m) past their edges,
   each weighted by its margin + 2 m times how near both edges are, since 2026-11-23: at
   a Y the edges parted up to 35 cm apart in height, a step and a hole the user found at
   (−2644.4, −2216.4); the bake's profile is too stiff for ties at the tips to fix it,
   tried) and
   has levels (a deck carries what's within a metre below its surface); `sample` is the
   level beneath any bridge, `land` the bare terrain. Bridges are drawn as slab and
   parapets; parapets are obstacles. Width changes (asked for by the user, 2026-11-22;
   format 15): where exactly two roads on the ground meet end to end (no road passing
   through the node) and differ by 0.25 m or more, the narrower one widens to the wider
   over 8 m per metre of difference (6–30 m, at most half the road) with a smoothstep
   (`roads::taper`, `Road::ends`, the game's `Road::width_at`; 123 ends). Where two road
   ends meet at an angle, both ribbons end on the mitre (`join_ends`), so no wedge opens
   on the outside of the bend. Curbed corners (asked for by the user, 2026-11-22; format
   16; `tools/bake/src/corners.rs`): at nodes where 3+ roads on the ground leave (none
   with a bridge), between each two neighbouring ones 25–160° apart, the inside of the
   turn is filled to an arc of radius by the narrower road (2.5 m under 3.5 m wide, 5 m
   under 6 m, else 8 m; ×0.6, ×0.35 if it doesn't fit in 0.8 of the arms or its arc
   reaches into a house). Since 2026-11-23 (the user: corners didn't meet curved roads
   or odd angles, gaps) built on the roads' edges as drawn (`Arm::stations`, across as
   the game's `Strip::side`): the arc's centre is where both edges moved out by the
   radius first cross, its ends where it touches them; the outline runs from the node
   out along one centreline, across to the arc, round it and back along the other
   centreline, so all but the visible part is under the roads (the cut adds no outline
   segment lying under a road, `Terrain::under_road`: corners either side of a road
   would overlap along its centreline, which spade asserts on). Heights: each arm's
   surface at the nearest point, weighted 1/(distance beyond its edge + 0.05)². The
   game's `Corner`s are
   in the road grid (entries past the roads), count as ground where no road covers, and
   are part of the ground mesh since 2026-11-23 (see below). **The ground is cut out under the roads** (asked
   for by the user, 2026-11-22: no seeing under a road's edge, no terrain through it;
   `src/terrain/cut.rs`, `RoadRibbons::ground_mesh`): per chunk, the grid cells a
   ribbon's or corner's outline crosses go into a constrained Delaunay triangulation
   (crate `spade`) with their corners, sides and diagonals (so the rest stays the grid's
   triangles) and the outlines (`add_constraint_and_split`); pieces whose centroid is
   inside a ribbon segment are dropped, those inside a corner fan are the corner (road
   colour); a point on a ribbon outline gets the drawn ribbon's height, else on a
   corner outline the corner's, else inside a corner its surface, else the grid's.
   Corners were separate fans before, 4 mm under the ribbons: with nothing beneath
   them, a corner lower than the blended ribbon over it opened a gap to see under the
   map (the user found one at (−2646, −2217)). Corner surfaces follow the ribbons:
   `Terrain::settle_corners` divides each spoke (first point to an outline point)
   ~1.5 m apart at load, heights on a road = the roads' blend (`ground_and_roads`), off
   the roads every near road's drawn surface just inside its nearest edge, weighted
   1/(distance + 0.05)² (no baked heights: those made it lean towards the terrain;
   threaded natively; physics uses the same `Corner::at`). `rays_never_pass_under_the_roads` casts low rays over junction
   blocks (`rays_at_many_junctions`, ignored, every hundredth junction); slits under
   1 cm don't count (where two overlapping roads' edges cross at heights a few mm
   apart, the ground can meet only one). Not cut:
   bridges, waterways. The ribbon segment is shared
   code (`Strip`), so outline and ribbon agree exactly; its last row is exactly the next
   segment's first. ~0.8 ms a chunk. Spade asserts on degenerate input: an outline
   added twice once split (so outlines are deduplicated, and a road's end row isn't one
   where it runs on into another road's end); `every_chunk_is_cut` cuts the whole map
   (the web build aborts on a panic; natively a failed chunk stays uncut). Tests also
   check the ground is closed up to the outlines and nothing is uncovered from above
   (sample points within ~1 mm: the mesh is f32, a quarter millimetre apart at x 3000).
   The physics' `land` still is the grid's triangles, so right beside a road the drawn
   ground differs from it by the edge's mismatch. The bake's `conform` (`Near`): under
   a road up to `SINK` below the lowest road over it (out of sight), within the 1 m
   shoulder level with the edge, blended between several roads by inverse distance⁴
   (so an island between roads at different heights slopes instead of a wall at one
   edge), beyond that `EMBANKMENT` from the nearest. A 2 m shoulder was tried and
   dropped (terrain rose under bridge ends). The bake is deterministic (BTreeMaps where
   it iterates; the same map bakes to the same bytes). Overlapping roads are one
   surface in the profile solve (`tie_overlaps`, 2026-11-23): at five places across
   each road point lying ≥ 0.25 m inside another road on the ground, connected or not,
   a tie (`Problem::ties`: weighted terms with a target, solved like the bends) pulls
   the two surfaces (crowns included) together; road ribbon triangles steeper than 26°
   went from 352 m² to 44 m² (waterways on steep banks aside).
   **No two ribbons on top of each other** (asked for by the user, 2026-11-23: a cobbled
   lane showed through Grubenweg at (−2945.5, −1939.2)): `RoadRibbons::mesh` clips each
   ribbon triangle against the better roads' ribbons over it (`Strip::covered`,
   `outranks`: kind as `RoadKind::ALL`, then width, then index; bridges and waterways
   neither), leaving `SEAM_OVERLAP` (3 cm) under their edge against slits. The cut
   needs no change (the ribbons' union is the same); the physics' surface is the best
   covering road's. `join_ends` also mitres where more than two ends meet (the
   straightest pair of equal width; unequal widths on a mitre leave the cut open edges),
   so a road split at a side road's node has no wedge. ~5 % more meshing time.
   `where_roads_overlap_only_the_best_is_drawn` checks three chunks by colour.
   Not done: junction middles as one mesh (the overlapping ribbons' facets show faint
   shading seams), tunnels, piers.
4. Road condition (done): `relief.rs`, OSM `smoothness` to ISO 8608 roughness (waves
   0.5–20 m), features from `traffic_calming=bump|hump|table` and `hazard=pothole`
   nodes. The baked file is zlib-compressed, heights predicted from neighbours.
5. Roads off buildings (done): bake `clearance.rs` moves roads sideways (≤ 2 m, tapered,
   shared points pinned) and writes what's left to `map/gerach.conflicts.gpx`
   (gitignored), for JOSM. 303 overlaps → 106: 50 near junctions, 46 between houses
   narrower than the road (wants a `width` tag), a few needing more than 2 m.
6. Fences (done; the user dropped trenches): `barrier` ways (imported 2026-11-18:
   184 guard rails, 35 walls, 32 fences, 15 hedges, 14 railings) baked by
   `tools/bake/src/fences.rs` (gates cut as gaps, barriers along roads pushed 0.5 m
   off the edge, those along bridges dropped, the rest on roads listed in the conflicts
   file) into format v12; the game's `src/fences.rs` draws six kinds in pieces ≤ 2.5 m
   on `Terrain::sample` and puts a solid box per piece in the obstacle grid. Agreed:
   solid (not breakable), gates open, only what's in the map (no generated rails).
7. Map editor (agreed 2026-11-18, built the night after): see "Map editor" below.

### Map editor (step 7, agreed with the user)

A third app, `src/bin/editor.rs` (`cargo run --bin editor`), native only, its own
plugin list on `CorePlugin`'s shared pieces but **no cars** (no simulation, traffic,
HUD, sound). Decisions:

- **Edits go straight into `map/gerach.osm`**: the editor reads and writes it itself
  (the bake's `osm.rs` model and JOSM-style writer, shared), with its own undo/redo and
  Save. No JOSM connection at all. Written the way JOSM writes: new elements get
  negative ids, changed ones `action='modify'`, deleted ones stay as
  `action='delete'`, so JOSM can still open the file and `bake import` still merges.
- **Camera**: fly. Hold RMB to look, WASD to move, Q/E down/up, Shift faster, mouse wheel
  sets the speed. Left click selects and edits. Teleport via the 2D map (M, click).
- **Edits, all of them**: tags on ways (roads: smoothness, width, surface, highway,
  bridge…; fences: barrier, fence_type, height; buildings: levels; areas: kind), point
  features (bumps, humps, tables, potholes, gates: place, move, delete), geometry (move,
  add, delete nodes; draw new roads, fences, buildings, areas; split and delete ways),
  road heights (`ele` on a node pins the road's height: a hard constraint in the bake's
  profile solve; raised and lowered in 3D), land-use areas (draw and edit closed ways for
  paved areas, forest, fields, meadows, water…).
- **Terrain areas** (closed ways with our own tags, applied by the bake to the lidar
  before roads are fitted): flatten to a height, raise/lower by an amount (faded at the
  edge), smooth, and slope (a plane through heights set at its corners).
- **Re-bake on a key/button**: save, run the full bake (~20 s, progress shown) and
  reload the map in place.
- **The driving apps load `assets/gerach.terrain` at startup** on native (embedded copy
  as fallback and for the web), so a bake shows up without recompiling.
- Inspector: what's under the cursor (way id, tags), and the conflict markers from the
  bake shown in 3D with next/previous.

Built (all in `src/editor/`, see its module docs and README "The map editor"; nothing
of it tried with a real mouse yet, only headless): `doc.rs` (the `Osm` with node
positions, ways per node, `Change`-based undo via `Doc::edit`), `index.rs` (`Class`
of a way by tags, `PointKind`, 64 m grid rebuilt per revision), `overlay.rs` (gizmo
lines draped 0.3 m above `Terrain::sample`, cached per revision; hover picks the
nearest node within 12 px × 1.3, else segment, in ground metres), `tools.rs` (`Tool`,
`Command` shared by keys and panel), `panel.rs` (an `egui::Area` top right: a
`Window` kept its old size and scrolled), `bake.rs` (`sh -c`: `cargo build --release`
in `tools/bake` without `CARGO_TARGET_DIR` (the dev shell's `cargo run` adds
`--profile`), then the binary; on success `Terrain::parse` + `scenery::Reload` +
`MapView::rebuild`), `conflicts.rs`. Decisions made alone: tags `carsim:terrain`
(flatten|raise|smooth|slope), `carsim:height` (m above sea level), `carsim:offset`,
`carsim:radius`, `carsim:fade` (default 5 m, inside the outline); slope corners and
pins are `ele` (above sea level; baked format 13 on stores `Terrain::base`, the lowest
point's height, to convert). egui input is **not** absorbed in the editor (bevy_egui
then resets the mouse buttons whenever egui is over the pointer, dropping drags):
systems check `EguiWantsInput` themselves. The bake is shared code-wise: `src/osm.rs`
is `#[path]`-included by `tools/bake` (std + quick-xml only). Map file ids: new
elements without `version` (JOSM rejects one on new elements).
