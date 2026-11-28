//! Real-world terrain baked by `tools/bake`: a heightfield rendered flat-shaded and queried
//! analytically (the physics sees exactly the triangles that are drawn, but right beside
//! the roads, where the ground is cut to meet their edges: `cut`), and roads with their
//! own heights on it (`Terrain::ground`), drawn as ribbons on that same surface. Meshes
//! are built per chunk, on demand (see `scenery`).

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::fences::{Fence, FenceKind};
use crate::physics::collision::Obstacles;
use crate::physics::surface::Surface;
use crate::relief::{Feature, Waves};

mod cut;

/// From Gerach (96161, Bavaria) to Rattelsdorf, Ebing and Baunach: 9.6 × 8.4 km.
const BAKED: &[u8] = include_bytes!("../assets/gerach.terrain");
/// The same, as a path from the repo's root (the native apps read it at startup).
pub const TERRAIN_FILE: &str = "assets/gerach.terrain";
/// Width of the raised rim at the map edge (baked into the heights; roads stop before it).
const RIM: f32 = 30.0;
/// How far waterway ribbons float above the terrain, to avoid z-fighting.
const WATER_LIFT: f32 = 0.05;
/// Maximum spacing of road ribbon vertices, along and across the road.
const DRAPE_STEP: f32 = 1.0;
/// Cell size of the grid of road segments, for looking up the surface (m).
const ROAD_CELL: f32 = 4.0;
/// A corner's spokes are divided about this finely (m), its surface following the roads'
/// where it meets them (see `Terrain::settle_corners`).
const CORNER_LATTICE: f32 = 1.5;
/// Where roads overlap (junctions), the surface blends them, each by how far inside its
/// edge the point is, fading out over this distance towards the road's ends (m), so the
/// surface stays continuous where a side road's crown meets a main road's camber.
const ROAD_FADE: f32 = 3.0;
/// Roads meeting at a junction also blend into each other out to this far beyond their
/// edges (m): where two roads leaving a junction at different grades part, their edges
/// meet at one height instead of a step, and the surface warps between them over this.
const JOIN_REACH: f32 = 2.0;
/// A surface carries something at most this far below it (m): a car on a bridge is on the
/// deck, one under it on the road below.
const LEVEL_REACH: f32 = 1.0;
/// Where a better road covers a lesser one, the lesser one's ribbon is drawn only up to
/// this far under the better one's edge (m), so no slit opens between them.
const SEAM_OVERLAP: f32 = 0.03;

/// Where the cars start: they're placed on the nearest paved road, facing along it in the
/// direction closest to the heading. Not part of the map: the server says where (alone,
/// it's the default, in Gerach).
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct Spawn {
    /// Map point (x east, z south, m).
    pub at: Vec2,
    /// Compass degrees (0 north, 90 east); none faces the way with more road behind.
    pub heading: Option<f32>,
}

impl Spawn {
    /// In Gerach, facing south.
    pub const DEFAULT: Spawn = Spawn {
        at: Vec2::new(-2984.5, -2007.2),
        heading: Some(180.0),
    };

    /// The heading as a direction in the map frame (x east, z south).
    fn direction(&self) -> Option<Vec2> {
        let h = self.heading?.to_radians();
        Some(Vec2::new(h.sin(), -h.cos()))
    }
}

impl Spawn {
    /// Reads "x z [heading]" (map metres, compass degrees; spaces or commas between).
    pub fn parse(text: &str) -> Result<Self, String> {
        let numbers: Vec<f32> = text
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|s| !s.is_empty())
            .map(|s| s.parse().map_err(|_| format!("not a number: {s}")))
            .collect::<Result<_, _>>()?;
        if numbers.iter().any(|n| !n.is_finite()) {
            return Err(format!("not a point: {text}"));
        }
        match numbers[..] {
            [x, z] => Ok(Spawn {
                at: Vec2::new(x, z),
                heading: None,
            }),
            [x, z, heading] => Ok(Spawn {
                at: Vec2::new(x, z),
                heading: Some(heading),
            }),
            _ => Err(format!("x z [heading], not {text}")),
        }
    }
}

impl Default for Spawn {
    fn default() -> Self {
        Self::DEFAULT
    }
}

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Terrain::load())
            .init_resource::<Spawn>();
    }
}

/// A road's surface, or a waterway (numbered as in the baked file).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoadKind {
    Asphalt,
    Gravel,
    Water,
    Cobbles,
    Dirt,
}

impl RoadKind {
    /// Bottom to top, as the map draws them.
    pub const ALL: [RoadKind; 5] = [
        RoadKind::Water,
        RoadKind::Dirt,
        RoadKind::Gravel,
        RoadKind::Cobbles,
        RoadKind::Asphalt,
    ];

    fn from_byte(byte: u8) -> Self {
        [
            RoadKind::Asphalt,
            RoadKind::Gravel,
            RoadKind::Water,
            RoadKind::Cobbles,
            RoadKind::Dirt,
        ][byte as usize]
    }

    /// Its place in `ALL`: where roads overlap, the higher one is drawn (see `outranks`).
    fn rank(self) -> usize {
        RoadKind::ALL.iter().position(|&k| k == self).unwrap()
    }

    /// Asphalt or stones.
    pub fn paved(self) -> bool {
        matches!(self, RoadKind::Asphalt | RoadKind::Cobbles)
    }

    pub fn surface(self) -> Surface {
        match self {
            RoadKind::Asphalt => Surface::Asphalt,
            RoadKind::Cobbles => Surface::Cobbles,
            RoadKind::Gravel => Surface::Gravel,
            RoadKind::Dirt => Surface::Dirt,
            RoadKind::Water => Surface::Water,
        }
    }

    /// How far the ribbon is drawn above the road's surface, so where roads overlap the
    /// better one shows (m).
    fn lift(self) -> f32 {
        match self {
            RoadKind::Asphalt => 0.012,
            RoadKind::Cobbles => 0.009,
            RoadKind::Gravel => 0.006,
            RoadKind::Dirt => 0.003,
            RoadKind::Water => WATER_LIFT,
        }
    }

    /// Vertex colour of the ribbon (linear).
    pub fn color(self) -> [f32; 4] {
        match self {
            RoadKind::Asphalt => [0.24, 0.24, 0.26, 1.0],
            RoadKind::Cobbles => [0.36, 0.34, 0.33, 1.0],
            RoadKind::Gravel => [0.56, 0.49, 0.38, 1.0],
            RoadKind::Dirt => [0.42, 0.33, 0.24, 1.0],
            RoadKind::Water => [0.22, 0.42, 0.58, 1.0],
        }
    }
}

pub struct Road {
    pub kind: RoadKind,
    /// The width but at tapered ends (see `ends`, `width_at`).
    pub width: f32,
    pub bridge: bool,
    /// Per end (first point, last point): the width there and the length it tapers to
    /// `width` over (0: no taper), where the road runs on into a wider one.
    pub ends: [(f32, f32); 2],
    /// Per end: the direction (first point to last) its ribbon ends square to, where it
    /// meets another road's end at an angle (halfway between the two), so no wedge opens
    /// between them on the outside of the bend.
    joins: [Option<Vec2>; 2],
    /// The other roads on the ground it shares a point with (sorted): they blend into each
    /// other near where they meet (`JOIN_REACH`).
    meets: Vec<u32>,
    /// The surface falls this much per metre from the middle to the edges.
    pub crown: f32,
    /// Centreline in the xz plane (x east, z south), about a metre apart.
    pub points: Vec<Vec2>,
    /// The centreline's height at each point (waterways: zero; they lie on the terrain).
    pub heights: Vec<f32>,
    /// Distance along the road to each point.
    along: Vec<f32>,
    /// How uneven its surface is (ISO 8608's displacement spectrum at 0.1 cycles per
    /// metre, in 10⁻⁶ m³: class A is 16), and that unevenness.
    pub roughness: f32,
    waves: Waves,
    /// Speed bumps and potholes, by how far along the road they are.
    pub features: Vec<Feature>,
}

impl Road {
    /// The width `along` metres from the first point: `width`, widened towards each
    /// tapered end's width with a smoothstep (as the bake's `Road::width_at`).
    pub fn width_at(&self, along: f32) -> f32 {
        let total = self.along[self.along.len() - 1];
        let mut width = self.width;
        for (&(end, length), from_end) in self.ends.iter().zip([along, total - along]) {
            if length > 0.0 {
                let t = (1.0 - from_end / length).clamp(0.0, 1.0);
                width += (end - self.width) * t * t * (3.0 - 2.0 * t);
            }
        }
        width
    }

    /// The widest the road gets.
    pub fn widest(&self) -> f32 {
        self.ends.iter().fold(self.width, |w, e| w.max(e.0))
    }

    /// The unevenness and the features at `along` metres along and `right` of the
    /// centreline, `margin` inside the edge: height, and slope along and rightward.
    fn relief(&self, along: f32, right: f32, margin: f32) -> (f32, Vec2) {
        let (mut height, mut slope) = self.waves.at(along, right, margin);
        for feature in &self.features {
            let (h, s) = feature.at(along, right);
            height += h;
            slope += s;
        }
        (height, slope)
    }

    /// Whether a feature reaches within `distance` of `along`.
    fn feature_near(&self, along: f32, distance: f32) -> bool {
        self.features
            .iter()
            .any(|f| (f.along - along).abs() < f.reach() + distance)
    }
}

/// Whether road `a` is drawn over road `b` where they overlap (and its surface is the one
/// under the wheels): the better kind, else the wider one, else the later one.
fn outranks(roads: &[Road], a: usize, b: usize) -> bool {
    let key = |r: usize| (roads[r].kind.rank(), roads[r].width, r);
    key(a) > key(b)
}

/// The ground somewhere: its height, upward normal, and what it is.
#[derive(Clone, Copy, Debug)]
pub struct Ground {
    pub height: f32,
    pub normal: Vec3,
    pub surface: Surface,
}

#[derive(Resource)]
pub struct Terrain {
    /// Grid cells along x and z; each cell is two triangles.
    cells_x: usize,
    cells_z: usize,
    cell: f32,
    /// Heights at the full-detail grid vertices, `(cells_x + 1)(cells_z + 1)`, row-major in z.
    heights: Vec<f32>,
    /// Chunk side in cells, and the chunks, row-major in z.
    chunk_cells: usize,
    chunks: Vec<Chunk>,
    /// Height above sea level of the map's y = 0 (its lowest point).
    pub base: f32,
    pub roads: Vec<Road>,
    /// The insides of turns between roads leaving a junction, curbed.
    pub corners: Vec<Corner>,
    /// The road segments and corners near each cell, for `surface`.
    road_grid: RoadGrid,
    pub buildings: Vec<Building>,
    /// Fences, walls, hedges and guard rails.
    pub fences: Vec<Fence>,
    /// Houses and tree trunks as boxes, for collisions (filled in at startup, see
    /// `obstacles.rs`; empty in tests unless built).
    pub obstacles: Obstacles,
    /// A hash of the baked file it was read from (0 for the test grounds): players on a
    /// server must all drive the same map.
    pub fingerprint: u64,
}

struct Chunk {
    /// Resolution as a step in grid cells (1 is full detail).
    step: usize,
    /// Land use class per cell at that resolution, row-major in z (classes as in `tools/bake`).
    classes: Vec<u8>,
}

