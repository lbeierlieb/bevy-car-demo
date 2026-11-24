//! Low-poly houses from the OSM building outlines (visual only). Each outline is fitted
//! with a rectangle aligned to its longest edge and becomes a box with a gable roof
//! along its length; small sheds get a flat roof.

use bevy::prelude::*;

use crate::terrain::{FlatMesh, Terrain};

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
/// Window glass, unlit by day.
const GLASS: [f32; 4] = [0.05, 0.06, 0.08, 1.0];
/// Windows: width and height, sill above the storey's floor, spacing along a wall (m),
/// and how many in a hundred are lit at night.
const WINDOW: Vec2 = Vec2::new(1.0, 1.2);
const SILL: f32 = 0.9;
const WINDOW_SPACING: f32 = 3.2;
const LIT: u32 = 40;

/// The OSM buildings, indexed by the chunk their outline starts in.
pub struct Buildings {
    by_chunk: Vec<Vec<usize>>,
}

impl Buildings {
    pub fn new(terrain: &Terrain) -> Self {
        let mut by_chunk = vec![Vec::new(); terrain.chunk_count()];
        for (i, building) in terrain.buildings.iter().enumerate() {
            by_chunk[terrain.chunk_at(building.outline[0])].push(i);
        }
        Self { by_chunk }
    }

    /// The houses of one chunk, and their windows that are lit at night (the others are
    /// in `mesh`).
    pub fn mesh(&self, terrain: &Terrain, index: usize, mesh: &mut FlatMesh, lit: &mut FlatMesh) {
        for &i in &self.by_chunk[index] {
            let building = &terrain.buildings[i];
            house(mesh, lit, terrain, building.levels, &building.outline, i);
        }
    }
}

/// The rectangle a house is built on: aligned with the outline's longest edge, with `axis`
/// along its length.
pub struct Footprint {
    pub center: Vec2,
    pub axis: Vec2,
    pub length: f32,
    pub width: f32,
}

impl Footprint {
    /// The fitted rectangle, or `None` for outlines too small to become a house.
    pub fn fit(outline: &[Vec2]) -> Option<Self> {
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
        (length * width >= 6.0).then_some(Self {
            center,
            axis,
            length,
            width,
        })
    }

    pub fn area(&self) -> f32 {
        self.length * self.width
    }

    /// The four corners, going round.
    pub fn corners(&self) -> [Vec2; 4] {
        [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(a, p)| {
            self.center
                + self.axis * a * self.length / 2.0
                + self.axis.perp() * p * self.width / 2.0
        })
    }
}

/// A house's walls: the footprint, from the bottom to the top of the walls (y).
pub struct Shell {
    pub footprint: Footprint,
    pub bottom: f32,
    pub top: f32,
    pub storeys: f32,
}

impl Shell {
    /// The walls built on `outline`, or `None` for outlines too small to become a house.
    pub fn of(terrain: &Terrain, levels: f32, outline: &[Vec2]) -> Option<Self> {
        let footprint = Footprint::fit(outline)?;
        let storeys = if levels > 0.0 {
            levels
        } else if footprint.area() > 60.0 {
            2.0
        } else {
            1.0
        };
        let grounds = footprint.corners().map(|c| terrain.sample(c.x, c.y).0);
        // Sink into the lowest corner; full wall height above the highest one.
        let bottom = grounds.iter().copied().fold(f32::MAX, f32::min) - 0.3;
        let top = grounds.iter().copied().fold(f32::MIN, f32::max) + storeys * STOREY + 0.3;
        Some(Self {
            footprint,
            bottom,
            top,
            storeys,
        })
    }

    /// Height of the ridge above the walls (none on a shed's flat roof).
    pub fn roof_rise(&self) -> f32 {
        if self.footprint.area() < SHED_AREA {
            0.0
        } else {
            self.footprint.width / 2.0 * ROOF_PITCH
        }
    }
}

fn house(
    mesh: &mut FlatMesh,
    lit: &mut FlatMesh,
    terrain: &Terrain,
    levels: f32,
    outline: &[Vec2],
    index: usize,
) {
    let Some(shell) = Shell::of(terrain, levels, outline) else {
        return;
    };
    let Shell {
        footprint,
        bottom,
        top,
        storeys,
    } = &shell;
    let (bottom, top) = (*bottom, *top);
    let Footprint {
        center,
        axis,
        length,
        width: _,
    } = *footprint;
    let corners = footprint.corners();
    let at = |c: Vec2, y: f32| Vec3::new(c.x, y, c.y);

    // Windows: a row per storey along every wall long enough, standing 3 cm off it, some
    // lit at night (by a hash, so the same ones every time).
    if footprint.area() >= SHED_AREA {
        let floor = top - 0.3 - storeys * STOREY;
        for k in 0..4 {
            let (c0, c1) = (corners[k], corners[(k + 1) % 4]);
            let length = c0.distance(c1);
            let count = ((length - 1.0) / WINDOW_SPACING).floor().max(0.0) as u32;
            let along = (c1 - c0) / length;
            let out = ((c0 + c1) / 2.0 - center).normalize_or_zero();
            let half = along * WINDOW.x / 2.0;
            for storey in 0..*storeys as u32 {
                let sill = floor + storey as f32 * STOREY + SILL;
                for i in 0..count {
                    let c = c0 + along * length * (i as f32 + 0.5) / count as f32 + out * 0.03;
                    let [a, b, d, e] = [
                        at(c - half, sill),
                        at(c + half, sill),
                        at(c + half, sill + WINDOW.y),
                        at(c - half, sill + WINDOW.y),
                    ];
                    let seed = (index as u32)
                        .wrapping_mul(2_654_435_761)
                        .wrapping_add(k as u32 * 97 + storey * 31 + i)
                        .wrapping_mul(2_246_822_519);
                    let target = if (seed >> 7) % 100 < LIT {
                        &mut *lit
                    } else {
                        &mut *mesh
                    };
                    let outward = out.extend(0.0).xzy();
                    target.facing([a, b, d], GLASS, outward);
                    target.facing([a, d, e], GLASS, outward);
                }
            }
        }
    }

    let wall = WALLS[index % WALLS.len()];
    let middle = at(center, (bottom + top) / 2.0);
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
    if footprint.area() < SHED_AREA {
        quad([t0, t1, t2, t3], FLAT_ROOF);
        return;
    }
    // Gable roof: the ridge runs along the length, above the middle of each short end.
    let rise = shell.roof_rise();
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
