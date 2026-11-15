//! Real-world terrain baked by `tools/bake`: a heightfield rendered flat-shaded and queried
//! analytically (the physics sees exactly the triangles that are drawn), plus road ribbons.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// Gerach (96161, Bavaria) with Mauschendorf and Sendelbach: a 3 km square.
const BAKED: &[u8] = include_bytes!("../assets/gerach.terrain");
/// Width of the raised rim at the map edge (baked into the heights; roads stop before it).
const RIM: f32 = 30.0;
/// How far road ribbons float above the terrain, to avoid z-fighting.
const ROAD_LIFT: f32 = 0.05;
/// Maximum spacing of road ribbon vertices, along and across the road.
const DRAPE_STEP: f32 = 1.0;
/// Chunks whose centre is further from the camera are hidden: just past the fog (which
/// ends at 480 m) plus half a chunk diagonal.
const DRAW_DISTANCE: f32 = 560.0;

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Terrain::baked())
            .add_systems(Startup, spawn_terrain);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RoadKind {
    Paved,
    Unpaved,
    Water,
}

pub struct Road {
    pub kind: RoadKind,
    pub width: f32,
    /// Centreline in the xz plane (x east, z south).
    pub points: Vec<Vec2>,
}

#[derive(Resource)]
pub struct Terrain {
    /// Grid cells per side; each cell is two triangles.
    cells: usize,
    cell: f32,
    /// Heights at the full-detail grid vertices, `(cells + 1)²`, row-major in z.
    heights: Vec<f32>,
    /// Chunk side in cells, and the chunks, row-major in z.
    chunk_cells: usize,
    chunks: Vec<Chunk>,
    /// Where the cars start (they're placed on the nearest paved road).
    spawn: Vec2,
    pub roads: Vec<Road>,
    pub buildings: Vec<Building>,
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
        assert_eq!(r.u32(), 4, "unsupported terrain version");
        let cells = r.u32() as usize;
        let cell = r.f32();
        let chunk_cells = r.u32() as usize;
        let spawn = Vec2::new(r.f32(), r.f32());
        let n = cells + 1;
        let mut heights = vec![0.0; n * n];
        let mut chunks = Vec::new();
        for cz in 0..cells / chunk_cells {
            for cx in 0..cells / chunk_cells {
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
            cells,
            cell,
            heights,
            chunk_cells,
            chunks,
            spawn,
            roads,
            buildings,
        }
    }

    pub fn half_size(&self) -> f32 {
        self.cells as f32 * self.cell / 2.0
    }

    fn vertex(&self, ix: usize, iz: usize) -> Vec3 {
        Vec3::new(
            ix as f32 * self.cell - self.half_size(),
            self.heights[iz * (self.cells + 1) + ix],
            iz as f32 * self.cell - self.half_size(),
        )
    }

    /// The triangle of the rendered mesh that contains (x, z), wound counter-clockwise from above.
    fn triangle(&self, x: f32, z: f32) -> [Vec3; 3] {
        let max = self.cells as f32 - 1e-3;
        let gx = ((x + self.half_size()) / self.cell).clamp(0.0, max);
        let gz = ((z + self.half_size()) / self.cell).clamp(0.0, max);
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
        let max = self.cells - 1;
        let grid = |v: f32| (((v + self.half_size()) / self.cell).max(0.0) as usize).min(max);
        let (ix, iz) = (grid(x), grid(z));
        let per_side = self.cells / self.chunk_cells;
        let chunk = &self.chunks[iz / self.chunk_cells * per_side + ix / self.chunk_cells];
        let m = self.chunk_cells / chunk.step;
        let local = |i: usize| i % self.chunk_cells / chunk.step;
        chunk.classes[local(iz) * m + local(ix)]
    }

    /// Whether (x, z) is inside the map, away from the rim that slopes down at its edge.
    pub fn inside(&self, x: f32, z: f32) -> bool {
        let inner = self.half_size() - RIM;
        x.abs() < inner && z.abs() < inner
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

    /// `count` spots along the paved road nearest the spawn point, `spacing` metres apart,
    /// as (position in xz, yaw facing along the road).
    pub fn road_spawns(&self, count: usize, spacing: f32) -> Vec<(Vec2, f32)> {
        let (road, start) = self
            .roads
            .iter()
            .filter(|r| r.kind == RoadKind::Paved && r.width >= 5.0)
            .flat_map(|r| {
                r.points
                    .iter()
                    .enumerate()
                    .map(move |(i, p)| (r, i, p.distance(self.spawn)))
            })
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(r, i, _)| (r, i))
            .expect("map has no paved roads");
        // Walk towards whichever end of the road is further away.
        let forward = start < road.points.len() / 2;
        let mut points: Vec<Vec2> = if forward {
            road.points[start..].to_vec()
        } else {
            road.points[..=start].iter().rev().copied().collect()
        };
        points.dedup();
        let mut spawns = Vec::new();
        let mut next = 0.0;
        let mut travelled = 0.0;
        for pair in points.windows(2) {
            let length = pair[0].distance(pair[1]);
            while spawns.len() < count && next <= travelled + length {
                let along = pair[0].lerp(pair[1], (next - travelled) / length);
                let dir = (pair[1] - pair[0]) / length;
                spawns.push((along, (-dir.x).atan2(-dir.y)));
                next += spacing;
            }
            travelled += length;
        }
        spawns
    }

