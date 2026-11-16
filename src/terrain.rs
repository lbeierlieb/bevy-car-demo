//! Real-world terrain baked by `tools/bake`: a heightfield rendered flat-shaded and queried
//! analytically (the physics sees exactly the triangles that are drawn), plus road ribbons.
//! Meshes are built per chunk, on demand (see `scenery`).

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::physics::collision::Obstacles;

/// From Gerach (96161, Bavaria) to Rattelsdorf, Ebing and Baunach: 9.6 × 8.4 km.
const BAKED: &[u8] = include_bytes!("../assets/gerach.terrain");
/// Width of the raised rim at the map edge (baked into the heights; roads stop before it).
const RIM: f32 = 30.0;
/// How far road ribbons float above the terrain, to avoid z-fighting.
const ROAD_LIFT: f32 = 0.05;
/// Maximum spacing of road ribbon vertices, along and across the road.
const DRAPE_STEP: f32 = 1.0;

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Terrain::baked());
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RoadKind {
    Paved,
    Unpaved,
    Water,
}

impl RoadKind {
    /// Vertex colour of the ribbon (linear).
    pub fn color(self) -> [f32; 4] {
        match self {
            RoadKind::Paved => [0.24, 0.24, 0.26, 1.0],
            RoadKind::Unpaved => [0.56, 0.49, 0.38, 1.0],
            RoadKind::Water => [0.22, 0.42, 0.58, 1.0],
        }
    }
}