/// The inside of a turn between two roads leaving a junction, filled out to an arc: a fan
/// of triangles around its first point (x, height, z). Its surface is divided along the
/// fan's spokes (from the first point to each of the arc's), and its heights there follow
/// the roads' surface where they meet them (`Terrain::settle_corners`).
pub struct Corner {
    pub kind: RoadKind,
    pub points: Vec<Vec3>,
    /// How many pieces each spoke is divided into, and per spoke the heights at its
    /// division points, from the first point out (empty: the fan's flat triangles).
    divisions: usize,
    spokes: Vec<Vec<f32>>,
}

impl Corner {
    fn new(kind: RoadKind, points: Vec<Vec3>) -> Self {
        Self {
            kind,
            points,
            divisions: 1,
            spokes: Vec::new(),
        }
    }

    /// Division point `k` of spoke `s` (to arc point `s`), at its height.
    fn spoke_point(&self, s: usize, k: usize) -> Vec3 {
        let (e, b) = (self.points[0], self.points[s + 1]);
        let p = e + (b - e) * (k as f32 / self.divisions as f32);
        match self.spokes.get(s) {
            Some(heights) => p.with_y(heights[k]),
            None => p,
        }
    }

    /// Its fan's triangles in the xz plane (heights as the fan's).
    fn fan(&self) -> impl Iterator<Item = [Vec3; 3]> + '_ {
        let e = self.points[0];
        self.points[1..].windows(2).map(move |w| [e, w[0], w[1]])
    }

    /// Its outline at the height it's drawn at (the ground's, `cut`, follows it): from the
    /// first point out along the first spoke, round the arc, and back along the last.
    fn outline(&self) -> Vec<Vec3> {
        let (n, last) = (self.divisions, self.points.len() - 2);
        let lift = Vec3::Y * self.kind.lift();
        let mut out: Vec<Vec3> = (0..=n).map(|k| self.spoke_point(0, k)).collect();
        out.extend((1..=last).map(|s| self.spoke_point(s, n)));
        out.extend((0..n).rev().map(|k| self.spoke_point(last, k)));
        out.iter().map(|p| *p + lift).collect()
    }

    /// Height and upward normal of its surface at `p`, if it's inside.
    fn at(&self, p: Vec2) -> Option<(f32, Vec3)> {
        let e = self.points[0];
        for (t, w) in self.points[1..].windows(2).enumerate() {
            let (b, c) = (w[0], w[1]);
            let (eb, ec) = (b.xz() - e.xz(), c.xz() - e.xz());
            let det = eb.perp_dot(ec);
            if det.abs() < 1e-9 {
                continue;
            }
            let ep = p - e.xz();
            let (u, v) = (ep.perp_dot(ec) / det, eb.perp_dot(ep) / det);
            if u >= -1e-4 && v >= -1e-4 && u + v <= 1.0 + 1e-4 {
                // Between the spokes `t` and `t + 1`, in the strip from division `k` out:
                // the quad's triangle the point is in.
                let n = self.divisions;
                let k = (((u + v) * n as f32).max(0.0) as usize).min(n - 1);
                let q = |s, k| self.spoke_point(s, k);
                let (a, b, c, d) = (q(t, k), q(t + 1, k), q(t + 1, k + 1), q(t, k + 1));
                // Split along a–c; the innermost strip is one triangle (a and b are the
                // first point).
                let side = (c.xz() - a.xz()).perp_dot(p - a.xz());
                let turn = (c.xz() - a.xz()).perp_dot(b.xz() - a.xz());
                let tri = if k > 0 && side * turn >= 0.0 {
                    [a, b, c]
                } else {
                    [a, c, d]
                };
                let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
                if normal.y.abs() < 1e-6 {
                    continue;
                }
                let normal = if normal.y < 0.0 { -normal } else { normal };
                let h = tri[0].y
                    - (normal.x * (p.x - tri[0].x) + normal.z * (p.y - tri[0].z)) / normal.y;
                return Some((h, normal));
            }
        }
        None
    }
}

pub struct Building {
    /// Storeys tagged in OSM, or 0 if unknown.
    pub levels: f32,
    /// Footprint outline in the xz plane.
    pub outline: Vec<Vec2>,
}

pub struct RayHit {
    pub distance: f32,
    pub point: Vec3,
    pub normal: Vec3,
}

