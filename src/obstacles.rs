//! The map's houses, tree trunks and bridge parapets as boxes for collisions, built once
//! at startup into the terrain's grid of obstacles.

use bevy::prelude::*;

use crate::buildings::Shell;
use crate::physics::collision::{Obstacles, Upright};
use crate::terrain::{DECK_DEPTH, PARAPET_HEIGHT, PARAPET_WIDTH, RoadKind, Terrain};
use crate::trees::Forest;

/// Grid cell (m): about the trees' spacing, so a cell holds one or two.
const CELL: f32 = 8.0;

pub struct ObstaclesPlugin;

impl Plugin for ObstaclesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, |mut terrain: ResMut<Terrain>| {
            terrain.obstacles = build(&terrain);
        });
    }
}

/// Every house (walls up to the ridge), every tree trunk (a square as wide as it), and
/// the parapets along bridges (in pieces a couple of metres long, down to the slab's
/// bottom, so nothing slips under them).
pub fn build(terrain: &Terrain) -> Obstacles {
    let mut boxes = Vec::new();
    for building in &terrain.buildings {
        if let Some(shell) = Shell::of(terrain, building.levels, &building.outline) {
            let f = &shell.footprint;
            boxes.push(Upright {
                center: f.center,
                half: Vec2::new(f.length, f.width) / 2.0,
                bottom: shell.bottom,
                top: shell.top + shell.roof_rise(),
                angle: f.axis.to_angle(),
            });
        }
    }
    let forest = Forest::new(terrain);
    for chunk in 0..terrain.chunk_count() {
        for tree in forest.trees(terrain, chunk) {
            if let Some((radius, height)) = tree.trunk() {
                boxes.push(Upright {
                    center: tree.ground.xz(),
                    half: Vec2::splat(radius),
                    bottom: tree.ground.y - 0.5,
                    top: tree.ground.y + height,
                    angle: 0.0,
                });
            }
        }
    }
    for road in terrain
        .roads
        .iter()
        .filter(|r| r.bridge && r.kind != RoadKind::Water)
    {
        let edge = |i: usize| road.heights[i] - road.crown * road.width / 2.0;
        let n = road.points.len();
        for i in (0..n - 1).step_by(2) {
            let j = (i + 2).min(n - 1);
            let (a, b) = (road.points[i], road.points[j]);
            let axis = (b - a).normalize_or_zero();
            let (low, high) = (edge(i).min(edge(j)), edge(i).max(edge(j)));
            for out in [axis.perp(), -axis.perp()] {
                boxes.push(Upright {
                    center: (a + b) / 2.0 + out * (road.width + PARAPET_WIDTH) / 2.0,
                    half: Vec2::new(a.distance(b) / 2.0, PARAPET_WIDTH / 2.0),
                    bottom: low - DECK_DEPTH,
                    top: high + PARAPET_HEIGHT,
                    angle: axis.to_angle(),
                });
            }
        }
    }
    Obstacles::new(boxes, CELL)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn houses_and_trees_become_obstacles() {
        let terrain = Terrain::baked();
        let start = std::time::Instant::now();
        let obstacles = build(&terrain);
        let houses = terrain.buildings.len();
        println!(
            "{} obstacles ({houses} buildings) in {:?}",
            obstacles.boxes.len(),
            start.elapsed()
        );
        assert!(obstacles.boxes.len() > houses);
    }
}