pub struct Road {
    pub kind: RoadKind,
    pub width: f32,
    /// Centreline in the xz plane (x east, z south).
    pub points: Vec<Vec2>,
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
    /// Where the cars start (they're placed on the nearest paved road), and the direction
    /// they should face along it (zero if none).
    spawn: Vec2,
    spawn_heading: Vec2,
    pub roads: Vec<Road>,
    pub buildings: Vec<Building>,
    /// Houses and tree trunks as boxes, for collisions (filled in at startup, see
    /// `obstacles.rs`; empty in tests unless built).
    pub obstacles: Obstacles,
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
    /// Parses the embedded map (format described in `tools/bake`). Chunks are stored at
    /// different resolutions; the full-detail height grid is rebuilt from them, with the
    /// vertices of coarse chunks lying on their coarse triangles, so physics queries match
    /// the rendered surface exactly.
    pub fn baked() -> Self {
        let mut r = Bytes(BAKED);
        assert_eq!(r.take(4), b"CSTR", "not a baked terrain file");
        assert_eq!(r.u32(), 6, "unsupported terrain version");
        let (cells_x, cells_z) = (r.u32() as usize, r.u32() as usize);
        let cell = r.f32();
        let chunk_cells = r.u32() as usize;
        let spawn = Vec2::new(r.f32(), r.f32());
        let spawn_heading = Vec2::new(r.f32(), r.f32());
        let n = cells_x + 1;
        let mut heights = vec![0.0; n * (cells_z + 1)];
        let mut chunks = Vec::new();
        for cz in 0..cells_z / chunk_cells {
            for cx in 0..cells_x / chunk_cells {
                let step = r.take(1)[0] as usize;
                let m = chunk_cells / step;
                let coarse: Vec<f32> = (0..(m + 1) * (m + 1)).map(|_| r.f32()).collect();
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
        let roads = (0..r.u32())
            .map(|_| {
                let kind =
                    [RoadKind::Paved, RoadKind::Unpaved, RoadKind::Water][r.take(1)[0] as usize];
                let width = r.f32();
                let points = (0..r.u32()).map(|_| Vec2::new(r.f32(), r.f32())).collect();
                Road {
                    kind,
                    width,
                    points,
                }
            })
            .collect();
        let buildings = (0..r.u32())
            .map(|_| {
                let levels = r.f32();
                let outline = (0..r.u32()).map(|_| Vec2::new(r.f32(), r.f32())).collect();
                Building { levels, outline }
            })
            .collect();
        Self {
            cells_x,
            cells_z,
            cell,
            heights,
            chunk_cells,
            chunks,
            spawn,
            spawn_heading,
            roads,
            buildings,
            obstacles: Obstacles::default(),
        }
    }

    /// A flat, empty square 1 km across at height 0, for driving tests.
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
                    classes: vec![0],
                })
                .collect(),
            spawn: Vec2::ZERO,
            spawn_heading: Vec2::NEG_Y,
            roads: Vec::new(),
            buildings: Vec::new(),
            obstacles: Obstacles::default(),
        }
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

    /// Surface height and upward normal at (x, z).
    pub fn sample(&self, x: f32, z: f32) -> (f32, Vec3) {
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

    /// Ground colour at (x, z), as the terrain is drawn there.
    pub fn ground_color_at(&self, x: f32, z: f32, variation: usize) -> [f32; 4] {
        let (_, normal) = self.sample(x, z);
        ground_color(self.land_class(x, z), normal.y, variation)
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
    pub fn road_spawns(&self, count: usize, spacing: f32) -> Vec<(Vec2, f32)> {
        let (road, i, start) = self
            .closest_on_road(self.spawn, |r| r.kind == RoadKind::Paved && r.width >= 5.0)
            .expect("map has no paved roads");
        // Face along the spawn heading; without one, so there's more road behind.
        let n = road.points.len();
        let forward = if self.spawn_heading == Vec2::ZERO {
            i >= n / 2
        } else {
            (road.points[i + 1] - road.points[i]).dot(self.spawn_heading) >= 0.0
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

    /// The drivable road point closest to `p` (inside the map), as (position in xz, yaw
    /// facing along the road in the direction closest to `heading`).
    pub fn drive_spot(&self, p: Vec2, heading: Vec2) -> Option<(Vec2, f32)> {
        let (road, i, point) = self.closest_on_road(p, |r| r.kind != RoadKind::Water)?;
        let along = (road.points[i + 1] - road.points[i]).normalize_or(Vec2::NEG_Y);
        let facing = if along.dot(heading) >= 0.0 {
            along
        } else {
            -along
        };
        Some((point, (-facing.x).atan2(-facing.y)))
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

/// Ribbons along the road centrelines, draped onto the terrain and meshed per chunk.
pub struct RoadRibbons {
    /// Each road's runs inside the map, resampled to `DRAPE_STEP`.
    ribbons: Vec<Ribbon>,
    /// Per chunk, the (ribbon, row) pairs whose quads start there.
    by_chunk: Vec<Vec<(usize, usize)>>,
}

struct Ribbon {
    color: [f32; 4],
    width: f32,
    points: Vec<Vec2>,
}

impl RoadRibbons {
    pub fn new(terrain: &Terrain) -> Self {
        let mut ribbons = Vec::new();
        for road in &terrain.roads {
            let color = road.kind.color();
            // Skip where the road climbs the rim at the map edge.
            for run in road
                .points
                .split(|p| !terrain.inside(p.x, p.y))
                .filter(|run| run.len() >= 2)
            {
                ribbons.push(Ribbon {
                    color,
                    width: road.width,
                    points: resample(run, DRAPE_STEP),
                });
            }
        }
        let mut by_chunk = vec![Vec::new(); terrain.chunk_count()];
        for (r, ribbon) in ribbons.iter().enumerate() {
            for (i, p) in ribbon.points[..ribbon.points.len() - 1].iter().enumerate() {
                by_chunk[terrain.chunk_at(*p)].push((r, i));
            }
        }
        Self { ribbons, by_chunk }
    }

    /// The ribbon quads starting in one chunk. Vertices are at most `DRAPE_STEP` apart
    /// along and across the road, each set onto the terrain, so the ribbon follows the
    /// 2 m terrain mesh.
    pub fn mesh(&self, terrain: &Terrain, index: usize, mesh: &mut FlatMesh) {
        let lift = |p: Vec2| Vec3::new(p.x, terrain.sample(p.x, p.y).0 + ROAD_LIFT, p.y);
        for &(r, i) in &self.by_chunk[index] {
            let ribbon = &self.ribbons[r];
            let (points, n) = (&ribbon.points, ribbon.points.len());
            let across = (ribbon.width / DRAPE_STEP).ceil().max(1.0) as usize;
            let row = |i: usize| -> Vec<Vec3> {
                let tangent =
                    (points[(i + 1).min(n - 1)] - points[i.saturating_sub(1)]).normalize_or_zero();
                let side = Vec2::new(-tangent.y, tangent.x) * ribbon.width / 2.0;
                (0..=across)
                    .map(|j| lift(points[i] - side + side * (2.0 * j as f32 / across as f32)))
                    .collect()
            };
            let (near, far) = (row(i), row(i + 1));
            for j in 0..across {
                let (a, b) = (near[j], near[j + 1]);
                let (c, d) = (far[j], far[j + 1]);
                mesh.triangle([a, b, c], ribbon.color);
                mesh.triangle([b, d, c], ribbon.color);
            }
        }
    }
}

/// Resamples a polyline so consecutive points are at most `step` apart, keeping the
/// original vertices.
fn resample(points: &[Vec2], step: f32) -> Vec<Vec2> {
    let mut out = vec![points[0]];
    for pair in points.windows(2) {
        let steps = (pair[0].distance(pair[1]) / step).ceil().max(1.0) as usize;
        out.extend((1..=steps).map(|i| pair[0].lerp(pair[1], i as f32 / steps as f32)));
    }
    out
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

/// Little-endian reader over the baked file.
struct Bytes(&'static [u8]);

impl Bytes {
    fn take(&mut self, n: usize) -> &'static [u8] {
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        head
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

/// Ground colour for a land use class (see `tools/bake`), with steep natural slopes
/// showing bare earth.
fn ground_color(class: u8, up: f32, variation: usize) -> [f32; 4] {
    let jitter = (hash(variation as u32) - 0.5) * 0.04;
    let base = match class {
        1 => Vec3::new(0.42, 0.56, 0.33),  // residential: gardens
        2 => Vec3::new(0.78, 0.68, 0.42),  // field: ripe grain
        3 => Vec3::new(0.70, 0.62, 0.42),  // field: stubble
        4 => Vec3::new(0.52, 0.41, 0.30),  // field: ploughed
        5 => Vec3::new(0.46, 0.60, 0.26),  // field: green crop
        6 => Vec3::new(0.36, 0.58, 0.28),  // meadow
        7 => Vec3::new(0.32, 0.44, 0.24),  // scrub
        8 => Vec3::new(0.17, 0.32, 0.15),  // forest
        9 => Vec3::new(0.22, 0.42, 0.58),  // water
        10 => Vec3::new(0.30, 0.30, 0.32), // paved area
        _ => Vec3::new(0.34, 0.53, 0.27),  // unmapped: grass
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
    fn spawns_lie_on_a_road() {
        let terrain = Terrain::baked();
        let spawns = terrain.road_spawns(3, 9.0);
        assert_eq!(spawns.len(), 3);
        assert!(spawns[0].0.distance(spawns[2].0) > 10.0);
        // At the baked spawn point (Gerach), facing along the spawn heading.
        assert!(spawns[0].0.distance(terrain.spawn) < 30.0);
        for (p, yaw) in &spawns {
            let facing = Vec2::new(-yaw.sin(), -yaw.cos());
            assert!(
                facing.dot(terrain.spawn_heading) > 0.5,
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
                        a.lerp(b, t).distance(p) < 0.01
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
                assert!((terrain.sample(p.x, p.z).0 - plane_y).abs() < 1e-3);
            }
        }
        assert!(coarse_chunks > 0);
    }
}
