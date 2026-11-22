//! Real-world terrain baked by `tools/bake`: a heightfield rendered flat-shaded and queried
//! analytically (the physics sees exactly the triangles that are drawn), and roads with
//! their own heights on it (`Terrain::ground`), drawn as ribbons on that same surface.
//! Meshes are built per chunk, on demand (see `scenery`).

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::fences::{Fence, FenceKind};
use crate::physics::collision::Obstacles;
use crate::physics::surface::Surface;
use crate::relief::{Feature, Waves};

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
/// Where roads overlap (junctions), the surface blends them, each by how far inside its
/// edge the point is, fading out over this distance towards the road's ends (m), so the
/// surface stays continuous where a side road's crown meets a main road's camber.
const ROAD_FADE: f32 = 3.0;
/// A surface carries something at most this far below it (m): a car on a bridge is on the
/// deck, one under it on the road below.
const LEVEL_REACH: f32 = 1.0;

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
    pub width: f32,
    pub bridge: bool,
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
    /// The road segments near each cell, for `surface`.
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
        if version != 14 {
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
        let half = Vec2::new(cells_x as f32, cells_z as f32) * cell / 2.0;
        Ok(Self {
            cells_x,
            cells_z,
            cell,
            heights,
            chunk_cells,
            chunks,
            base,
            road_grid: RoadGrid::new(&roads, half),
            roads,
            buildings,
            fences,
            obstacles: Obstacles::default(),
            // FNV-1a.
            fingerprint: file.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
            }),
        })
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
            road_grid: RoadGrid::new(&[], Vec2::splat(cells as f32 * 2.0)),
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
            crown: 0.025,
            heights: vec![0.1; points.len()],
            points,
            along,
            roughness,
            waves: Waves::new(roughness, 1),
            features: Vec::new(),
        });
        terrain.road_grid = RoadGrid::new(&terrain.roads, terrain.half_size());
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
        let p = Vec2::new(x, z);
        // Per road covering it: (road, weight, height, slope as d height / d (x, z)).
        let mut covering: [(usize, f32, f32, Vec2); 8] = [(usize::MAX, 0.0, 0.0, Vec2::ZERO); 8];
        let mut count = 0;
        let mut water = false;
        for &(r, i) in self.road_grid.near(p) {
            let (r, i) = (r as usize, i as usize);
            let road = &self.roads[r];
            let (a, b) = (road.points[i], road.points[i + 1]);
            let length = a.distance(b).max(1e-6);
            let along = (b - a) / length;
            let t = ((p - a).dot(along) / length).clamp(0.0, 1.0);
            let off = a.lerp(b, t).distance(p);
            let margin = road.width / 2.0 - off;
            if margin < 0.0 {
                continue;
            }
            if road.kind == RoadKind::Water {
                water = true;
                continue;
            }
            let s = road.along[i] + t * length;
            let to_end = s.min(road.along[road.along.len() - 1] - s);
            let weight = margin.max(1e-3) * (to_end / ROAD_FADE).clamp(0.02, 1.0);
            let side = along.perp();
            let right = (p - a).dot(side);
            let rise = (road.heights[i + 1] - road.heights[i]) / length;
            let (bump, tilt) = road.relief(s, right, margin);
            let height = road.heights[i] + rise * t * length - road.crown * off + bump;
            let slope = along * (rise + tilt.x) + side * (tilt.y - road.crown * right.signum());
            // One entry per road: the segment it covers the point best with.
            match covering[..count].iter_mut().find(|c| c.0 == r) {
                Some(entry) if entry.1 >= weight => {}
                Some(entry) => *entry = (r, weight, height, slope),
                None if count < 8 => {
                    covering[count] = (r, weight, height, slope);
                    count += 1;
                }
                None => {}
            }
        }
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
            let main = layer().max_by(|a, b| a.1.total_cmp(&b.1))?;
            let slope = layer().map(|c| c.1 * c.3).sum::<Vec2>() / total;
            Some(Ground {
                height: layer().map(|c| c.1 * c.2).sum::<f32>() / total,
                normal: Vec3::new(-slope.x, 1.0, -slope.y).normalize(),
                surface: self.roads[main.0].kind.surface(),
            })
        };
        let below = blend(Some(false)).unwrap_or_else(|| {
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
        match blend(Some(true)) {
            // Level with the roads on the ground (where they meet a bridge's end): one.
            Some(deck) if (deck.height - below.height).abs() < LEVEL_REACH / 2.0 => {
                blend(None).unwrap()
            }
            Some(deck) if deck.height <= y + LEVEL_REACH && deck.height > below.height => deck,
            _ => below,
        }
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
        let (road, i, point) = self.closest_on_road(p, |r| r.kind != RoadKind::Water)?;
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
/// over the map, in compressed rows.
struct RoadGrid {
    half: Vec2,
    cells_x: usize,
    cells_z: usize,
    /// Where each cell's entries start in `entries` (one more than there are cells).
    starts: Vec<u32>,
    entries: Vec<(u32, u32)>,
}

impl RoadGrid {
    fn new(roads: &[Road], half: Vec2) -> Self {
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
                let reach = Vec2::splat(road.width / 2.0);
                let (lo, hi) = (pair[0].min(pair[1]) - reach, pair[0].max(pair[1]) + reach);
                let ((x0, z0), (x1, z1)) = (grid.cell(lo), grid.cell(hi));
                for cz in z0..=z1 {
                    for cx in x0..=x1 {
                        cells[cz * cells_x + cx].push((r as u32, i as u32));
                    }
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
}

impl RoadRibbons {
    pub fn new(terrain: &Terrain) -> Self {
        let mut by_chunk = vec![Vec::new(); terrain.chunk_count()];
        for (r, road) in terrain.roads.iter().enumerate() {
            for (i, p) in road.points[..road.points.len() - 1].iter().enumerate() {
                by_chunk[terrain.chunk_at(*p)].push((r, i));
            }
        }
        Self { by_chunk }
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
            let road = &terrain.roads[r];
            let (points, n) = (&road.points, road.points.len());
            // Finer where there's more to see: twice on rough roads, four times around
            // bumps and potholes.
            let fine = if road.feature_near(road.along[i], 1.0) {
                4
            } else if road.roughness >= 64.0 {
                2
            } else {
                1
            };
            let step = DRAPE_STEP / fine as f32;
            let across = 2 * (road.width / 2.0 / step).ceil().max(1.0) as usize;
            // Half the road across at point `i`.
            let side = |i: usize| {
                let tangent =
                    (points[(i + 1).min(n - 1)] - points[i.saturating_sub(1)]).normalize_or_zero();
                Vec2::new(-tangent.y, tangent.x) * road.width / 2.0
            };
            // The vertices across the road `f` of the way from point `i` to the next.
            let row = |f: f32| -> Vec<Vec3> {
                let centre = points[i].lerp(points[i + 1], f);
                let side = side(i).lerp(side(i + 1), f).normalize_or_zero() * road.width / 2.0;
                let level = road.heights[i] + (road.heights[i + 1] - road.heights[i]) * f;
                (0..=across)
                    .map(|j| {
                        let u = 2.0 * j as f32 / across as f32 - 1.0;
                        let p = centre + side * u;
                        // Looked up a little inside the edge, which may lie just outside
                        // the segments' cover on a bend (and would drop off a bridge).
                        let q = centre + side * u * 0.98;
                        let ground = if road.kind == RoadKind::Water {
                            terrain.land(p.x, p.y).0
                        } else {
                            terrain.ground(q.x, q.y, level).height
                        };
                        Vec3::new(p.x, ground + road.kind.lift(), p.y)
                    })
                    .collect()
            };
            let rows: Vec<Vec<Vec3>> = (0..=fine).map(|k| row(k as f32 / fine as f32)).collect();
            let color = road.kind.color();
            for pair in rows.windows(2) {
                let (near, far) = (&pair[0], &pair[1]);
                for j in 0..across {
                    let (a, b) = (near[j], near[j + 1]);
                    let (c, d) = (far[j], far[j + 1]);
                    mesh.triangle([a, b, c], color);
                    mesh.triangle([b, d, c], color);
                }
            }
            let (near, far) = (&rows[0], &rows[fine]);
            if road.bridge && road.kind != RoadKind::Water {
                let edges = |row: &[Vec3], i: usize| [(row[0], -side(i)), (row[across], side(i))];
                bridge_segment(
                    edges(near, i),
                    edges(far, i + 1),
                    (i == 0, i + 2 == n),
                    structure,
                );
            }
        }
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