impl Terrain {
    /// The map the apps start with: on native, the baked file in `assets/` as it is on disk
    /// (so a fresh bake shows up without recompiling), else the copy built into the game.
    pub fn load() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = Self::file() {
            match std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| Self::parse(&b))
            {
                Ok(terrain) => {
                    info!("Map loaded from {}", path.display());
                    return terrain;
                }
                Err(e) => warn!(
                    "Couldn't load {}: {e}; using the built-in map",
                    path.display()
                ),
            }
        }
        Self::baked()
    }

    /// Where the baked map file is on disk: `assets/gerach.terrain` from the working
    /// directory, or in the source tree the game was built from.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn file() -> Option<std::path::PathBuf> {
        [
            std::path::PathBuf::from(TERRAIN_FILE),
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(TERRAIN_FILE),
        ]
        .into_iter()
        .find(|path| path.is_file())
    }

    /// The map built into the game.
    pub fn baked() -> Self {
        Self::parse(BAKED).expect("the built-in map is broken")
    }

    /// Parses a baked map (format described in `tools/bake`). Chunks are stored at
    /// different resolutions; the full-detail height grid is rebuilt from them, with the
    /// vertices of coarse chunks lying on their coarse triangles, so physics queries match
    /// the rendered surface exactly.
    pub fn parse(file: &[u8]) -> Result<Self, String> {
        let inflated = miniz_oxide::inflate::decompress_to_vec_zlib(file)
            .map_err(|_| "the file is corrupt".to_string())?;
        if inflated.len() < 8 || &inflated[..4] != b"CSTR" {
            return Err("not a baked terrain file".into());
        }
        let mut r = Bytes(&inflated);
        r.take(4);
        let version = r.u32();
        if version != 16 {
            return Err(format!("unsupported terrain version {version}"));
        }
        let (cells_x, cells_z) = (r.u32() as usize, r.u32() as usize);
        let cell = r.f32();
        let chunk_cells = r.u32() as usize;
        let base = r.f32();
        let n = cells_x + 1;
        let mut heights = vec![0.0; n * (cells_z + 1)];
        let mut chunks = Vec::new();
        for cz in 0..cells_z / chunk_cells {
            for cx in 0..cells_x / chunk_cells {
                let step = r.take(1)[0] as usize;
                let m = chunk_cells / step;
                let coarse = predicted_heights(&mut r, m + 1);
                for lz in 0..=chunk_cells {
                    for lx in 0..=chunk_cells {
                        let i = (cz * chunk_cells + lz) * n + cx * chunk_cells + lx;
                        heights[i] = on_coarse_grid(&coarse, m, step, lx, lz);
                    }
                }
                let classes = r.take(m * m).to_vec();
                chunks.push(Chunk { step, classes });
            }
        }
        let mut roads: Vec<Road> = (0..r.u32())
            .map(|_| {
                let (kind, bridge) = (RoadKind::from_byte(r.take(1)[0]), r.take(1)[0] != 0);
                let (width, crown, roughness) = (r.f32(), r.f32(), r.f32());
                let ends = [(r.f32(), r.f32()), (r.f32(), r.f32())];
                // Centimetres along, millimetres up: the first point, then steps.
                let count = r.u32() as usize;
                let mut at = [r.u32() as i32, r.u32() as i32, r.u32() as i32];
                let (mut points, mut heights) = (Vec::new(), Vec::new());
                for i in 0..count {
                    if i > 0 {
                        for v in &mut at {
                            *v += r.u16() as i16 as i32;
                        }
                    }
                    points.push(Vec2::new(at[0] as f32, at[1] as f32) / 100.0);
                    heights.push(at[2] as f32 / 1000.0);
                }
                let mut along = vec![0.0];
                for pair in points.windows(2) {
                    along.push(along.last().unwrap() + pair[0].distance(pair[1]));
                }
                // The same bumps whatever else changes in the map.
                let start = (points[0] * 100.0).as_ivec2();
                let seed = (start.x as u32).wrapping_mul(73_856_093)
                    ^ (start.y as u32).wrapping_mul(19_349_663);
                Road {
                    kind,
                    width,
                    bridge,
                    ends,
                    joins: [None; 2],
                    meets: Vec::new(),
                    crown,
                    points,
                    heights,
                    along,
                    roughness,
                    waves: Waves::new(roughness, seed),
                    features: Vec::new(),
                }
            })
            .collect();
        join_ends(&mut roads);
        find_meetings(&mut roads);
        for _ in 0..r.u32() {
            let road = r.u32() as usize;
            let kind = Feature::kind_from_byte(r.take(1)[0]);
            let (along, right, size, height) = (r.f32(), r.f32(), r.f32(), r.f32());
            roads[road].features.push(Feature {
                kind,
                along,
                right,
                size,
                height,
            });
        }
        let buildings = (0..r.u32())
            .map(|_| {
                let levels = r.f32();
                let outline = (0..r.u32()).map(|_| Vec2::new(r.f32(), r.f32())).collect();
                Building { levels, outline }
            })
            .collect();
        let fences = (0..r.u32())
            .map(|_| {
                let (kind, height) = (FenceKind::from_byte(r.take(1)[0]), r.f32());
                let points = (0..r.u32()).map(|_| Vec2::new(r.f32(), r.f32())).collect();
                Fence {
                    kind,
                    height,
                    points,
                }
            })
            .collect();
        let corners: Vec<Corner> = (0..r.u32())
            .map(|_| {
                let kind = RoadKind::from_byte(r.take(1)[0]);
                let points = (0..r.u32())
                    .map(|_| {
                        let (x, z, y) = (r.f32(), r.f32(), r.f32());
                        Vec3::new(x, y, z)
                    })
                    .collect();
                Corner::new(kind, points)
            })
            .collect();
        let half = Vec2::new(cells_x as f32, cells_z as f32) * cell / 2.0;
        let mut terrain = Self {
            cells_x,
            cells_z,
            cell,
            heights,
            chunk_cells,
            chunks,
            base,
            road_grid: RoadGrid::new(&roads, &corners, half),
            roads,
            corners,
            buildings,
            fences,
            obstacles: Obstacles::default(),
            // FNV-1a.
            fingerprint: file.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
            }),
        };
        terrain.settle_corners();
        Ok(terrain)
    }

    /// A flat, empty square 1 km across at height 0, paved all over, for driving tests.
    #[cfg(test)]
    pub fn flat() -> Self {
        let (cells, chunk_cells) = (256, 32);
        let chunks = (cells / chunk_cells) * (cells / chunk_cells);
        Self {
            cells_x: cells,
            cells_z: cells,
            cell: 4.0,
            heights: vec![0.0; (cells + 1) * (cells + 1)],
            chunk_cells,
            chunks: (0..chunks)
                .map(|_| Chunk {
                    step: chunk_cells,
                    classes: vec![PAVED],
                })
                .collect(),
            base: 0.0,
            roads: Vec::new(),
            corners: Vec::new(),
            road_grid: RoadGrid::new(&[], &[], Vec2::splat(cells as f32 * 2.0)),
            buildings: Vec::new(),
            fences: Vec::new(),
            obstacles: Obstacles::default(),
            fingerprint: 0,
        }
    }

    /// `flat`, with a straight asphalt road 6 m wide along z through the middle, as uneven
    /// as `roughness` says (see `Road::roughness`).
    #[cfg(test)]
    pub fn flat_road(roughness: f32) -> Self {
        let mut terrain = Self::flat();
        let points: Vec<Vec2> = (0..=900)
            .map(|i| Vec2::new(0.0, 450.0 - i as f32))
            .collect();
        let along = (0..=900).map(|i| i as f32).collect();
        terrain.roads.push(Road {
            kind: RoadKind::Asphalt,
            width: 6.0,
            bridge: false,
            ends: [(6.0, 0.0); 2],
            joins: [None; 2],
            meets: Vec::new(),
            crown: 0.025,
            heights: vec![0.1; points.len()],
            points,
            along,
            roughness,
            waves: Waves::new(roughness, 1),
            features: Vec::new(),
        });
        terrain.road_grid = RoadGrid::new(&terrain.roads, &[], terrain.half_size());
        terrain
    }

    /// `flat` covered in meadow (grass grips under half as well as asphalt), with a road 6
    /// m wide along z through the middle level with it: a car straddling an edge has split
    /// grip.
    #[cfg(test)]
    pub fn flat_meadow() -> Self {
        let mut terrain = Self::flat_road(0.0);
        for chunk in &mut terrain.chunks {
            chunk.classes = vec![6];
        }
        for road in &mut terrain.roads {
            road.heights.fill(0.0);
            road.crown = 0.0;
        }
        terrain
    }

    /// Half the map's extent along x and z; the map is centred on the origin.
    pub fn half_size(&self) -> Vec2 {
        Vec2::new(self.cells_x as f32, self.cells_z as f32) * self.cell / 2.0
    }

    fn chunks_x(&self) -> usize {
        self.cells_x / self.chunk_cells
    }

    fn vertex(&self, ix: usize, iz: usize) -> Vec3 {
        Vec3::new(
            ix as f32 * self.cell - self.half_size().x,
            self.heights[iz * (self.cells_x + 1) + ix],
            iz as f32 * self.cell - self.half_size().y,
        )
    }

    /// The triangle of the rendered mesh that contains (x, z), wound counter-clockwise from above.
    fn triangle(&self, x: f32, z: f32) -> [Vec3; 3] {
        let half = self.half_size();
        let gx = ((x + half.x) / self.cell).clamp(0.0, self.cells_x as f32 - 1e-3);
        let gz = ((z + half.y) / self.cell).clamp(0.0, self.cells_z as f32 - 1e-3);
        let (ix, iz) = (gx as usize, gz as usize);
        let (a, b) = (self.vertex(ix, iz), self.vertex(ix + 1, iz));
        let (c, d) = (self.vertex(ix, iz + 1), self.vertex(ix + 1, iz + 1));
        // Each quad is split along its a-d diagonal.
        if gx.fract() > gz.fract() {
            [a, d, b]
        } else {
            [a, c, d]
        }
    }

    /// The terrain's height and upward normal at (x, z), without the roads on it.
    pub fn land(&self, x: f32, z: f32) -> (f32, Vec3) {
        let [a, b, c] = self.triangle(x, z);
        let normal = (b - a).cross(c - a).normalize();
        // Plane through `a`: normal · (p - a) = 0, solved for y.
        let y = a.y - (normal.x * (x - a.x) + normal.z * (z - a.z)) / normal.y;
        (y, normal)
    }

    /// Land use class at (x, z) (classes as in `tools/bake`).
    pub fn land_class(&self, x: f32, z: f32) -> u8 {
        let half = self.half_size();
        let grid = |v: f32, max: usize| ((v / self.cell).max(0.0) as usize).min(max - 1);
        let (ix, iz) = (
            grid(x + half.x, self.cells_x),
            grid(z + half.y, self.cells_z),
        );
        let chunk = &self.chunks[iz / self.chunk_cells * self.chunks_x() + ix / self.chunk_cells];
        let m = self.chunk_cells / chunk.step;
        let local = |i: usize| i % self.chunk_cells / chunk.step;
        chunk.classes[local(iz) * m + local(ix)]
    }

    /// Height and upward normal of the ground at (x, z) beneath any bridge: the roads
    /// where there are any, else the terrain.
    pub fn sample(&self, x: f32, z: f32) -> (f32, Vec3) {
        let ground = self.ground(x, z, f32::NEG_INFINITY);
        (ground.height, ground.normal)
    }

    /// What the ground at (x, z) beneath any bridge is.
    pub fn surface(&self, x: f32, z: f32) -> Surface {
        self.ground(x, z, f32::NEG_INFINITY).surface
    }

    /// Height and normal of the ground under `p` (see `ground`).
    pub fn ground_under(&self, p: Vec3) -> (f32, Vec3) {
        let ground = self.ground(p.x, p.z, p.y);
        (ground.height, ground.normal)
    }

    /// The ground at (x, z) for something at height `y`: the highest surface at most
    /// `LEVEL_REACH` above it (or the lowest, if none is), so a bridge's deck carries what's
    /// on it and lets what's under it pass. The surfaces are the bridges' decks, and the
    /// roads covering the point (blended where they overlap, see `ROAD_FADE`), else the
    /// terrain (and its land use, or a waterway on it).
    pub fn ground(&self, x: f32, z: f32, y: f32) -> Ground {
        self.ground_and_roads(x, z, y).0
    }

    /// `ground`, and the height of the roads on the ground there (blended), if any cover
    /// it.
    /// Whether `p` is on a road on the ground (not a bridge or waterway), at least
    /// `margin` in from its edge.
    fn under_road(&self, p: Vec2, margin: f32) -> bool {
        self.road_grid.near(p).iter().any(|&(r, i)| {
            let (r, i) = (r as usize, i as usize);
            let Some(road) = self.roads.get(r) else {
                return false;
            };
            if road.bridge || road.kind == RoadKind::Water {
                return false;
            }
            let (a, b) = (road.points[i], road.points[i + 1]);
            let length = a.distance(b).max(1e-6);
            let t = ((p - a).dot(b - a) / (length * length)).clamp(0.0, 1.0);
            let off = a.lerp(b, t).distance(p);
            off + margin <= road.width_at(road.along[i] + t * length) / 2.0
        })
    }

    fn ground_and_roads(&self, x: f32, z: f32, y: f32) -> (Ground, Option<f32>) {
        let p = Vec2::new(x, z);
        // Per road covering it or within `JOIN_REACH` of it, the segment it reaches
        // furthest over the point with: (road, how far inside its edge, segment, how far
        // along it).
        let mut reach: [(usize, f32, usize, f32); 12] = [(usize::MAX, 0.0, 0, 0.0); 12];
        let mut count = 0;
        let mut water = false;
        let mut corner = None;
        for &(r, i) in self.road_grid.near(p) {
            let (r, i) = (r as usize, i as usize);
            let Some(road) = self.roads.get(r) else {
                let c = &self.corners[r - self.roads.len()];
                if corner.is_none() {
                    corner = c.at(p).map(|(height, normal)| Ground {
                        height,
                        normal,
                        surface: c.kind.surface(),
                    });
                }
                continue;
            };
            let (a, b) = (road.points[i], road.points[i + 1]);
            let length = a.distance(b).max(1e-6);
            let along = (b - a) / length;
            let t = ((p - a).dot(along) / length).clamp(0.0, 1.0);
            let off = a.lerp(b, t).distance(p);
            let s = road.along[i] + t * length;
            let margin = road.width_at(s) / 2.0 - off;
            if margin <= -JOIN_REACH || (margin < 0.0 && road.meets.is_empty()) {
                continue;
            }
            if road.kind == RoadKind::Water {
                water |= margin >= 0.0;
                continue;
            }
            match reach[..count].iter_mut().find(|c| c.0 == r) {
                Some(entry) if entry.1 >= margin => {}
                Some(entry) => *entry = (r, margin, i, t),
                None if count < 12 => {
                    reach[count] = (r, margin, i, t);
                    count += 1;
                }
                None => {}
            }
        }
        // Each one's height there with its relief, and fade towards its ends: (road, how
        // far inside its edge, fade, height, slope as d height / d (x, z)).
        let mut near: [(usize, f32, f32, f32, Vec2); 12] =
            [(usize::MAX, 0.0, 0.0, 0.0, Vec2::ZERO); 12];
        for (entry, &(r, margin, i, t)) in near.iter_mut().zip(&reach[..count]) {
            let road = &self.roads[r];
            let (a, b) = (road.points[i], road.points[i + 1]);
            let length = a.distance(b).max(1e-6);
            let along = (b - a) / length;
            let off = a.lerp(b, t).distance(p);
            let s = road.along[i] + t * length;
            let to_end = s.min(road.along[road.along.len() - 1] - s);
            let fade = (to_end / ROAD_FADE).clamp(0.02, 1.0);
            let side = along.perp();
            let right = (p - a).dot(side);
            let rise = (road.heights[i + 1] - road.heights[i]) / length;
            let (bump, tilt) = road.relief(s, right, margin.max(0.0));
            let height = road.heights[i] + rise * t * length - road.crown * off + bump;
            let slope = along * (rise + tilt.x) + side * (tilt.y - road.crown * right.signum());
            *entry = (r, margin, fade, height, slope);
        }
        let near = &near[..count];
        // Each road's weight: how far inside its edge; near a road it meets, as if both
        // edges were `JOIN_REACH` further out, each scaled by how near the other's and
        // its own edge are (so at the point where the two edges part, they're even).
        let mut covering: [(usize, f32, f32, Vec2, bool); 12] =
            [(usize::MAX, 0.0, 0.0, Vec2::ZERO, false); 12];
        let mut count = 0;
        for &(r, margin, fade, height, slope) in near {
            let road = &self.roads[r];
            let nearness = |margin: f32| smoothstep((margin + JOIN_REACH) / JOIN_REACH);
            let joined = near
                .iter()
                .filter(|o| road.meets.binary_search(&(o.0 as u32)).is_ok())
                .map(|o| nearness(o.1))
                .fold(0.0, f32::max);
            let own = if margin >= 0.0 { margin.max(1e-3) } else { 0.0 };
            let weight = own.max(joined * nearness(margin) * (margin + JOIN_REACH)) * fade;
            if weight > 0.0 {
                covering[count] = (r, weight, height, slope, margin >= 0.0);
                count += 1;
            }
        }
        // On a road only where one covers the point.
        let on = |bridges: bool| {
            near.iter()
                .any(|c| c.1 >= 0.0 && self.roads[c.0].bridge == bridges)
        };
        let covering = &covering[..count];
        // The roads on the ground, and the bridges, each blended among themselves (or all
        // together: `None`).
        let blend = |bridges: Option<bool>| {
            let layer = || {
                covering
                    .iter()
                    .filter(move |c| bridges.is_none_or(|b| self.roads[c.0].bridge == b))
            };
            let total: f32 = layer().map(|c| c.1).sum();
            // The surface is the one drawn: the best road covering the point.
            let main = layer()
                .filter(|c| c.4)
                .reduce(|a, b| {
                    if outranks(&self.roads, b.0, a.0) {
                        b
                    } else {
                        a
                    }
                })
                .or_else(|| layer().max_by(|a, b| a.1.total_cmp(&b.1)))?;
            let slope = layer().map(|c| c.1 * c.3).sum::<Vec2>() / total;
            Some(Ground {
                height: layer().map(|c| c.1 * c.2).sum::<f32>() / total,
                normal: Vec3::new(-slope.x, 1.0, -slope.y).normalize(),
                surface: self.roads[main.0].kind.surface(),
            })
        };
        let roads = blend(Some(false)).filter(|_| on(false));
        let below = roads.or(corner).unwrap_or_else(|| {
            let (height, normal) = self.land(x, z);
            let surface = if water {
                Surface::Water
            } else {
                land_surface(self.land_class(x, z))
            };
            Ground {
                height,
                normal,
                surface,
            }
        });
        let ground = match blend(Some(true)).filter(|_| on(true)) {
            // Level with the roads on the ground (where they meet a bridge's end): one.
            Some(deck) if (deck.height - below.height).abs() < LEVEL_REACH / 2.0 => {
                blend(None).unwrap()
            }
            Some(deck) if deck.height <= y + LEVEL_REACH && deck.height > below.height => deck,
            _ => below,
        };
        (ground, roads.map(|r| r.height))
    }

    /// Divides the corners' spokes (`CORNER_LATTICE` apart) and sets their heights to
    /// follow the roads: a point on a road is on its surface as the ribbons are drawn
    /// (blended where roads overlap), one off the roads blends the surfaces at the near
    /// roads' edges, weighted by how near each edge is. So a corner meets every road
    /// at its edge's height and runs smoothly between them; the ground beyond its arc
    /// joins it there (`cut`).
    fn settle_corners(&mut self) {
        // Natively spread over the cores (a fifth of a second on one); the web has one.
        #[cfg(not(target_arch = "wasm32"))]
        let settled: Vec<(usize, Vec<Vec<f32>>)> = {
            let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
            let share = self.corners.len().div_ceil(threads).max(1);
            let this = &*self;
            std::thread::scope(|scope| {
                let parts: Vec<_> = this
                    .corners
                    .chunks(share)
                    .map(|part| {
                        scope.spawn(move || {
                            part.iter()
                                .map(|corner| this.corner_spokes(corner))
                                .collect::<Vec<_>>()
                        })
                    })
                    .collect();
                parts
                    .into_iter()
                    .flat_map(|part| part.join().expect("settling corners"))
                    .collect()
            })
        };
        #[cfg(target_arch = "wasm32")]
        let settled: Vec<(usize, Vec<Vec<f32>>)> = self
            .corners
            .iter()
            .map(|corner| self.corner_spokes(corner))
            .collect();
        for (corner, (n, spokes)) in self.corners.iter_mut().zip(settled) {
            corner.divisions = n;
            corner.spokes = spokes;
        }
    }

    fn corner_spokes(&self, corner: &Corner) -> (usize, Vec<Vec<f32>>) {
        let e = corner.points[0];
        let longest = corner.points[1..]
            .iter()
            .map(|p| p.xz().distance(e.xz()))
            .fold(0.0, f32::max);
        let n = ((longest / CORNER_LATTICE).ceil() as usize).clamp(1, 16);
        // The road segments near it: those it lies between, whose edges its arc meets.
        let (lo, hi) = corner
            .points
            .iter()
            .fold((Vec2::MAX, Vec2::MIN), |(lo, hi), p| {
                (lo.min(p.xz()), hi.max(p.xz()))
            });
        let mut near: Vec<(usize, usize)> = Vec::new();
        let (c0, c1) = (self.road_grid.cell(lo - 4.0), self.road_grid.cell(hi + 4.0));
        for cz in c0.1..=c1.1 {
            for cx in c0.0..=c1.0 {
                let k = cz * self.road_grid.cells_x + cx;
                let cell = &self.road_grid.entries
                    [self.road_grid.starts[k] as usize..self.road_grid.starts[k + 1] as usize];
                for &(r, i) in cell {
                    let (r, i) = (r as usize, i as usize);
                    let keep = self
                        .roads
                        .get(r)
                        .is_some_and(|road| road.kind != RoadKind::Water && !road.bridge);
                    if keep {
                        near.push((r, i));
                    }
                }
            }
        }
        near.sort_unstable();
        near.dedup();
        let roads_at = |p: Vec2| self.ground_and_roads(p.x, p.y, f32::NEG_INFINITY).1;
        let height = |p: Vec3| -> f32 {
            let q = p.xz();
            if let Some(h) = roads_at(q) {
                return h;
            }
            // Per road near: how far its edge is, and a point just inside it there.
            let mut edges: Vec<(usize, f32, Vec2)> = Vec::new();
            for &(r, i) in &near {
                let road = &self.roads[r];
                let (a, b) = (road.points[i], road.points[i + 1]);
                let length = a.distance(b).max(1e-6);
                let t = ((q - a).dot(b - a) / (length * length)).clamp(0.0, 1.0);
                let centre = a.lerp(b, t);
                let off = centre.distance(q);
                let half = road.width_at(road.along[i] + t * length) / 2.0;
                let beyond = (off - half).max(0.0);
                let inside = centre + (q - centre).normalize_or_zero() * (half - 0.05).max(0.0);
                match edges.iter_mut().find(|e| e.0 == r) {
                    Some(e) if beyond < e.1 => *e = (r, beyond, inside),
                    Some(_) => {}
                    None => edges.push((r, beyond, inside)),
                }
            }
            // Their surfaces there, each weighted by how near: smooth between the roads,
            // and each one's own at its edge. (Those much farther than the nearest count
            // for next to nothing.)
            let nearest = edges.iter().map(|e| e.1).fold(f32::INFINITY, f32::min);
            let (mut sum, mut weights) = (0.0, 0.0);
            for (_, beyond, inside) in edges.into_iter().filter(|e| e.1 < nearest + 8.0) {
                if let Some(h) = roads_at(inside) {
                    let w = 1.0 / (beyond + 0.05).powi(2);
                    sum += w * h;
                    weights += w;
                }
            }
            if weights > 0.0 { sum / weights } else { p.y }
        };
        // The first point is every spoke's first.
        let middle = height(e);
        let spokes = corner.points[1..]
            .iter()
            .map(|&b| {
                let mut heights = vec![middle];
                heights.extend((1..=n).map(|k| height(e + (b - e) * (k as f32 / n as f32))));
                heights
            })
            .collect();
        (n, spokes)
    }

    /// Whether (x, z) is inside the map, away from the rim that slopes down at its edge.
    pub fn inside(&self, x: f32, z: f32) -> bool {
        let inner = self.half_size() - RIM;
        x.abs() < inner.x && z.abs() < inner.y
    }

    /// Marches along the ray and refines the surface crossing by bisection.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, max_distance: f32) -> Option<RayHit> {
        let above = |t: f32| {
            let p = origin + dir * t;
            p.y - self.sample(p.x, p.z).0
        };
        if above(0.0) < 0.0 {
            // Origin is already underground: report contact at the origin.
            return self.hit(origin, dir, 0.0);
        }
        let step = 0.1;
        let mut t0 = 0.0;
        while t0 < max_distance {
            let t1 = (t0 + step).min(max_distance);
            if above(t1) < 0.0 {
                let (mut lo, mut hi) = (t0, t1);
                for _ in 0..10 {
                    let mid = 0.5 * (lo + hi);
                    if above(mid) < 0.0 { hi = mid } else { lo = mid }
                }
                return self.hit(origin, dir, 0.5 * (lo + hi));
            }
            t0 = t1;
        }
        None
    }

    fn hit(&self, origin: Vec3, dir: Vec3, distance: f32) -> Option<RayHit> {
        let point = origin + dir * distance;
        let (_, normal) = self.sample(point.x, point.z);
        Some(RayHit {
            distance,
            point,
            normal,
        })
    }

    /// `count` spots on the paved road nearest the spawn point, `spacing` metres apart, as
    /// (position in xz, yaw facing along the road). The first is the closest point on the
    /// road to the spawn point, facing in the road direction closest to the spawn heading;
    /// the others line up behind it.
    pub fn road_spawns(&self, spawn: &Spawn, count: usize, spacing: f32) -> Vec<(Vec2, f32)> {
        let (road, i, start) = self
            .closest_on_road(spawn.at, |r| r.kind.paved() && r.width >= 5.0)
            .expect("map has no paved roads");
        // Face along the spawn heading; without one, so there's more road behind.
        let n = road.points.len();
        let forward = match spawn.direction() {
            None => i >= n / 2,
            Some(heading) => (road.points[i + 1] - road.points[i]).dot(heading) >= 0.0,
        };
        // Walk backwards from the start; the cars face the other way.
        let behind: Vec<Vec2> = if forward {
            road.points[..=i].iter().rev().copied().collect()
        } else {
            road.points[i + 1..].to_vec()
        };
        let mut points = vec![start];
        points.extend(behind);
        points.dedup();
        let mut spawns = Vec::new();
        let mut next = 0.0;
        let mut travelled = 0.0;
        for pair in points.windows(2) {
            let length = pair[0].distance(pair[1]);
            while spawns.len() < count && next <= travelled + length {
                let along = pair[0].lerp(pair[1], (next - travelled) / length);
                let facing = (pair[0] - pair[1]) / length;
                spawns.push((along, (-facing.x).atan2(-facing.y)));
                next += spacing;
            }
            travelled += length;
        }
        spawns
    }

    /// A spot on the drivable road closest to `p` (inside the map), as (position in xz,
    /// yaw facing along the road in the direction closest to `heading`): in the right-hand
    /// lane where the traffic has a lane each way, else in the middle.
    pub fn drive_spot(&self, p: Vec2, heading: Vec2) -> Option<(Vec2, f32)> {
        self.drive_spot_along(p, heading, 0.0)
    }

    /// Like `drive_spot`, but `shift` metres along that road from the point nearest to `p`
    /// (towards its last point if positive, its first if negative; the other way if it
    /// ends before, else to its end), so a car stuck on a road isn't put back where it was.
    pub fn drive_spot_along(&self, p: Vec2, heading: Vec2, shift: f32) -> Option<(Vec2, f32)> {
        let (road, i, point) = self.closest_on_road(p, |r| r.kind != RoadKind::Water)?;
        let (i, point) = match walk_along(&road.points, i, point, shift) {
            // The road ends before: the other way instead.
            Err(_) => walk_along(&road.points, i, point, -shift).unwrap_or_else(|end| end),
            Ok(moved) => moved,
        };
        if !self.inside(point.x, point.y) {
            return if shift == 0.0 {
                None
            } else {
                self.drive_spot_along(p, heading, 0.0)
            };
        }
        let along = (road.points[i + 1] - road.points[i]).normalize_or(Vec2::NEG_Y);
        let facing = if along.dot(heading) >= 0.0 {
            along
        } else {
            -along
        };
        let right = Vec2::new(-facing.y, facing.x);
        let lane = crate::traffic::network::lane_offset(road.width).unwrap_or(0.0);
        Some((point + lane * right, (-facing.x).atan2(-facing.y)))
    }

    /// The point on a paved road nearest to `p`, the road's direction there and its width.
    pub fn paved_road_near(&self, p: Vec2) -> Option<(Vec2, Vec2, f32)> {
        let (road, i, point) = self.closest_on_road(p, |r| r.kind.paved())?;
        let along = (road.points[i + 1] - road.points[i]).normalize_or(Vec2::NEG_Y);
        Some((point, along, road.width))
    }

    /// The point closest to `p` on the roads that pass `keep`, away from the rim: (road,
    /// segment start index, point).
    fn closest_on_road(
        &self,
        p: Vec2,
        keep: impl Fn(&Road) -> bool,
    ) -> Option<(&Road, usize, Vec2)> {
        self.roads
            .iter()
            .filter(|r| keep(r))
            .flat_map(|r| {
                r.points.windows(2).enumerate().map(move |(i, pair)| {
                    let (a, b) = (pair[0], pair[1]);
                    let t =
                        ((p - a).dot(b - a) / (b - a).length_squared().max(1e-6)).clamp(0.0, 1.0);
                    (r, i, a.lerp(b, t))
                })
            })
            .filter(|(_, _, point)| self.inside(point.x, point.y))
            .min_by(|a, b| a.2.distance(p).total_cmp(&b.2.distance(p)))
    }

    /// Number of chunks; chunks are numbered row-major in z.
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// The chunk containing (x, z), clamped to the map.
    pub fn chunk_at(&self, p: Vec2) -> usize {
        let size = self.chunk_cells as f32 * self.cell;
        let (chunks_x, chunks_z) = (self.chunks_x(), self.chunks.len() / self.chunks_x());
        let index = |v: f32, count: usize| ((v / size).max(0.0) as usize).min(count - 1);
        let half = self.half_size();
        index(p.y + half.y, chunks_z) * chunks_x + index(p.x + half.x, chunks_x)
    }

    /// Lower and upper corners of a chunk in the xz plane.
    pub fn chunk_bounds(&self, index: usize) -> (Vec2, Vec2) {
        let chunks_x = self.chunks_x();
        let size = self.chunk_cells as f32 * self.cell;
        let lo = Vec2::new((index % chunks_x) as f32, (index / chunks_x) as f32) * size
            - self.half_size();
        (lo, lo + size)
    }

    /// Flat-shaded ground of one chunk, at the chunk's resolution: every triangle gets its
    /// own vertices, coloured by land use and slope.
    pub fn ground_mesh(&self, index: usize, mesh: &mut FlatMesh) {
        let per_side = self.chunks_x();
        let chunk = &self.chunks[index];
        let s = chunk.step;
        let m = self.chunk_cells / s;
        let (cx, cz) = (index % per_side, index / per_side);
        for jz in 0..m {
            for jx in 0..m {
                let (ix, iz) = (
                    cx * self.chunk_cells + jx * s,
                    cz * self.chunk_cells + jz * s,
                );
                let (a, b) = (self.vertex(ix, iz), self.vertex(ix + s, iz));
                let (c, d) = (self.vertex(ix, iz + s), self.vertex(ix + s, iz + s));
                for tri in [[a, d, b], [a, c, d]] {
                    let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize();
                    let class = chunk.classes[jz * m + jx];
                    mesh.triangle(tri, ground_color(class, normal.y, ix * 31 + iz * 17));
                }
            }
        }
    }
}

