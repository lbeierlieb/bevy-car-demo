//! Real-world terrain baked by `tools/bake`: a heightfield rendered flat-shaded and queried
//! analytically (the physics sees exactly the triangles that are drawn), plus road ribbons.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// Gerach (96161, Bavaria): 1.6 km square around the village.
const BAKED: &[u8] = include_bytes!("../assets/gerach.terrain");
/// Width of the raised rim that keeps cars on the map.
const RIM: f32 = 30.0;
/// How far road ribbons float above the terrain, to avoid z-fighting.
const ROAD_LIFT: f32 = 0.05;

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Terrain::baked())
            .add_systems(Startup, spawn_terrain);
    }
}

pub struct Road {
    pub paved: bool,
    pub width: f32,
    /// Centreline in the xz plane (x east, z south).
    pub points: Vec<Vec2>,
}

#[derive(Resource)]
pub struct Terrain {
    /// Grid cells per side; each cell is two triangles.
    cells: usize,
    cell: f32,
    /// Heights at the grid vertices, `(cells + 1)²`, row-major in z.
    heights: Vec<f32>,
    pub roads: Vec<Road>,
    pub buildings: Vec<Building>,
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
    /// Parses the embedded map (format described in `tools/bake`) and raises the rim.
    pub fn baked() -> Self {
        let mut r = Bytes(BAKED);
        assert_eq!(r.take(4), b"CSTR", "not a baked terrain file");
        assert_eq!(r.u32(), 2, "unsupported terrain version");
        let cells = r.u32() as usize;
        let cell = r.f32();
        let mut heights: Vec<f32> = (0..(cells + 1) * (cells + 1)).map(|_| r.f32()).collect();
        let roads = (0..r.u32())
            .map(|_| {
                let paved = r.take(1)[0] != 0;
                let width = r.f32();
                let points = (0..r.u32()).map(|_| Vec2::new(r.f32(), r.f32())).collect();
                Road {
                    paved,
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

        let half = cells as f32 * cell / 2.0;
        for (i, h) in heights.iter_mut().enumerate() {
            let (x, z) = (
                (i % (cells + 1)) as f32 * cell - half,
                (i / (cells + 1)) as f32 * cell - half,
            );
            let edge = (x.abs().max(z.abs()) - (half - RIM)).max(0.0);
            *h += edge * edge * 0.05;
        }
        Self {
            cells,
            cell,
            heights,
            roads,
            buildings,
        }
    }

    fn half_size(&self) -> f32 {
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

    /// `count` spots along the paved road nearest the map centre, `spacing` metres apart,
    /// as (position in xz, yaw facing along the road).
    pub fn road_spawns(&self, count: usize, spacing: f32) -> Vec<(Vec2, f32)> {
        let (road, start) = self
            .roads
            .iter()
            .filter(|r| r.paved && r.width >= 5.0)
            .flat_map(|r| {
                r.points
                    .iter()
                    .enumerate()
                    .map(move |(i, p)| (r, i, p.length()))
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

    /// Flat-shaded mesh: every triangle gets its own vertices, coloured by height and slope.
    fn mesh(&self) -> Mesh {
        let mut mesh = FlatMesh::default();
        let relief = self.heights.iter().copied().fold(0.0, f32::max).max(1.0);
        for iz in 0..self.cells {
            for ix in 0..self.cells {
                let (a, b) = (self.vertex(ix, iz), self.vertex(ix + 1, iz));
                let (c, d) = (self.vertex(ix, iz + 1), self.vertex(ix + 1, iz + 1));
                for tri in [[a, d, b], [a, c, d]] {
                    let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize();
                    let height = (tri[0].y + tri[1].y + tri[2].y) / 3.0 / relief;
                    mesh.triangle(tri, ground_color(height, normal.y, ix * 31 + iz * 17));
                }
            }
        }
        mesh.build()
    }

    /// Ribbons along the road centrelines, draped onto the terrain.
    fn road_mesh(&self) -> Mesh {
        let mut mesh = FlatMesh::default();
        let inner = self.half_size() - RIM;
        let lift = |p: Vec2| Vec3::new(p.x, self.sample(p.x, p.y).0 + ROAD_LIFT, p.y);
        for road in &self.roads {
            let color = if road.paved {
                [0.24, 0.24, 0.26, 1.0]
            } else {
                [0.56, 0.49, 0.38, 1.0]
            };
            // Skip where the road climbs the rim at the map edge.
            let inside = |p: &Vec2| p.x.abs() < inner && p.y.abs() < inner;
            for run in road
                .points
                .split(|p| !inside(p))
                .filter(|run| run.len() >= 2)
            {
                let n = run.len();
                let edges: Vec<(Vec3, Vec3)> = (0..n)
                    .map(|i| {
                        let tangent = (run[(i + 1).min(n - 1)] - run[i.saturating_sub(1)])
                            .normalize_or_zero();
                        let side = Vec2::new(-tangent.y, tangent.x) * road.width / 2.0;
                        (lift(run[i] - side), lift(run[i] + side))
                    })
                    .collect();
                for pair in edges.windows(2) {
                    let ((l0, r0), (l1, r1)) = (pair[0], pair[1]);
                    mesh.triangle([l0, r0, l1], color);
                    mesh.triangle([r0, r1, l1], color);
                }
            }
        }
        mesh.build()
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

/// `height` is relative: 0 at the lowest point of the map, 1 at the highest.
fn ground_color(height: f32, up: f32, variation: usize) -> [f32; 4] {
    let jitter = (hash(variation as u32) - 0.5) * 0.06;
    let base = if up < 0.85 {
        Vec3::new(0.45, 0.40, 0.36) // steep: rock
    } else {
        // Lush valley floor to drier fields on the hilltops.
        Vec3::new(0.30, 0.50, 0.26).lerp(Vec3::new(0.55, 0.60, 0.38), height.min(1.0))
    };
    (base + Vec3::splat(jitter)).extend(1.0).to_array()
}

fn spawn_terrain(
    mut commands: Commands,
    terrain: Res<Terrain>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Mesh3d(meshes.add(terrain.mesh())),
        MeshMaterial3d(materials.add(StandardMaterial {
            perceptual_roughness: 0.95,
            ..default()
        })),
    ));
    commands.spawn((
        Mesh3d(meshes.add(terrain.road_mesh())),
        MeshMaterial3d(materials.add(StandardMaterial {
            perceptual_roughness: 0.9,
            depth_bias: 1.0,
            ..default()
        })),
    ));
}

/// Integer hash to [0, 1).
fn hash(n: u32) -> f32 {
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
    }
}
