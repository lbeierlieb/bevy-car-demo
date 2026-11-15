//! Low-poly trees scattered over the forest areas (and bushes over scrub), visual only.
//! Positions come from a jittered grid, kept clear of roads and streams. All trees are
//! merged into the terrain's chunk meshes, so a whole forest costs a few draw calls.

use std::f32::consts::TAU;

use bevy::prelude::*;

use crate::terrain::{ChunkedMesh, FlatMesh, Terrain, hash};

/// Land use classes (as in `tools/bake`).
const SCRUB: u8 = 7;
const FOREST: u8 = 8;
/// Grid spacing for tree candidates (m); each is jittered within its grid cell.
const SPACING: f32 = 7.0;
/// Space kept free beside road edges (m).
const CLEARANCE: f32 = 1.5;
/// Raster resolution of the road mask (m).
const MASK_CELL: f32 = 2.0;

const TRUNK: [f32; 4] = [0.30, 0.22, 0.15, 1.0];
const NEEDLES: [[f32; 4]; 3] = [
    [0.10, 0.24, 0.13, 1.0],
    [0.12, 0.28, 0.14, 1.0],
    [0.09, 0.21, 0.12, 1.0],
];
const LEAVES: [[f32; 4]; 3] = [
    [0.22, 0.38, 0.14, 1.0],
    [0.27, 0.42, 0.16, 1.0],
    [0.20, 0.33, 0.12, 1.0],
];

pub struct TreesPlugin;

impl Plugin for TreesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_trees);
    }
}

fn spawn_trees(
    mut commands: Commands,
    terrain: Res<Terrain>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut chunks = ChunkedMesh::new(&terrain);
    let count = scatter(&terrain, |p, i| {
        let ground = Vec3::new(p.x, terrain.sample(p.x, p.y).0, p.y);
        let mesh = chunks.at(ground);
        let size = 0.7 + 0.6 * hash(i ^ 0x5bd1);
        let pick = (hash(i ^ 0x2c1b) * 3.0) as usize % 3;
        let turn = hash(i ^ 0x7e3f) * TAU;
        match terrain.land_class(p.x, p.y) {
            SCRUB => crown(
                mesh,
                ground,
                0.0,
                1.0 * size,
                2.0 * size,
                LEAVES[pick],
                turn,
            ),
            _ if hash(i ^ 0x1f4a) < 0.6 => {
                // Spruce: a short trunk and one long cone.
                trunk(mesh, ground, 2.0 * size, 0.25 * size, turn);
                cone(
                    mesh,
                    ground,
                    1.5 * size,
                    2.4 * size,
                    15.0 * size,
                    NEEDLES[pick],
                    turn,
                );
            }
            _ => {
                // Broadleaf: a taller trunk and a rounded crown.
                trunk(mesh, ground, 5.0 * size, 0.3 * size, turn);
                crown(
                    mesh,
                    ground,
                    4.0 * size,
                    2.6 * size,
                    10.0 * size,
                    LEAVES[pick],
                    turn,
                );
            }
        }
    });
    info!("{count} trees and bushes");
    let material = materials.add(StandardMaterial {
        perceptual_roughness: 0.9,
        ..default()
    });
    chunks.spawn(&mut commands, &mut meshes, material, true);
}

/// Calls `plant` with the position and a seed for every tree or bush, returning the count.
fn scatter(terrain: &Terrain, mut plant: impl FnMut(Vec2, u32)) -> usize {
    let mask = RoadMask::new(terrain);
    let n = (2.0 * mask.half_size / SPACING) as i32;
    let mut count = 0;
    for gz in 0..n {
        for gx in 0..n {
            let i = (gz * n + gx) as u32;
            let jitter = Vec2::new(hash(i ^ 0x3a7c), hash(i ^ 0x6d2e));
            let p = (Vec2::new(gx as f32, gz as f32) + jitter) * SPACING - mask.half_size;
            if !terrain.inside(p.x, p.y) || mask.blocked(p) {
                continue;
            }
            // Bushes over scrub are sparser than trees in forest.
            let keep = match terrain.land_class(p.x, p.y) {
                FOREST => true,
                SCRUB => hash(i ^ 0x4b9d) < 0.35,
                _ => false,
            };
            if keep {
                plant(p, i);
                count += 1;
            }
        }
    }
    count
}

/// Cells within `CLEARANCE` of a road or stream.
struct RoadMask {
    half_size: f32,
    side: usize,
    blocked: Vec<bool>,
}