/// The road segments (road, segment start) whose ribbons reach into each cell of a grid
/// over the map, and the corners ((road count + corner, 0)), in compressed rows.
struct RoadGrid {
    half: Vec2,
    cells_x: usize,
    cells_z: usize,
    /// Where each cell's entries start in `entries` (one more than there are cells).
    starts: Vec<u32>,
    entries: Vec<(u32, u32)>,
}

impl RoadGrid {
    fn new(roads: &[Road], corners: &[Corner], half: Vec2) -> Self {
        let (cells_x, cells_z) = (
            (2.0 * half.x / ROAD_CELL).ceil() as usize,
            (2.0 * half.y / ROAD_CELL).ceil() as usize,
        );
        let mut grid = Self {
            half,
            cells_x,
            cells_z,
            starts: Vec::new(),
            entries: Vec::new(),
        };
        let mut cells: Vec<Vec<(u32, u32)>> = vec![Vec::new(); cells_x * cells_z];
        for (r, road) in roads.iter().enumerate() {
            for (i, pair) in road.points.windows(2).enumerate() {
                let reach = Vec2::splat(road.widest() / 2.0 + JOIN_REACH);
                let (lo, hi) = (pair[0].min(pair[1]) - reach, pair[0].max(pair[1]) + reach);
                let ((x0, z0), (x1, z1)) = (grid.cell(lo), grid.cell(hi));
                for cz in z0..=z1 {
                    for cx in x0..=x1 {
                        cells[cz * cells_x + cx].push((r as u32, i as u32));
                    }
                }
            }
        }
        for (c, corner) in corners.iter().enumerate() {
            let (lo, hi) = corner
                .points
                .iter()
                .fold((Vec2::MAX, Vec2::MIN), |(lo, hi), p| {
                    (lo.min(p.xz()), hi.max(p.xz()))
                });
            let ((x0, z0), (x1, z1)) = (grid.cell(lo), grid.cell(hi));
            for cz in z0..=z1 {
                for cx in x0..=x1 {
                    cells[cz * cells_x + cx].push(((roads.len() + c) as u32, 0));
                }
            }
        }
        grid.starts.push(0);
        for cell in cells {
            grid.entries.extend(cell);
            grid.starts.push(grid.entries.len() as u32);
        }
        grid
    }

