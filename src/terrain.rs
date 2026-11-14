//! Procedural low-poly terrain: a noise heightfield, rendered flat-shaded, and queried
//! analytically (the physics sees exactly the triangles that are drawn).

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// Side length of the square terrain, in metres.
pub const SIZE: f32 = 512.0;
/// Grid cells per side; each cell is two triangles.
const CELLS: usize = 128;
const CELL: f32 = SIZE / CELLS as f32;

pub struct TerrainPlugin;

impl Plugin for TerrainPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Terrain::generate(7))
            .add_systems(Startup, spawn_terrain);
    }
}

/// Heights at the grid vertices, `(CELLS + 1)²`, row-major in z.
#[derive(Resource)]
pub struct Terrain {
    heights: Vec<f32>,
}

pub struct RayHit {
    pub distance: f32,
    pub point: Vec3,
    pub normal: Vec3,
}

impl Terrain {
    pub fn generate(seed: u32) -> Self {
        let n = CELLS + 1;
        let mut heights = Vec::with_capacity(n * n);
        for iz in 0..n {
            for ix in 0..n {
                let x = ix as f32 * CELL - SIZE / 2.0;
                let z = iz as f32 * CELL - SIZE / 2.0;
                // Rolling hills plus smaller bumps, flattened around the spawn area.
                let hills = fbm(seed, x / 160.0, z / 160.0, 3) * 18.0;
                let bumps = fbm(seed + 1, x / 20.0, z / 20.0, 2) * 1.2;
                let flat = ((Vec2::new(x, z).length() - 25.0) / 60.0).clamp(0.0, 1.0);
                // Raised rim so cars stay on the map.
                let edge = (x.abs().max(z.abs()) - (SIZE / 2.0 - 30.0)).max(0.0);
                heights.push((hills + bumps) * flat + edge * edge * 0.05);
            }
        }
        Self { heights }
    }

    fn vertex(&self, ix: usize, iz: usize) -> Vec3 {
        Vec3::new(
            ix as f32 * CELL - SIZE / 2.0,
            self.heights[iz * (CELLS + 1) + ix],
            iz as f32 * CELL - SIZE / 2.0,
        )
    }

    /// The triangle of the rendered mesh that contains (x, z), wound counter-clockwise from above.
    fn triangle(&self, x: f32, z: f32) -> [Vec3; 3] {
        let gx = ((x + SIZE / 2.0) / CELL).clamp(0.0, CELLS as f32 - 1e-3);
        let gz = ((z + SIZE / 2.0) / CELL).clamp(0.0, CELLS as f32 - 1e-3);
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

    /// Flat-shaded mesh: every triangle gets its own vertices, coloured by height and slope.
    fn mesh(&self) -> Mesh {
        let mut positions = Vec::new();
        let mut normals = Vec::new();
        let mut colors = Vec::new();
        for iz in 0..CELLS {
            for ix in 0..CELLS {
                let (a, b) = (self.vertex(ix, iz), self.vertex(ix + 1, iz));
                let (c, d) = (self.vertex(ix, iz + 1), self.vertex(ix + 1, iz + 1));
                for tri in [[a, d, b], [a, c, d]] {
                    let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize();
                    let height = (tri[0].y + tri[1].y + tri[2].y) / 3.0;
                    let color = ground_color(height, normal.y, ix * 31 + iz * 17);
                    for v in tri {
                        positions.push(v.to_array());
                        normals.push(normal.to_array());
                        colors.push(color);
                    }
                }
            }
        }
        let indices = (0..positions.len() as u32).collect();
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
        .with_inserted_indices(Indices::U32(indices))
    }
}

fn ground_color(height: f32, up: f32, variation: usize) -> [f32; 4] {
    let jitter = (hash(variation as u32, 99) - 0.5) * 0.06;
    let base = if up < 0.85 {
        Vec3::new(0.45, 0.40, 0.36) // steep: rock
    } else if height > 12.0 {
        Vec3::new(0.55, 0.60, 0.40) // high: dry grass
    } else {
        Vec3::new(0.33, 0.52, 0.28) // grass
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
}

/// Integer hash to [0, 1).
fn hash(n: u32, seed: u32) -> f32 {
    let mut h = n.wrapping_mul(0x27d4_eb2d) ^ seed.wrapping_mul(0x1656_67b1);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 13;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

/// Smoothly interpolated lattice noise in [-1, 1].
fn value_noise(seed: u32, x: f32, z: f32) -> f32 {
    let (x0, z0) = (x.floor(), z.floor());
    let (fx, fz) = (x - x0, z - z0);
    let (sx, sz) = (fx * fx * (3.0 - 2.0 * fx), fz * fz * (3.0 - 2.0 * fz));
    let corner = |dx: i32, dz: i32| {
        let (cx, cz) = (x0 as i32 + dx, z0 as i32 + dz);
        hash(
            (cx as u32).wrapping_mul(73_856_093) ^ (cz as u32).wrapping_mul(19_349_663),
            seed,
        )
    };
    let top = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * sx;
    let bottom = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * sx;
    (top + (bottom - top) * sz) * 2.0 - 1.0
}

fn fbm(seed: u32, x: f32, z: f32, octaves: u32) -> f32 {
    (0..octaves)
        .map(|i| {
            let f = (1 << i) as f32;
            value_noise(seed + i, x * f, z * f) / f
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raycast_matches_sampled_height() {
        let terrain = Terrain::generate(7);
        for &(x, z) in &[(0.0, 0.0), (90.3, -41.7), (-150.2, 133.9)] {
            let (h, _) = terrain.sample(x, z);
            let hit = terrain
                .raycast(Vec3::new(x, h + 1.0, z), Vec3::NEG_Y, 2.0)
                .unwrap();
            assert!((hit.distance - 1.0).abs() < 1e-3);
        }
    }
}
