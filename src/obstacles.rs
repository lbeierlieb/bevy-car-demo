//! The map's houses and tree trunks as boxes for collisions, built once at startup into
//! the terrain's grid of obstacles.

use bevy::prelude::*;

use crate::buildings::Shell;
use crate::physics::collision::{Obstacles, Upright};
use crate::terrain::Terrain;
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

/// Every house (walls up to the ridge) and every tree trunk (a square as wide as it).
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