    /// Flat-shaded mesh: every triangle gets its own vertices, coloured by land use and slope.
    fn mesh(&self) -> ChunkedMesh {
        let mut meshes = ChunkedMesh::new(self);
        let per_side = self.cells / self.chunk_cells;
        for (index, chunk) in self.chunks.iter().enumerate() {
            // Each chunk is meshed at its own resolution.
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
                    let mesh = meshes.at(a);
                    for tri in [[a, d, b], [a, c, d]] {
                        let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize();
                        let class = chunk.classes[jz * m + jx];
                        mesh.triangle(tri, ground_color(class, normal.y, ix * 31 + iz * 17));
                    }
                }
            }
        }
        meshes
    }

    /// Ribbons along the road centrelines, draped onto the terrain.
    fn road_mesh(&self) -> ChunkedMesh {
        let mut chunks = ChunkedMesh::new(self);
        let lift = |p: Vec2| Vec3::new(p.x, self.sample(p.x, p.y).0 + ROAD_LIFT, p.y);
        for road in &self.roads {
            let color = match road.kind {
                RoadKind::Paved => [0.24, 0.24, 0.26, 1.0],
                RoadKind::Unpaved => [0.56, 0.49, 0.38, 1.0],
                RoadKind::Water => [0.22, 0.42, 0.58, 1.0],
            };
            // Skip where the road climbs the rim at the map edge.
            let inside = |p: &Vec2| self.inside(p.x, p.y);
            for run in road
                .points
                .split(|p| !inside(p))
                .filter(|run| run.len() >= 2)
            {
                // Drape the ribbon: vertices at most `DRAPE_STEP` apart along and across
                // the road, each set onto the terrain, so it follows the 2 m terrain mesh.
                let run = resample(run, DRAPE_STEP);
                let n = run.len();
                let across = (road.width / DRAPE_STEP).ceil().max(1.0) as usize;
                let rows: Vec<Vec<Vec3>> = (0..n)
                    .map(|i| {
                        let tangent = (run[(i + 1).min(n - 1)] - run[i.saturating_sub(1)])
                            .normalize_or_zero();
                        let side = Vec2::new(-tangent.y, tangent.x) * road.width / 2.0;
                        (0..=across)
                            .map(|j| lift(run[i] - side + side * (2.0 * j as f32 / across as f32)))
                            .collect()
                    })
                    .collect();
                for pair in rows.windows(2) {
                    for j in 0..across {
                        let (a, b) = (pair[0][j], pair[0][j + 1]);
                        let (c, d) = (pair[1][j], pair[1][j + 1]);
                        let mesh = chunks.at(a);
                        mesh.triangle([a, b, c], color);
                        mesh.triangle([b, d, c], color);
                    }
                }
            }
        }
        chunks
    }
}

/// Flat meshes split into square chunks by position, so the camera can cull what's
/// behind it or beyond the fog instead of drawing the whole map every frame.
pub struct ChunkedMesh {
    half_size: f32,
    chunk_size: f32,
    per_side: usize,
    chunks: Vec<FlatMesh>,
}

impl ChunkedMesh {
    /// Empty meshes, one per terrain chunk.
    pub fn new(terrain: &Terrain) -> Self {
        let per_side = terrain.cells / terrain.chunk_cells;
        let chunks = (0..per_side * per_side)
            .map(|_| FlatMesh::default())
            .collect();
        Self {
            half_size: terrain.half_size(),
            chunk_size: terrain.chunk_cells as f32 * terrain.cell,
            per_side,
            chunks,
        }
    }

    /// The chunk containing `p` (by its x and z).
    pub fn at(&mut self, p: Vec3) -> &mut FlatMesh {
        let index = |v: f32| {
            (((v + self.half_size) / self.chunk_size).max(0.0) as usize).min(self.per_side - 1)
        };
        &mut self.chunks[index(p.z) * self.per_side + index(p.x)]
    }

    /// One entity per non-empty chunk, hidden beyond `DRAW_DISTANCE`.
    pub fn spawn(
        self,
        commands: &mut Commands,
        meshes: &mut Assets<Mesh>,
        material: Handle<StandardMaterial>,
        casts_shadows: bool,
    ) {
        for chunk in self
            .chunks
            .into_iter()
            .filter(|chunk| !chunk.positions.is_empty())
        {
            let mut entity = commands.spawn((
                Mesh3d(meshes.add(chunk.build())),
                MeshMaterial3d(material.clone()),
                // Measured from each chunk's bounding box centre, as vertices are in world space.
                VisibilityRange {
                    use_aabb: true,
                    ..VisibilityRange::abrupt(0.0, DRAW_DISTANCE)
                },
            ));
            if !casts_shadows {
                entity.insert(NotShadowCaster);
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

fn spawn_terrain(
    mut commands: Commands,
    terrain: Res<Terrain>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Neither casts shadows: on gentle real terrain self-shadowing is barely visible, and
    // skipping it keeps the big meshes out of every shadow cascade. They still receive
    // shadows from houses and cars.
    let ground = materials.add(StandardMaterial {
        perceptual_roughness: 0.95,
        ..default()
    });
    terrain
        .mesh()
        .spawn(&mut commands, &mut meshes, ground, false);
    let roads = materials.add(StandardMaterial {
        perceptual_roughness: 0.9,
        depth_bias: 1.0,
        ..default()
    });
    terrain
        .road_mesh()
        .spawn(&mut commands, &mut meshes, roads, false);
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
        // Near the baked spawn point (Gerach), not somewhere else on the map.
        assert!(spawns[0].0.distance(terrain.spawn) < 200.0);
    }

    #[test]
    fn physics_matches_coarse_render_triangles() {
        let terrain = Terrain::baked();
        let per_side = terrain.cells / terrain.chunk_cells;
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
