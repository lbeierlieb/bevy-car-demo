//! Low-poly houses from the OSM building outlines (visual only). Each outline is fitted
//! with a rectangle aligned to its longest edge and becomes a box with a gable roof
//! along its length; small sheds get a flat roof.

use bevy::prelude::*;

use crate::terrain::{ChunkedMesh, Terrain};

const STOREY: f32 = 2.8;
/// Roof rise per metre of half-width (about 39°).
const ROOF_PITCH: f32 = 0.8;
/// Footprints smaller than this (m²) get a flat roof.
const SHED_AREA: f32 = 20.0;

const WALLS: [[f32; 4]; 4] = [
    [0.92, 0.89, 0.82, 1.0],
    [0.96, 0.95, 0.92, 1.0],
    [0.88, 0.80, 0.64, 1.0],
    [0.80, 0.80, 0.78, 1.0],
];
const ROOFS: [[f32; 4]; 4] = [
    [0.62, 0.22, 0.16, 1.0],
    [0.50, 0.17, 0.13, 1.0],
    [0.45, 0.30, 0.22, 1.0],
    [0.25, 0.25, 0.28, 1.0],
];
const FLAT_ROOF: [f32; 4] = [0.42, 0.42, 0.44, 1.0];

pub struct BuildingsPlugin;

impl Plugin for BuildingsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_buildings);
    }
}

fn spawn_buildings(
    mut commands: Commands,
    terrain: Res<Terrain>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut chunks = ChunkedMesh::new(&terrain);
    for (i, building) in terrain.buildings.iter().enumerate() {
        house(&mut chunks, &terrain, building.levels, &building.outline, i);
    }
    let material = materials.add(StandardMaterial {
        perceptual_roughness: 0.85,
        ..default()
    });
    chunks.spawn(&mut commands, &mut meshes, material, true);
}

fn house(chunks: &mut ChunkedMesh, terrain: &Terrain, levels: f32, outline: &[Vec2], index: usize) {
    // Rectangle aligned with the outline's longest edge, with `axis` along its length.
    let longest = (0..outline.len())
        .map(|i| outline[(i + 1) % outline.len()] - outline[i])
        .max_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
        .unwrap_or(Vec2::X);
    let mut axis = longest.normalize_or(Vec2::X);
    let extent = |dir: Vec2| {
        let dots = outline.iter().map(|p| p.dot(dir));
        let (lo, hi) = dots.fold((f32::MAX, f32::MIN), |(lo, hi), d| (lo.min(d), hi.max(d)));
        (lo, hi)
    };
    let ((a0, a1), (p0, p1)) = (extent(axis), extent(axis.perp()));
    let (mut length, mut width) = (a1 - a0, p1 - p0);
    let center = axis * (a0 + a1) / 2.0 + axis.perp() * (p0 + p1) / 2.0;
    if width > length {
        (length, width, axis) = (width, length, axis.perp());
    }
    let area = length * width;
    if area < 6.0 {
        return;
    }

    let storeys = if levels > 0.0 {
        levels
    } else if area > 60.0 {
        2.0
    } else {
        1.0
    };
    let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
        .map(|(a, p)| center + axis * a * length / 2.0 + axis.perp() * p * width / 2.0);
    let grounds = corners.map(|c| terrain.sample(c.x, c.y).0);
    // Sink into the lowest corner; full wall height above the highest one.
    let bottom = grounds.iter().copied().fold(f32::MAX, f32::min) - 0.3;
    let top = grounds.iter().copied().fold(f32::MIN, f32::max) + storeys * STOREY + 0.3;
    let at = |c: Vec2, y: f32| Vec3::new(c.x, y, c.y);

    let wall = WALLS[index % WALLS.len()];
    let middle = at(center, (bottom + top) / 2.0);
    let mesh = chunks.at(middle);
    let mut quad = |q: [Vec3; 4], color: [f32; 4]| {
        let outward = (q[0] + q[1] + q[2] + q[3]) / 4.0 - middle;
        mesh.facing([q[0], q[1], q[2]], color, outward);
        mesh.facing([q[0], q[2], q[3]], color, outward);
    };
    for k in 0..4 {
        let (c0, c1) = (corners[k], corners[(k + 1) % 4]);
        quad(
            [at(c0, bottom), at(c1, bottom), at(c1, top), at(c0, top)],
            wall,
        );
    }

    let [t0, t1, t2, t3] = corners.map(|c| at(c, top));
    if area < SHED_AREA {
        quad([t0, t1, t2, t3], FLAT_ROOF);
        return;
    }
    // Gable roof: the ridge runs along the length, above the middle of each short end.
    let rise = width / 2.0 * ROOF_PITCH;
    let r0 = at(center - axis * length / 2.0, top + rise);
    let r1 = at(center + axis * length / 2.0, top + rise);
    let roof = ROOFS[(index / WALLS.len()) % ROOFS.len()];
    quad([t0, t1, r1, r0], roof);
    quad([t2, t3, r0, r1], roof);
    // Gable ends, in wall colour, facing out along the axis.
    let end = axis.extend(0.0).xzy();
    mesh.facing([t1, t2, r1], wall, end);
    mesh.facing([t3, t0, r0], wall, -end);
}