impl RoadMask {
    fn new(terrain: &Terrain) -> Self {
        let half_size = terrain.half_size();
        let side = (2.0 * half_size / MASK_CELL).ceil() as usize;
        let mut blocked = vec![false; side * side];
        let cell = |v: f32| ((v + half_size) / MASK_CELL).clamp(0.0, (side - 1) as f32) as usize;
        for road in &terrain.roads {
            let reach = road.width / 2.0 + CLEARANCE;
            for pair in road.points.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let (lo, hi) = (a.min(b) - reach, a.max(b) + reach);
                for z in cell(lo.y)..=cell(hi.y) {
                    for x in cell(lo.x)..=cell(hi.x) {
                        let p = (Vec2::new(x as f32, z as f32) + 0.5) * MASK_CELL - half_size;
                        let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-6))
                            .clamp(0.0, 1.0);
                        // Cell centre test, widened by half a cell diagonal.
                        if p.distance(a + (b - a) * t) < reach + MASK_CELL * 0.71 {
                            blocked[z * side + x] = true;
                        }
                    }
                }
            }
        }
        Self {
            half_size,
            side,
            blocked,
        }
    }

    fn blocked(&self, p: Vec2) -> bool {
        let cell =
            |v: f32| ((v + self.half_size) / MASK_CELL).clamp(0.0, (self.side - 1) as f32) as usize;
        self.blocked[cell(p.y) * self.side + cell(p.x)]
    }
}

/// Ring of `SIDES` points around `centre` at height `y` above it.
const SIDES: usize = 5;

fn ring(centre: Vec3, y: f32, radius: f32, turn: f32) -> [Vec3; SIDES] {
    std::array::from_fn(|k| {
        let a = turn + k as f32 * TAU / SIDES as f32;
        centre + Vec3::new(radius * a.cos(), y, radius * a.sin())
    })
}

/// Three-sided trunk, sunk a little into the ground for slopes.
fn trunk(mesh: &mut FlatMesh, ground: Vec3, height: f32, radius: f32, turn: f32) {
    let corner = |k: usize, y: f32| {
        let a = turn + k as f32 * TAU / 3.0;
        ground + Vec3::new(radius * a.cos(), y, radius * a.sin())
    };
    for k in 0..3 {
        let (b0, b1) = (corner(k, -0.5), corner(k + 1, -0.5));
        let (t0, t1) = (corner(k, height), corner(k + 1, height));
        let outward = (b0 + b1) / 2.0 - ground;
        mesh.facing([b0, b1, t1], TRUNK, outward);
        mesh.facing([b0, t1, t0], TRUNK, outward);
    }
}

/// Open cone from a ring at `base` height up to an apex at `top`.
fn cone(
    mesh: &mut FlatMesh,
    ground: Vec3,
    base: f32,
    radius: f32,
    top: f32,
    color: [f32; 4],
    turn: f32,
) {
    let rim = ring(ground, base, radius, turn);
    let apex = ground + Vec3::Y * top;
    for k in 0..SIDES {
        let (r0, r1) = (rim[k], rim[(k + 1) % SIDES]);
        let mid = (r0 + r1 + apex) / 3.0;
        mesh.facing([r0, r1, apex], color, mid - ground.with_y(mid.y));
    }
}

/// Rounded crown between `base` and `top`: two cones meeting at a ring at 40 % height.
fn crown(
    mesh: &mut FlatMesh,
    ground: Vec3,
    base: f32,
    radius: f32,
    top: f32,
    color: [f32; 4],
    turn: f32,
) {
    let waist = base + (top - base) * 0.4;
    let rim = ring(ground, waist, radius, turn);
    let (low, high) = (ground + Vec3::Y * base, ground + Vec3::Y * top);
    let centre = ground + Vec3::Y * waist;
    for k in 0..SIDES {
        let (r0, r1) = (rim[k], rim[(k + 1) % SIDES]);
        mesh.facing([r0, r1, high], color, (r0 + r1 + high) / 3.0 - centre);
        mesh.facing([r0, r1, low], color, (r0 + r1 + low) / 3.0 - centre);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trees_grow_in_forest_and_keep_off_roads() {
        let terrain = Terrain::baked();
        let mut trees = Vec::new();
        let count = scatter(&terrain, |p, _| trees.push(p));
        println!("{count} trees and bushes");
        assert!(count > 1000);
        for p in trees {
            assert!(matches!(terrain.land_class(p.x, p.y), SCRUB | FOREST));
            for road in &terrain.roads {
                for pair in road.points.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    let t =
                        ((p - a).dot(b - a) / (b - a).length_squared().max(1e-6)).clamp(0.0, 1.0);
                    assert!(p.distance(a + (b - a) * t) > road.width / 2.0 + CLEARANCE);
                }
            }
        }
    }
}
