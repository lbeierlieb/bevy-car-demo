//! Low-poly trees scattered over the forest areas (and bushes over scrub); trunks are
//! obstacles (see `obstacles.rs`), bushes aren't.
//! Positions come from a jittered grid, kept clear of roads and streams. Trees are meshed
//! per terrain chunk, so a whole forest costs a few draw calls.

use std::f32::consts::TAU;

use bevy::prelude::*;

use crate::terrain::{FlatMesh, Terrain, hash};

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

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Bush,
    Spruce,
    Broadleaf,
}

/// One tree or bush: where it stands and how it looks.
pub struct Tree {
    pub ground: Vec3,
    pub kind: Kind,
    /// Scale (about 1).
    pub size: f32,
    /// Which colour, and how it's turned about its trunk.
    pick: usize,
    turn: f32,
}

impl Tree {
    /// The trunk's radius and the tree's height (m); a bush has no trunk.
    pub fn trunk(&self) -> Option<(f32, f32)> {
        match self.kind {
            Kind::Bush => None,
            Kind::Spruce => Some((0.25 * self.size, 15.0 * self.size)),
            Kind::Broadleaf => Some((0.3 * self.size, 10.0 * self.size)),
        }
    }
}

/// Tree positions: a jittered grid over forest and scrub, minus the roads.
pub struct Forest {
    mask: RoadMask,
}

impl Forest {
    pub fn new(terrain: &Terrain) -> Self {
        Self {
            mask: RoadMask::new(terrain),
        }
    }

    /// The trees and bushes of one chunk.
    pub fn trees(&self, terrain: &Terrain, index: usize) -> impl Iterator<Item = Tree> {
        self.positions(terrain, index).into_iter().map(|(p, i)| {
            let kind = match terrain.land_class(p.x, p.y) {
                SCRUB => Kind::Bush,
                _ if hash(i ^ 0x1f4a) < 0.6 => Kind::Spruce,
                _ => Kind::Broadleaf,
            };
            Tree {
                ground: Vec3::new(p.x, terrain.sample(p.x, p.y).0, p.y),
                kind,
                size: 0.7 + 0.6 * hash(i ^ 0x5bd1),
                pick: (hash(i ^ 0x2c1b) * 3.0) as usize % 3,
                turn: hash(i ^ 0x7e3f) * TAU,
            }
        })
    }

    /// The trees and bushes of one chunk as a mesh.
    pub fn mesh(&self, terrain: &Terrain, index: usize, mesh: &mut FlatMesh) {
        for tree in self.trees(terrain, index) {
            let Tree {
                ground,
                size,
                pick,
                turn,
                ..
            } = tree;
            match tree.kind {
                Kind::Bush => crown(
                    mesh,
                    ground,
                    0.0,
                    1.0 * size,
                    2.0 * size,
                    LEAVES[pick],
                    turn,
                ),
                Kind::Spruce => {
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
                Kind::Broadleaf => {
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
        }
    }

    /// Positions and seeds of the trees and bushes whose grid cell lies in a chunk.
    fn positions(&self, terrain: &Terrain, index: usize) -> Vec<(Vec2, u32)> {
        let half_size = terrain.half_size();
        let n = (2.0 * half_size / SPACING).as_uvec2();
        let (lo, hi) = terrain.chunk_bounds(index);
        // Grid cells whose corner lies in [lo, hi).
        let range = |lo: f32, hi: f32, half: f32, n: u32| {
            let first = ((lo + half) / SPACING).ceil() as u32;
            let end = (((hi + half) / SPACING).ceil() as u32).min(n);
            first..end
        };
        let mut out = Vec::new();
        for gz in range(lo.y, hi.y, half_size.y, n.y) {
            for gx in range(lo.x, hi.x, half_size.x, n.x) {
                let i = gz * n.x + gx;
                let jitter = Vec2::new(hash(i ^ 0x3a7c), hash(i ^ 0x6d2e));
                let p = (Vec2::new(gx as f32, gz as f32) + jitter) * SPACING - half_size;
                if !terrain.inside(p.x, p.y) || self.mask.blocked(p) {
                    continue;
                }
                // Bushes over scrub are sparser than trees in forest.
                let keep = match terrain.land_class(p.x, p.y) {
                    FOREST => true,
                    SCRUB => hash(i ^ 0x4b9d) < 0.35,
                    _ => false,
                };
                if keep {
                    out.push((p, i));
                }
            }
        }
        out
    }
}

/// Cells within `CLEARANCE` of a road or stream.
struct RoadMask {
    half_size: Vec2,
    size: UVec2,
    blocked: Vec<bool>,
}

impl RoadMask {
    fn new(terrain: &Terrain) -> Self {
        let half_size = terrain.half_size();
        let size = (2.0 * half_size / MASK_CELL).ceil().as_uvec2();
        let mut mask = Self {
            half_size,
            size,
            blocked: vec![false; (size.x * size.y) as usize],
        };
        for road in &terrain.roads {
            let reach = road.width / 2.0 + CLEARANCE;
            for pair in road.points.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let (lo, hi) = (mask.cell(a.min(b) - reach), mask.cell(a.max(b) + reach));
                for z in lo.y..=hi.y {
                    for x in lo.x..=hi.x {
                        let p = (UVec2::new(x, z).as_vec2() + 0.5) * MASK_CELL - half_size;
                        let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-6))
                            .clamp(0.0, 1.0);
                        // Cell centre test, widened by half a cell diagonal.
                        if p.distance(a + (b - a) * t) < reach + MASK_CELL * 0.71 {
                            mask.blocked[(z * size.x + x) as usize] = true;
                        }
                    }
                }
            }
        }
        mask
    }

    fn cell(&self, p: Vec2) -> UVec2 {
        ((p + self.half_size) / MASK_CELL)
            .max(Vec2::ZERO)
            .as_uvec2()
            .min(self.size - 1)
    }

    fn blocked(&self, p: Vec2) -> bool {
        let c = self.cell(p);
        self.blocked[(c.y * self.size.x + c.x) as usize]
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
        let forest = Forest::new(&terrain);
        let trees: Vec<Vec2> = (0..terrain.chunk_count())
            .flat_map(|index| forest.positions(&terrain, index))
            .map(|(p, _)| p)
            .collect();
        println!("{} trees and bushes", trees.len());
        assert!(trees.len() > 1000);
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
