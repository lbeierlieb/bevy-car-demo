//! Fences, walls, hedges and guard rails from the map's barriers (gates are gaps, cut out
//! by the bake). Drawn in pieces with the other objects of their chunk, and solid: each
//! piece is a box in the obstacle grid (`obstacles.rs`).

use bevy::prelude::*;

use crate::terrain::{FlatMesh, Terrain};

/// Longest piece (m): the ground is sampled at both ends, and the bigger posts stand this
/// far apart at most.
const PIECE: f32 = 2.5;
/// Collision boxes are at least this thick (m), so a fast car can't slip through.
const SOLID: f32 = 0.3;
/// Walls, hedges and panels reach this far into the ground (m), so slopes show no gap.
const FOOTING: f32 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FenceKind {
    /// Wooden posts and two rails.
    Rails,
    /// Thin metal posts and rails (railings, mesh and wire fences).
    Metal,
    /// Closed boards or panels.
    Panel,
    Wall,
    Hedge,
    GuardRail,
}

impl FenceKind {
    pub fn from_byte(byte: u8) -> Self {
        [
            Self::Rails,
            Self::Metal,
            Self::Panel,
            Self::Wall,
            Self::Hedge,
            Self::GuardRail,
        ][byte as usize]
    }

    /// How thick it is (m).
    fn thickness(self) -> f32 {
        match self {
            Self::Rails => 0.12,
            Self::Metal => 0.06,
            Self::Panel => 0.1,
            Self::Wall => 0.3,
            Self::Hedge => 0.9,
            Self::GuardRail => 0.12,
        }
    }
}

pub struct Fence {
    pub kind: FenceKind,
    /// Height above the ground (m).
    pub height: f32,
    pub points: Vec<Vec2>,
}

/// A straight piece of a fence, with a post at its start (and at its end if it's the
/// last).
#[derive(Clone, Copy)]
pub struct Piece {
    pub fence: usize,
    pub a: Vec2,
    pub b: Vec2,
    pub last: bool,
}

/// A fence in pieces at most `PIECE` long (the guard rail's posts 2 m apart).
pub fn pieces(fence: &Fence, index: usize) -> Vec<Piece> {
    let longest = if fence.kind == FenceKind::GuardRail {
        2.0
    } else {
        PIECE
    };
    let mut out = Vec::new();
    for w in fence.points.windows(2) {
        let steps = (w[0].distance(w[1]) / longest).ceil().max(1.0) as usize;
        for k in 0..steps {
            out.push(Piece {
                fence: index,
                a: w[0].lerp(w[1], k as f32 / steps as f32),
                b: w[0].lerp(w[1], (k + 1) as f32 / steps as f32),
                last: false,
            });
        }
    }
    if let Some(last) = out.last_mut() {
        last.last = true;
    }
    out
}

/// A fence piece's box for collisions: from below the ground to its top, at least
/// `SOLID` thick.
pub fn solid(
    terrain: &Terrain,
    fence: &Fence,
    piece: &Piece,
) -> crate::physics::collision::Upright {
    let (ya, yb) = (ground(terrain, piece.a), ground(terrain, piece.b));
    let axis = (piece.b - piece.a).normalize_or(Vec2::X);
    crate::physics::collision::Upright {
        center: (piece.a + piece.b) / 2.0,
        half: Vec2::new(
            piece.a.distance(piece.b) / 2.0,
            fence.kind.thickness().max(SOLID) / 2.0,
        ),
        bottom: ya.min(yb) - 0.5,
        top: ya.max(yb) + fence.height,
        angle: axis.to_angle(),
    }
}

/// The ground a fence stands on: beneath any bridge, the road where there is one.
fn ground(terrain: &Terrain, p: Vec2) -> f32 {
    terrain.sample(p.x, p.y).0
}

const WOOD: [f32; 4] = [0.42, 0.31, 0.20, 1.0];
const BOARDS: [f32; 4] = [0.50, 0.38, 0.25, 1.0];
const METAL: [f32; 4] = [0.50, 0.52, 0.53, 1.0];
const STEEL: [f32; 4] = [0.72, 0.74, 0.76, 1.0];
const STONE: [f32; 4] = [0.66, 0.63, 0.57, 1.0];
const LEAVES: [f32; 4] = [0.22, 0.36, 0.17, 1.0];

/// The fences, in pieces by the chunk each piece's middle is in.
pub struct Fences {
    by_chunk: Vec<Vec<Piece>>,
}