    fn cell(&self, p: Vec2) -> (usize, usize) {
        let index = |v: f32, count: usize| ((v / ROAD_CELL).max(0.0) as usize).min(count - 1);
        (
            index(p.x + self.half.x, self.cells_x),
            index(p.y + self.half.y, self.cells_z),
        )
    }

    fn near(&self, p: Vec2) -> &[(u32, u32)] {
        let (cx, cz) = self.cell(p);
        let i = cz * self.cells_x + cx;
        &self.entries[self.starts[i] as usize..self.starts[i + 1] as usize]
    }
}

/// Ribbons along the roads, set on the ground as the physics sees it and meshed per chunk,
/// and the bridges' structure under and beside their decks.
pub struct RoadRibbons {
    /// Per chunk, the (road, point) pairs whose quads start there.
    by_chunk: Vec<Vec<(usize, usize)>>,
    /// Per chunk, the corners whose first point is in it.
    corners: Vec<Vec<usize>>,
}

impl RoadRibbons {
    pub fn new(terrain: &Terrain) -> Self {
        let mut by_chunk = vec![Vec::new(); terrain.chunk_count()];
        for (r, road) in terrain.roads.iter().enumerate() {
            for (i, p) in road.points[..road.points.len() - 1].iter().enumerate() {
                by_chunk[terrain.chunk_at(*p)].push((r, i));
            }
        }
        let mut corners = vec![Vec::new(); terrain.chunk_count()];
        for (c, corner) in terrain.corners.iter().enumerate() {
            corners[terrain.chunk_at(corner.points[0].xz())].push(c);
        }
        Self { by_chunk, corners }
    }

    /// The ribbon quads starting in one chunk, into `mesh`, and the bridges' slabs and
    /// parapets into `structure`. Ribbon vertices are at most `DRAPE_STEP` apart across
    /// the road (with one on the crown), each on the ground at the road's own level (its
    /// surface blended with overlapping roads; a bridge's deck, or what passes under it);
    /// waterways lie on the terrain.
    pub fn mesh(
        &self,
        terrain: &Terrain,
        index: usize,
        mesh: &mut FlatMesh,
        structure: &mut FlatMesh,
    ) {
        for &(r, i) in &self.by_chunk[index] {
            let strip = Strip::new(terrain, r, i);
            let (road, fine, across) = (strip.road, strip.fine, strip.across);
            let rows: Vec<Vec<Vec3>> = (0..=fine).map(|k| strip.row(k, 0..=across)).collect();
            let color = road.kind.color();
            // Where a better road covers it, only what's left (no two ribbons on top of
            // each other, but for a sliver along the better one's edge).
            let covered = strip.covered(r);
            for pair in rows.windows(2) {
                let (near, far) = (&pair[0], &pair[1]);
                for j in 0..across {
                    let (a, b) = (near[j], near[j + 1]);
                    let (c, d) = (far[j], far[j + 1]);
                    for tri in [[a, b, c], [b, d, c]] {
                        uncovered(tri, &covered, |tri| mesh.triangle(tri, color));
                    }
                }
            }
            let (near, far) = (&rows[0], &rows[fine]);
            if road.bridge && road.kind != RoadKind::Water {
                let edges = |row: &[Vec3], i: usize| {
                    [(row[0], -strip.side(i)), (row[across], strip.side(i))]
                };
                bridge_segment(
                    edges(near, i),
                    edges(far, i + 1),
                    (i == 0, i + 2 == road.points.len()),
                    structure,
                );
            }
        }
        // (The corners are part of the ground: `cut`.)
    }
}

/// One segment of a road's ribbon as drawn: from point `i` to the next, in `fine` + 1 rows
/// across the road of `across` + 1 vertices each.
struct Strip<'a> {
    terrain: &'a Terrain,
    road: &'a Road,
    i: usize,
    /// Rows along: finer where there's more to see (twice on rough roads, four times
    /// around bumps and potholes).
    fine: usize,
    across: usize,
}

impl<'a> Strip<'a> {
    fn new(terrain: &'a Terrain, r: usize, i: usize) -> Self {
        let road = &terrain.roads[r];
        let fine = if road.feature_near(road.along[i], 1.0) {
            4
        } else if road.roughness >= 64.0 {
            2
        } else {
            1
        };
        let step = DRAPE_STEP / fine as f32;
        let width = road
            .width_at(road.along[i])
            .max(road.width_at(road.along[i + 1]));
        let across = 2 * (width / 2.0 / step).ceil().max(1.0) as usize;
        Self {
            terrain,
            road,
            i,
            fine,
            across,
        }
    }

    fn width(&self, i: usize) -> f32 {
        self.road.width_at(self.road.along[i])
    }

    /// Half the road across at point `i`: where it meets another road's end at an angle,
    /// along the mitre between them, stretched to keep the width.
    fn side(&self, i: usize) -> Vec2 {
        let (points, n) = (&self.road.points, self.road.points.len());
        let tangent =
            (points[(i + 1).min(n - 1)] - points[i.saturating_sub(1)]).normalize_or_zero();
        let join = match i {
            0 => self.road.joins[0],
            i if i == n - 1 => self.road.joins[1],
            _ => None,
        };
        let (across, stretch) = match join {
            Some(mitre) => (mitre, 1.0 / mitre.dot(tangent).max(0.7)),
            None => (tangent, 1.0),
        };
        Vec2::new(-across.y, across.x) * stretch * self.width(i) / 2.0
    }

    /// Row `k`'s centre, half the road across from there to the right, and the centreline's
    /// height.
    fn frame(&self, k: usize) -> (Vec2, Vec2, f32) {
        let (road, i) = (self.road, self.i);
        // The last row is exactly the next segment's first (lerping to 1 can round off).
        let f = k as f32 / self.fine as f32;
        let mix = |a: f32, b: f32| if k == self.fine { b } else { a + (b - a) * f };
        let mix2 = |a: Vec2, b: Vec2| if k == self.fine { b } else { a.lerp(b, f) };
        let points = &road.points;
        let centre = mix2(points[i], points[i + 1]);
        let here = road.width_at(mix(road.along[i], road.along[i + 1]));
        let (near, far) = (self.side(i), self.side(i + 1));
        let stretch = mix(
            near.length() / self.width(i),
            far.length() / self.width(i + 1),
        );
        let side = mix2(near, far).normalize_or_zero() * stretch * here;
        (centre, side, mix(road.heights[i], road.heights[i + 1]))
    }

    /// The ribbon in the xz plane, as convex pieces (counter-clockwise) between its rows,
    /// moved `inset` in from its sides and from the road's ends.
    fn pieces(&self, inset: f32) -> impl Iterator<Item = Vec<Vec2>> + '_ {
        let edges = move |k: usize| {
            let (centre, side, _) = self.frame(k);
            let side = side * (1.0 - inset / side.length().max(1e-3));
            [centre - side, centre + side]
        };
        let n = self.road.points.len();
        (0..self.fine).flat_map(move |k| {
            let ([mut l0, mut r0], [mut l1, mut r1]) = (edges(k), edges(k + 1));
            let first = self.i == 0 && k == 0 && self.road.joins[0].is_none();
            let last = self.i + 2 == n && k + 1 == self.fine && self.road.joins[1].is_none();
            let ahead = ((l1 + r1) - (l0 + r0)).normalize_or_zero() * inset;
            if first {
                (l0, r0) = (l0 + ahead, r0 + ahead);
            }
            if last {
                (l1, r1) = (l1 - ahead, r1 - ahead);
            }
            convex([l0, r0, r1, l1])
        })
    }

    /// What of this segment better roads cover (see `outranks`), as convex pieces
    /// (counter-clockwise, `SEAM_OVERLAP` in from their edges). Bridges and waterways
    /// cover nothing and are never covered.
    fn covered(&self, r: usize) -> Vec<Vec<Vec2>> {
        let (terrain, road, i) = (self.terrain, self.road, self.i);
        let level = |road: &Road| !road.bridge && road.kind != RoadKind::Water;
        if !level(road) {
            return Vec::new();
        }
        // Mitred ends are stretched up to 1/0.7.
        let reach = road.widest() / 2.0 / 0.7 + 0.1;
        let (p, q) = (road.points[i], road.points[i + 1]);
        let (lo, hi) = (p.min(q) - reach, p.max(q) + reach);
        let grid = &terrain.road_grid;
        let ((x0, z0), (x1, z1)) = (grid.cell(lo), grid.cell(hi));
        let mut over: Vec<(usize, usize)> = Vec::new();
        for cz in z0..=z1 {
            for cx in x0..=x1 {
                let k = cz * grid.cells_x + cx;
                for &(b, j) in &grid.entries[grid.starts[k] as usize..grid.starts[k + 1] as usize] {
                    let b = b as usize;
                    if terrain.roads.get(b).is_some_and(|other| {
                        b != r && level(other) && outranks(&terrain.roads, b, r)
                    }) {
                        over.push((b, j as usize));
                    }
                }
            }
        }
        over.sort_unstable();
        over.dedup();
        over.into_iter()
            .flat_map(|(b, j)| {
                Strip::new(terrain, b, j)
                    .pieces(SEAM_OVERLAP)
                    .filter(|piece| {
                        let (a, z) = bounds(piece);
                        a.x <= hi.x && a.y <= hi.y && z.x >= lo.x && z.y >= lo.y
                    })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Row `k`'s vertices `js` (0 at the left edge, `across` at the right one).
    fn row(&self, k: usize, js: impl Iterator<Item = usize>) -> Vec<Vec3> {
        let (road, terrain) = (self.road, self.terrain);
        let (centre, side, level) = self.frame(k);
        js.map(|j| {
            let u = 2.0 * j as f32 / self.across as f32 - 1.0;
            let p = centre + side * u;
            // Looked up a little inside the edge, which may lie just outside the
            // segments' cover on a bend (and would drop off a bridge).
            let q = centre + side * u * 0.98;
            let ground = if road.kind == RoadKind::Water {
                terrain.land(p.x, p.y).0
            } else {
                terrain.ground(q.x, q.y, level).height
            };
            Vec3::new(p.x, ground + road.kind.lift(), p.y)
        })
        .collect()
    }
}

/// 0 below 0, 1 above 1, smoothly between.
fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The lower and upper corners of a polygon's bounding box.
fn bounds(poly: &[Vec2]) -> (Vec2, Vec2) {
    poly.iter().fold((Vec2::MAX, Vec2::MIN), |(lo, hi), p| {
        (lo.min(*p), hi.max(*p))
    })
}

/// Twice a polygon's signed area (positive counter-clockwise).
fn area2(poly: &[Vec2]) -> f32 {
    (0..poly.len())
        .map(|k| poly[k].perp_dot(poly[(k + 1) % poly.len()]))
        .sum()
}

/// A quad as convex polygons, counter-clockwise: itself, or (bent past convex, on a
/// tight bend) its two triangles.
fn convex(quad: [Vec2; 4]) -> Vec<Vec<Vec2>> {
    let turns = (0..4).map(|k| {
        let (a, b, c) = (quad[k], quad[(k + 1) % 4], quad[(k + 2) % 4]);
        (b - a).perp_dot(c - b)
    });
    let (left, right) = turns.fold((0, 0), |(l, r), t| {
        (l + (t > 0.0) as usize, r + (t < 0.0) as usize)
    });
    let pieces = if left == 0 || right == 0 {
        vec![quad.to_vec()]
    } else {
        vec![
            vec![quad[0], quad[1], quad[2]],
            vec![quad[0], quad[2], quad[3]],
        ]
    };
    pieces
        .into_iter()
        .filter_map(|mut p| {
            let a = area2(&p);
            if a < 0.0 {
                p.reverse();
            }
            (a.abs() > 1e-6).then_some(p)
        })
        .collect()
}

/// The parts of convex polygon `p` outside convex polygon `q` (both counter-clockwise),
/// as convex polygons, into `out`.
fn subtract(p: Vec<Vec2>, q: &[Vec2], out: &mut Vec<Vec<Vec2>>) {
    let mut rest = p;
    for k in 0..q.len() {
        let (a, b) = (q[k], q[(k + 1) % q.len()]);
        // Inside q is left of a → b.
        let side = |v: Vec2| (b - a).perp_dot(v - a);
        let (mut inside, mut outside) = (Vec::new(), Vec::new());
        for m in 0..rest.len() {
            let (u, v) = (rest[m], rest[(m + 1) % rest.len()]);
            let (su, sv) = (side(u), side(v));
            if su >= 0.0 {
                inside.push(u);
            }
            if su <= 0.0 {
                outside.push(u);
            }
            if (su > 0.0 && sv < 0.0) || (su < 0.0 && sv > 0.0) {
                let x = u.lerp(v, su / (su - sv));
                inside.push(x);
                outside.push(x);
            }
        }
        if outside.len() >= 3 && area2(&outside) > 1e-6 {
            out.push(outside);
        }
        if inside.len() < 3 || area2(&inside) <= 1e-6 {
            return;
        }
        rest = inside;
    }
}

/// What of triangle `tri` lies outside the convex polygons `covered`, as triangles in its
/// plane, each to `emit` (the whole triangle if nothing covers it).
fn uncovered(tri: [Vec3; 3], covered: &[Vec<Vec2>], mut emit: impl FnMut([Vec3; 3])) {
    let flat = tri.map(|v| v.xz());
    let (lo, hi) = bounds(&flat);
    let mut over = covered.iter().filter(|q| {
        let (a, b) = bounds(q);
        a.x < hi.x && a.y < hi.y && b.x > lo.x && b.y > lo.y
    });
    let Some(first) = over.next() else {
        return emit(tri);
    };
    let det = (flat[1] - flat[0]).perp_dot(flat[2] - flat[0]);
    if det.abs() < 1e-9 {
        return emit(tri);
    }
    // Clipped near the triangle's first corner, where f32 has room for the decimals.
    let origin = flat[0];
    let local = |v: &Vec2| *v - origin;
    let start: Vec<Vec2> = flat.iter().map(local).collect();
    let start = if det < 0.0 {
        start.into_iter().rev().collect()
    } else {
        start
    };
    let mut pieces = vec![start];
    for q in std::iter::once(first).chain(over) {
        let q: Vec<Vec2> = q.iter().map(local).collect();
        let mut left = Vec::new();
        for piece in pieces {
            subtract(piece, &q, &mut left);
        }
        pieces = left;
        if pieces.is_empty() {
            return;
        }
    }
    // Heights on the triangle's plane.
    let (e1, e2) = (flat[1] - flat[0], flat[2] - flat[0]);
    let lift = |p: Vec2| {
        let (u, v) = (p.perp_dot(e2) / det, e1.perp_dot(p) / det);
        let y = tri[0].y + (tri[1].y - tri[0].y) * u + (tri[2].y - tri[0].y) * v;
        Vec3::new(p.x + origin.x, y, p.y + origin.y)
    };
    for piece in pieces {
        for k in 1..piece.len() - 1 {
            emit([lift(piece[0]), lift(piece[k]), lift(piece[k + 1])]);
        }
    }
}

/// Sets each road's `meets`:the roads on the ground (not bridges or waterways) it shares
/// a point with (a junction).
fn find_meetings(roads: &mut [Road]) {
    let on_ground = |road: &Road| !road.bridge && road.kind != RoadKind::Water;
    let mut at: std::collections::HashMap<IVec2, Vec<u32>> = Default::default();
    for (r, road) in roads.iter().enumerate().filter(|(_, road)| on_ground(road)) {
        for &p in &road.points {
            at.entry((p * 100.0).round().as_ivec2())
                .or_default()
                .push(r as u32);
        }
    }
    let mut meets = vec![Vec::new(); roads.len()];
    for there in at.values().filter(|there| there.len() > 1) {
        for &a in there {
            meets[a as usize].extend(there.iter().filter(|&&b| b != a));
        }
    }
    for (road, mut meets) in roads.iter_mut().zip(meets) {
        meets.sort_unstable();
        meets.dedup();
        road.meets = meets;
    }
}

/// Where roads on the same level (not waterways) end at one point, sets the `joins` there
/// of the two running on into each other most straight (bending less than a right angle;
/// where more end there, as wide as each other there) to the direction halfway between
/// theirs. (Any others ending there end square, under the better roads:
/// `Strip::covered`.)
fn join_ends(roads: &mut [Road]) {
    // A road's end: (road, 0 for its first point or 1 for its last).
    type End = (usize, usize);
    let mut at: std::collections::HashMap<IVec2, Vec<End>> = Default::default();
    for (r, road) in roads.iter().enumerate() {
        if road.kind == RoadKind::Water {
            continue;
        }
        let n = road.points.len();
        for (end, i) in [(0, 0), (1, n - 1)] {
            let key = (road.points[i] * 100.0).round().as_ivec2();
            at.entry(key).or_default().push((r, end));
        }
    }
    // A road's direction at an end, pointing along it from first to last.
    let direction = |road: &Road, end: usize| {
        let p = &road.points;
        let n = p.len();
        match end {
            0 => (p[1] - p[0]).normalize_or_zero(),
            _ => (p[n - 1] - p[n - 2]).normalize_or_zero(),
        }
    };
    // Sorted, so the same map always joins the same way.
    let mut meetings: Vec<Vec<End>> = at.into_values().filter(|m| m.len() >= 2).collect();
    meetings.sort_unstable();
    for meeting in meetings {
        // Per pair: both directions as running from the first into the second.
        let mut best: Option<(End, End, Vec2, Vec2)> = None;
        for (k, &(a, ea)) in meeting.iter().enumerate() {
            for &(b, eb) in &meeting[k + 1..] {
                let (wa, wb) = (roads[a].ends[ea].0, roads[b].ends[eb].0);
                let alike = meeting.len() == 2 || (wa - wb).abs() < 0.01;
                if a == b || roads[a].bridge != roads[b].bridge || !alike {
                    continue;
                }
                let into = |road: usize, end: usize| {
                    let d = direction(&roads[road], end);
                    if (road == a) == (end == 1) { d } else { -d }
                };
                let (da, db) = (into(a, ea), into(b, eb));
                if best.is_none_or(|(_, _, ba, bb)| da.dot(db) > ba.dot(bb)) {
                    best = Some(((a, ea), (b, eb), da, db));
                }
            }
        }
        let Some(((a, ea), (b, eb), da, db)) = best else {
            continue;
        };
        let mean = (da + db).normalize_or_zero();
        // Only bends gentler than a right angle.
        if mean == Vec2::ZERO || da.dot(db) < 0.0 {
            continue;
        }
        roads[a].joins[ea] = Some(if ea == 1 { mean } else { -mean });
        roads[b].joins[eb] = Some(if eb == 0 { mean } else { -mean });
    }
}

/// How far a bridge's slab reaches below its deck's edges (m).
pub const DECK_DEPTH: f32 = 0.9;
/// A bridge's parapets: their height above the deck and their thickness (m).
pub const PARAPET_HEIGHT: f32 = 0.9;
pub const PARAPET_WIDTH: f32 = 0.3;
const CONCRETE: [f32; 4] = [0.55, 0.54, 0.51, 1.0];

/// One piece of a bridge between two rows across it: for each row the deck's left and
/// right edge with a horizontal vector pointing out of that edge, and whether the piece
/// starts or ends the bridge (which then gets an end face).
fn bridge_segment(
    near: [(Vec3, Vec2); 2],
    far: [(Vec3, Vec2); 2],
    (start, end): (bool, bool),
    mesh: &mut FlatMesh,
) {
    let up = Vec3::Y;
    // Per side and row: deck edge, parapet's outer top, outer bottom, inner top.
    let corners = |(edge, out): (Vec3, Vec2)| {
        let out = out.normalize_or_zero().extend(0.0).xzy() * PARAPET_WIDTH;
        let outer = edge + out;
        [
            edge,
            outer + up * PARAPET_HEIGHT,
            outer - up * DECK_DEPTH,
            edge + up * PARAPET_HEIGHT,
        ]
    };
    let mut quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3, outward: Vec3| {
        mesh.facing([a, b, c], CONCRETE, outward);
        mesh.facing([b, d, c], CONCRETE, outward);
    };
    let sides = [0, 1].map(|k| (corners(near[k]), corners(far[k]), near[k].1));
    for &(n, f, out) in &sides {
        let out = out.extend(0.0).xzy();
        // Outer face, top, inner face.
        quad(n[2], n[1], f[2], f[1], out);
        quad(n[3], n[1], f[3], f[1], up);
        quad(n[0], n[3], f[0], f[3], -out);
    }
    // Underside, between the outer bottoms.
    let ((ln, lf, _), (rn, rf, _)) = (sides[0], sides[1]);
    quad(ln[2], rn[2], lf[2], rf[2], -up);
    // End faces: the slab under the deck.
    for (cap, (l, r), toward) in [
        (start, (ln, rn), lf[0] - ln[0]),
        (end, (lf, rf), ln[0] - lf[0]),
    ] {
        if cap {
            quad(l[2], r[2], l[0], r[0], -toward);
        }
    }
}

/// Height at full-detail vertex (`lx`, `lz`) of a chunk stored at `step`, from its
/// `(m + 1)²` coarse vertices: on the coarse triangles, split along the same diagonal as
/// `Terrain::triangle`, so every full-detail triangle lies flat within a coarse one.
fn on_coarse_grid(coarse: &[f32], m: usize, step: usize, lx: usize, lz: usize) -> f32 {
    let (ix, iz) = ((lx / step).min(m - 1), (lz / step).min(m - 1));
    let u = (lx - ix * step) as f32 / step as f32;
    let v = (lz - iz * step) as f32 / step as f32;
    let h = |i: usize, j: usize| coarse[(iz + j) * (m + 1) + ix + i];
    let (a, b, c, d) = (h(0, 0), h(1, 0), h(0, 1), h(1, 1));
    if u > v {
        a + (b - a) * (u - v) + (d - a) * v
    } else {
        a + (c - a) * (v - u) + (d - a) * u
    }
}

/// A chunk's `side`² heights (m), stored as centimetres predicted from their neighbours
/// before them (see `tools/bake`): low bytes, then high bytes.
fn predicted_heights(r: &mut Bytes, side: usize) -> Vec<f32> {
    let count = side * side;
    let (low, high) = (r.take(count), r.take(count));
    let mut cm = vec![0u16; count];
    for j in 0..side {
        for i in 0..side {
            let at = |i: usize, j: usize| cm[j * side + i];
            let guess = match (i, j) {
                (0, 0) => 0,
                (0, _) => at(0, j - 1),
                (_, 0) => at(i - 1, 0),
                _ => at(i - 1, j)
                    .wrapping_add(at(i, j - 1))
                    .wrapping_sub(at(i - 1, j - 1)),
            };
            let k = j * side + i;
            cm[k] = guess.wrapping_add(u16::from_le_bytes([low[k], high[k]]));
        }
    }
    cm.into_iter().map(|h| h as f32 / 100.0).collect()
}

/// Little-endian reader over the baked file.
struct Bytes<'a>(&'a [u8]);

impl<'a> Bytes<'a> {
    fn take(&mut self, n: usize) -> &'a [u8] {
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        head
    }

    fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take(2).try_into().unwrap())
    }

    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }

    fn f32(&mut self) -> f32 {
        f32::from_le_bytes(self.take(4).try_into().unwrap())
    }
}