impl Fences {
    pub fn new(terrain: &Terrain) -> Self {
        let mut by_chunk = vec![Vec::new(); terrain.chunk_count()];
        for (i, fence) in terrain.fences.iter().enumerate() {
            for piece in pieces(fence, i) {
                by_chunk[terrain.chunk_at((piece.a + piece.b) / 2.0)].push(piece);
            }
        }
        Self { by_chunk }
    }

    /// The fence pieces of one chunk.
    pub fn mesh(&self, terrain: &Terrain, index: usize, mesh: &mut FlatMesh) {
        for piece in &self.by_chunk[index] {
            let fence = &terrain.fences[piece.fence];
            let (a, b) = (
                piece.a.extend(ground(terrain, piece.a)).xzy(),
                piece.b.extend(ground(terrain, piece.b)).xzy(),
            );
            draw(mesh, fence.kind, fence.height, a, b, piece.last);
        }
    }
}

/// One piece from ground point `a` to `b`.
fn draw(mesh: &mut FlatMesh, kind: FenceKind, h: f32, a: Vec3, b: Vec3, last: bool) {
    let posts = |mesh: &mut FlatMesh, side: f32, height: f32, color| {
        post(mesh, a, side, height, color);
        if last {
            post(mesh, b, side, height, color);
        }
    };
    match kind {
        FenceKind::Rails => {
            posts(mesh, 0.12, h, WOOD);
            for low in [0.35 * h, 0.8 * h] {
                slab(mesh, a, b, low, low + 0.1, 0.05, WOOD);
            }
        }
        FenceKind::Metal => {
            posts(mesh, 0.06, h, METAL);
            for low in [0.1, 0.5 * h, h - 0.04] {
                slab(mesh, a, b, low, low + 0.04, 0.03, METAL);
            }
        }
        FenceKind::Panel => {
            posts(mesh, 0.1, h, WOOD);
            slab(mesh, a, b, -FOOTING, h, 0.04, BOARDS);
        }
        FenceKind::Wall => slab(mesh, a, b, -FOOTING, h, kind.thickness(), STONE),
        FenceKind::Hedge => slab(mesh, a, b, -FOOTING, h, kind.thickness(), LEAVES),
        FenceKind::GuardRail => {
            posts(mesh, 0.1, h - 0.05, STEEL);
            slab(mesh, a, b, h - 0.33, h, 0.06, STEEL);
        }
    }
}

/// A box along `a` to `b` (on the ground), from `low` to `high` above it (following the
/// slope) and `thickness` across, with end faces; no bottom.
fn slab(
    mesh: &mut FlatMesh,
    a: Vec3,
    b: Vec3,
    low: f32,
    high: f32,
    thickness: f32,
    color: [f32; 4],
) {
    let along = (b - a).with_y(0.0).normalize_or(Vec3::X);
    let across = Vec3::new(-along.z, 0.0, along.x) * thickness / 2.0;
    let up = Vec3::Y;
    // Corners: near/far end, left/right side, bottom/top.
    let c = |end: Vec3, side: f32, height: f32| end + across * side + up * height;
    let mut quad = |p: [Vec3; 4], outward: Vec3| {
        mesh.facing([p[0], p[1], p[2]], color, outward);
        mesh.facing([p[1], p[3], p[2]], color, outward);
    };
    for side in [-1.0, 1.0] {
        quad(
            [
                c(a, side, low),
                c(a, side, high),
                c(b, side, low),
                c(b, side, high),
            ],
            across * side,
        );
    }
    quad(
        [
            c(a, -1.0, high),
            c(a, 1.0, high),
            c(b, -1.0, high),
            c(b, 1.0, high),
        ],
        up,
    );
    for (end, outward) in [(a, -along), (b, along)] {
        quad(
            [
                c(end, -1.0, low),
                c(end, 1.0, low),
                c(end, -1.0, high),
                c(end, 1.0, high),
            ],
            outward,
        );
    }
}

/// An upright square post `side` wide standing on `base`, `height` tall.
fn post(mesh: &mut FlatMesh, base: Vec3, side: f32, height: f32, color: [f32; 4]) {
    let half = side / 2.0;
    slab(
        mesh,
        base - Vec3::X * half,
        base + Vec3::X * half,
        -0.2,
        height,
        side,
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fences_come_in_short_pieces_with_a_post_at_the_end() {
        let fence = Fence {
            kind: FenceKind::Rails,
            height: 1.2,
            points: vec![Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 1.0)],
        };
        let pieces = pieces(&fence, 0);
        assert_eq!(pieces.len(), 5);
        assert!(pieces.iter().all(|p| p.a.distance(p.b) <= PIECE + 1e-5));
        assert_eq!(pieces.iter().filter(|p| p.last).count(), 1);
        assert_eq!(pieces[4].b, Vec2::new(10.0, 1.0));
    }
}