/// Accumulates flat-shaded, vertex-coloured triangles.
#[derive(Default)]
pub struct FlatMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
}

impl FlatMesh {
    /// A triangle wound to face upward.
    pub fn triangle(&mut self, tri: [Vec3; 3], color: [f32; 4]) {
        self.facing(tri, color, Vec3::Y);
    }

    /// A triangle wound so that its normal points to the same side as `outward`.
    pub fn facing(&mut self, [a, mut b, mut c]: [Vec3; 3], color: [f32; 4], outward: Vec3) {
        let mut normal = (b - a).cross(c - a).normalize_or_zero();
        if normal.dot(outward) < 0.0 {
            (b, c, normal) = (c, b, -normal);
        }
        for v in [a, b, c] {
            self.positions.push(v.to_array());
            self.normals.push(normal.to_array());
            self.colors.push(color);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }

    /// Adds another mesh's triangles.
    pub fn append(&mut self, other: FlatMesh) {
        self.positions.extend(other.positions);
        self.normals.extend(other.normals);
        self.colors.extend(other.colors);
    }

    pub fn build(self) -> Mesh {
        let indices = (0..self.positions.len() as u32).collect();
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_indices(Indices::U32(indices))
    }
}

/// Land use class of paved areas (see `tools/bake`).
const PAVED: u8 = 10;

/// What the ground of a land use class (see `tools/bake`) is.
fn land_surface(class: u8) -> Surface {
    match class {
        2..=5 => Surface::Field,
        7 | 8 => Surface::Forest,
        9 => Surface::Water,
        PAVED => Surface::Asphalt,
        _ => Surface::Grass,
    }
}

/// Ground colour for a land use class (see `tools/bake`), with steep natural slopes
/// showing bare earth.
pub fn ground_color(class: u8, up: f32, variation: usize) -> [f32; 4] {
    let jitter = (hash(variation as u32) - 0.5) * 0.04;
    let base = match class {
        1 => Vec3::new(0.42, 0.56, 0.33),     // residential: gardens
        2 => Vec3::new(0.78, 0.68, 0.42),     // field: ripe grain
        3 => Vec3::new(0.70, 0.62, 0.42),     // field: stubble
        4 => Vec3::new(0.52, 0.41, 0.30),     // field: ploughed
        5 => Vec3::new(0.46, 0.60, 0.26),     // field: green crop
        6 => Vec3::new(0.36, 0.58, 0.28),     // meadow
        7 => Vec3::new(0.32, 0.44, 0.24),     // scrub
        8 => Vec3::new(0.17, 0.32, 0.15),     // forest
        9 => Vec3::new(0.22, 0.42, 0.58),     // water
        PAVED => Vec3::new(0.30, 0.30, 0.32), // paved area
        _ => Vec3::new(0.34, 0.53, 0.27),     // unmapped: grass
    };
    let natural = !matches!(class, 8..=10);
    let base = if natural && up < 0.85 {
        Vec3::new(0.47, 0.40, 0.33)
    } else {
        base
    };
    (base + Vec3::splat(jitter)).extend(1.0).to_array()
}

/// Integer hash to [0, 1).
pub fn hash(n: u32) -> f32 {
    let mut h = n.wrapping_mul(0x27d4_eb2d) ^ 0x1656_67b1;
    h ^= h >> 15;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// From `point` on the segment starting at `points[i]`, `shift` metres along the line
/// (towards its last point if positive): the segment and point there, or `Err` with its
/// end if it ends before.
fn walk_along(
    points: &[Vec2],
    mut i: usize,
    mut point: Vec2,
    shift: f32,
) -> Result<(usize, Vec2), (usize, Vec2)> {
    let mut left = shift.abs();
    while left > 0.0 {
        let to = if shift > 0.0 {
            points[i + 1]
        } else {
            points[i]
        };
        let gap = point.distance(to);
        if gap >= left {
            return Ok((i, point.move_towards(to, left)));
        }
        left -= gap;
        point = to;
        if shift > 0.0 && i + 2 < points.len() {
            i += 1;
        } else if shift < 0.0 && i > 0 {
            i -= 1;
        } else {
            return Err((i, point));
        }
    }
    Ok((i, point))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raycast_matches_sampled_height() {
        let terrain = Terrain::baked();
        for &(x, z) in &[(0.0, 0.0), (90.3, -41.7), (-150.2, 133.9)] {
            let (h, _) = terrain.sample(x, z);
            let hit = terrain
                .raycast(Vec3::new(x, h + 1.0, z), Vec3::NEG_Y, 2.0)
                .unwrap();
            assert!((hit.distance - 1.0).abs() < 1e-3);
        }
    }

    #[test]
    fn spawns_are_read_from_text() {
        assert_eq!(
            Spawn::parse("-2984.5 -2007.2 180"),
            Ok(Spawn {
                at: Vec2::new(-2984.5, -2007.2),
                heading: Some(180.0)
            })
        );
        assert_eq!(Spawn::parse("10,-20").map(|s| s.heading), Ok(None));
        assert!(Spawn::parse("10").is_err());
        assert!(Spawn::parse("10 x").is_err());
        assert!(Spawn::parse("1 2 3 4").is_err());
        assert!(Spawn::parse("NaN 2").is_err());
    }

    #[test]
    fn spawns_lie_on_a_road() {
        let terrain = Terrain::baked();
        let spawn = Spawn::DEFAULT;
        let spawns = terrain.road_spawns(&spawn, 3, 9.0);
        assert_eq!(spawns.len(), 3);
        assert!(spawns[0].0.distance(spawns[2].0) > 10.0);
        // At the spawn point (Gerach), facing along the spawn heading.
        assert!(spawns[0].0.distance(spawn.at) < 30.0);
        for (p, yaw) in &spawns {
            let facing = Vec2::new(-yaw.sin(), -yaw.cos());
            assert!(
                facing.dot(spawn.direction().unwrap()) > 0.5,
                "facing {facing} at {p}"
            );
        }
    }

    #[test]
    fn drive_spots_lie_on_a_road_facing_the_heading() {
        let terrain = Terrain::baked();
        let on_road = |p: Vec2| {
            terrain.roads.iter().any(|road| {
                road.kind != RoadKind::Water
                    && road.points.windows(2).any(|pair| {
                        let (a, b) = (pair[0], pair[1]);
                        let t = ((p - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
                        a.lerp(b, t).distance(p) < road.width / 2.0
                    })
            })
        };
        for &(x, z) in &[(0.0, 0.0), (1500.0, -2000.0), (-3000.0, 2500.0)] {
            for heading in [Vec2::X, Vec2::NEG_Y] {
                let (spot, yaw) = terrain.drive_spot(Vec2::new(x, z), heading).unwrap();
                assert!(on_road(spot), "{spot} isn't on a road");
                assert!(terrain.inside(spot.x, spot.y));
                let facing = Vec2::new(-yaw.sin(), -yaw.cos());
                assert!(
                    facing.dot(heading) >= 0.0,
                    "facing {facing}, heading {heading}"
                );
            }
        }
    }

    #[test]
    fn drive_spots_move_along_the_road() {
        let terrain = Terrain::baked();
        let p = Vec2::new(1500.0, -2000.0);
        let (spot, _) = terrain.drive_spot(p, Vec2::X).unwrap();
        for shift in [-25.0, -8.0, 8.0, 25.0] {
            let (moved, _) = terrain.drive_spot_along(p, Vec2::X, shift).unwrap();
            let apart = moved.distance(spot);
            assert!(
                apart > 0.5 * shift.abs() && apart < shift.abs() + 0.5,
                "{shift} m along the road is {apart} m away"
            );
            // Still on that road: the nearest spot from there is itself.
            let (again, _) = terrain.drive_spot(moved, Vec2::X).unwrap();
            assert!(again.distance(moved) < 0.1, "{moved} isn't in the lane");
        }
    }

    #[test]
    fn roads_have_their_surface_and_the_land_its_own() {
        let terrain = Terrain::baked();
        // At each road's middle, the road's own surface, but for a few crossings.
        let (mut seen, mut own, mut count) = (Vec::new(), 0, 0);
        for road in terrain.roads.iter().filter(|r| r.kind != RoadKind::Water) {
            let p = road.points[road.points.len() / 2];
            own += (terrain.surface(p.x, p.y) == road.kind.surface()) as usize;
            count += 1;
            if !seen.contains(&road.kind) {
                seen.push(road.kind);
            }
        }
        assert!(own as f32 > 0.97 * count as f32, "{own} of {count}");
        assert_eq!(seen.len(), 4, "{seen:?}");
        // Away from the roads, the land use decides.
        let forest = (0..10_000)
            .map(|i| {
                Vec2::new(
                    (i % 100) as f32 * 80.0 - 4000.0,
                    (i / 100) as f32 * 80.0 - 4000.0,
                )
            })
            .find(|&p| terrain.land_class(p.x, p.y) == 8 && terrain.road_grid.near(p).is_empty())
            .unwrap();
        assert_eq!(terrain.surface(forest.x, forest.y), Surface::Forest);
    }

    #[test]
    fn bridges_carry_whats_on_them_and_let_roads_pass_under() {
        let terrain = Terrain::baked();
        let mut underpasses = 0;
        for road in terrain
            .roads
            .iter()
            .filter(|r| r.bridge && r.kind != RoadKind::Water)
        {
            for (i, (&p, &centre)) in road.points.iter().zip(&road.heights).enumerate() {
                let deck = terrain.ground(p.x, p.y, centre).height;
                assert!(
                    (deck - centre).abs() < 0.2,
                    "deck at {deck}, road at {centre}, {i} of {} at {p}, top {}, under {}",
                    road.points.len(),
                    terrain.ground(p.x, p.y, f32::INFINITY).height,
                    terrain.sample(p.x, p.y).0
                );
                let (under, _) = terrain.sample(p.x, p.y);
                // Well below, and a road there (where the terrain lies a little lower).
                if deck - under > 4.0 && under > terrain.land(p.x, p.y).0 + 0.02 {
                    underpasses += 1;
                    assert_eq!(terrain.ground(p.x, p.y, under + 0.3).height, under);
                    assert_eq!(terrain.ground(p.x, p.y, deck + 0.3).height, deck);
                }
            }
        }
        assert!(underpasses > 10, "{underpasses} points over roads");
    }

    #[test]
    fn potholes_and_bumps_are_in_the_road() {
        let mut terrain = Terrain::baked();
        // The middle of a long asphalt road, away from others.
        let (r, i) = terrain
            .roads
            .iter()
            .enumerate()
            .filter(|(_, r)| r.kind == RoadKind::Asphalt && !r.bridge && r.points.len() > 100)
            .find_map(|(r, road)| {
                let i = road.points.len() / 2;
                let p = road.points[i];
                (terrain
                    .road_grid
                    .near(p)
                    .iter()
                    .all(|&(o, _)| o as usize == r))
                .then_some((r, i))
            })
            .unwrap();
        let (p, along) = (terrain.roads[r].points[i], terrain.roads[r].along[i]);
        let before = terrain.sample(p.x, p.y).0;
        let feature = |kind, along, size, height| Feature {
            kind,
            along,
            right: 0.0,
            size,
            height,
        };
        terrain.roads[r].features = vec![
            feature(crate::relief::FeatureKind::Pothole, along, 0.8, -0.06),
            feature(crate::relief::FeatureKind::Bump, along + 5.0, 0.6, 0.08),
        ];
        let dip = terrain.sample(p.x, p.y).0 - before;
        assert!((dip + 0.06).abs() < 0.002, "pothole {dip} m deep");
        // The bump goes right across, also near the edge.
        let ahead = (terrain.roads[r].points[i + 5] - p).normalize();
        let q = p + ahead * 5.0 + ahead.perp() * (terrain.roads[r].width / 2.0 - 0.3);
        let without = {
            let saved = std::mem::take(&mut terrain.roads[r].features);
            let h = terrain.sample(q.x, q.y).0;
            terrain.roads[r].features = saved;
            h
        };
        let rise = terrain.sample(q.x, q.y).0 - without;
        assert!(rise > 0.05, "bump {rise} m high");
    }

    #[test]
    fn the_terrain_stays_under_bridge_decks() {
        let terrain = Terrain::baked();
        for road in terrain
            .roads
            .iter()
            .filter(|r| r.bridge && r.kind != RoadKind::Water)
        {
            let n = road.points.len();
            // Not at the ends, where the approaches carry on.
            for i in 1..n - 1 {
                let p = road.points[i];
                let across = (road.points[i + 1] - road.points[i - 1]).normalize().perp();
                for u in [-0.45, -0.25, 0.0, 0.25, 0.45] {
                    let q = p + across * u * road.width;
                    let deck = road.heights[i] - road.crown * (u * road.width).abs();
                    let land = terrain.land(q.x, q.y).0;
                    assert!(
                        land < deck - 0.05,
                        "terrain {land} through a deck at {deck}, at {q}"
                    );
                }
            }
        }
    }

    #[test]
    fn where_roads_overlap_only_the_best_is_drawn() {
        let terrain = Terrain::baked();
        let ribbons = RoadRibbons::new(&terrain);
        // How far inside each road (on the ground or not) `p` is, where it's inside.
        let inside = |p: Vec2| {
            let mut found: Vec<(usize, f32)> = Vec::new();
            for &(r, i) in terrain.road_grid.near(p) {
                let Some(road) = terrain.roads.get(r as usize) else {
                    continue;
                };
                let (a, b) = (road.points[i as usize], road.points[i as usize + 1]);
                let length = a.distance(b).max(1e-6);
                let t = (p - a).dot(b - a) / (length * length);
                // Ribbons end square.
                let n = road.points.len();
                if (t < 0.0 && i == 0) || (t > 1.0 && i as usize + 2 == n) {
                    continue;
                }
                let t = t.clamp(0.0, 1.0);
                let s = road.along[i as usize] + t * length;
                let margin = road.width_at(s) / 2.0 - a.lerp(b, t).distance(p);
                match found.iter_mut().find(|f| f.0 == r as usize) {
                    Some(f) => f.1 = f.1.max(margin),
                    None => found.push((r as usize, margin)),
                }
            }
            found.retain(|f| f.1 >= 0.0);
            found
        };
        let (mut checked, mut wrong) = (0, Vec::new());
        // At the junction the user found (a cobbled lane into Grubenweg), around Gerach
        // and in Baunach.
        for centre in [
            Vec2::new(-2945.5, -1939.2),
            Vec2::new(-2984.5, -2007.2),
            Vec2::new(-87.0, 2286.0),
        ] {
            let index = terrain.chunk_at(centre);
            let (mut roads, mut structure) = (FlatMesh::default(), FlatMesh::default());
            ribbons.mesh(&terrain, index, &mut roads, &mut structure);
            for (t, color) in roads.positions.chunks(3).zip(roads.colors.chunks(3)) {
                let c = t
                    .iter()
                    .fold(Vec2::ZERO, |c, v| c + Vec2::new(v[0], v[2]) / 3.0);
                let found = inside(c);
                let level = |r: usize| {
                    let road = &terrain.roads[r];
                    !road.bridge && road.kind != RoadKind::Water
                };
                if found.iter().any(|f| !level(f.0)) {
                    continue;
                }
                let best = found.iter().copied().reduce(|a, b| {
                    if outranks(&terrain.roads, b.0, a.0) {
                        b
                    } else {
                        a
                    }
                });
                if let Some((r, _)) = best.filter(|b| b.1 > 0.1 && found.len() > 1) {
                    checked += 1;
                    if color[0] != terrain.roads[r].kind.color() {
                        wrong.push(c);
                    }
                }
            }
        }
        assert!(
            checked > 200,
            "only {checked} triangles where roads overlap"
        );
        assert!(
            wrong.is_empty(),
            "{} of {checked} triangles show a lesser road, e.g. {:?}",
            wrong.len(),
            &wrong[..wrong.len().min(8)]
        );
    }

    #[test]
    fn physics_matches_coarse_render_triangles() {
        let terrain = Terrain::baked();
        let per_side = terrain.chunks_x();
        let mut coarse_chunks = 0;
        for (index, chunk) in terrain
            .chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| c.step > 1)
        {
            coarse_chunks += 1;
            let s = chunk.step;
            let (ix, iz) = (
                (index % per_side) * terrain.chunk_cells,
                (index / per_side) * terrain.chunk_cells,
            );
            let (a, b) = (terrain.vertex(ix, iz), terrain.vertex(ix + s, iz));
            let (c, d) = (terrain.vertex(ix, iz + s), terrain.vertex(ix + s, iz + s));
            // Points inside each rendered coarse triangle lie on its plane.
            for tri in [[a, d, b], [a, c, d]] {
                let p = (tri[0] + tri[1] * 2.0 + tri[2] * 3.0) / 6.0;
                let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
                let plane_y = tri[0].y
                    - (normal.x * (p.x - tri[0].x) + normal.z * (p.z - tri[0].z)) / normal.y;
                assert!((terrain.land(p.x, p.z).0 - plane_y).abs() < 1e-3);
            }
        }
        assert!(coarse_chunks > 0);
    }
}
