//! What the map file's ways and nodes are (`Class`, `PointKind`) and a grid of where they
//! are, rebuilt after every change, for drawing and picking near the camera.

use std::collections::HashSet;

use bevy::prelude::*;

use super::doc::Doc;
use crate::osm::Tags;

/// Grid cell side (m).
const CELL: f32 = 64.0;

pub struct IndexPlugin;

impl Plugin for IndexPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, |mut commands: Commands| {
            commands.insert_resource(Index::default())
        })
        .add_systems(PreUpdate, rebuild);
    }
}

/// What a way is, by its tags (the bake's reading of them, roughly).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    Road,
    Water,
    Building,
    Fence,
    /// Land use: paved areas, forest, fields, meadows, water…
    Area,
    /// Our own terrain edits (`carsim:terrain`).
    Terrain,
    /// Anything else (untagged multipolygon members, power lines…).
    Other,
}

impl Class {
    pub const ALL: [Class; 7] = [
        Class::Road,
        Class::Water,
        Class::Building,
        Class::Fence,
        Class::Area,
        Class::Terrain,
        Class::Other,
    ];

    pub fn of(tags: &Tags) -> Self {
        let has = |k: &str| tags.contains_key(k);
        if has(TERRAIN) {
            Class::Terrain
        } else if has("building") {
            Class::Building
        } else if has("barrier") {
            Class::Fence
        } else if has("highway") && tags.get("area").is_none_or(|a| a != "yes") {
            Class::Road
        } else if has("waterway") {
            Class::Water
        } else if ["landuse", "natural", "leisure", "area:highway"]
            .iter()
            .any(|k| has(k))
            || tags.get("amenity").is_some_and(|a| a == "parking")
            || has("highway")
        {
            Class::Area
        } else {
            Class::Other
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Class::Road => "Roads",
            Class::Water => "Waterways",
            Class::Building => "Buildings",
            Class::Fence => "Fences",
            Class::Area => "Land use",
            Class::Terrain => "Terrain areas",
            Class::Other => "Other",
        }
    }

    pub fn color(self) -> Color {
        match self {
            Class::Road => Color::srgb(1.0, 0.85, 0.2),
            Class::Water => Color::srgb(0.3, 0.6, 1.0),
            Class::Building => Color::srgb(1.0, 0.45, 0.35),
            Class::Fence => Color::srgb(0.85, 0.55, 1.0),
            Class::Area => Color::srgb(0.45, 0.95, 0.45),
            Class::Terrain => Color::srgb(1.0, 0.55, 0.1),
            Class::Other => Color::srgb(0.6, 0.6, 0.6),
        }
    }
}

/// The tag that makes a closed way a terrain area, and its operations (see the bake's
/// `terrain.rs`).
pub const TERRAIN: &str = "carsim:terrain";

/// Nodes that are something on their own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointKind {
    Bump,
    Hump,
    Table,
    Pothole,
    Gate,
    /// A road height pin (`ele`), or a terrain area's corner height.
    Height,
    Other,
}

impl PointKind {
    pub fn of(tags: &Tags) -> Option<Self> {
        let tag = |k: &str| tags.get(k).map(String::as_str);
        Some(
            match (tag("traffic_calming"), tag("hazard"), tag("barrier")) {
                (Some("bump"), ..) => PointKind::Bump,
                (Some("hump"), ..) => PointKind::Hump,
                (Some("table"), ..) => PointKind::Table,
                (_, Some("pothole"), _) => PointKind::Pothole,
                (_, _, Some(_)) => PointKind::Gate,
                _ if tags.contains_key("ele") => PointKind::Height,
                _ if tags.contains_key("traffic_calming") => PointKind::Other,
                _ => return None,
            },
        )
    }

    pub fn color(self) -> Color {
        match self {
            PointKind::Bump | PointKind::Hump | PointKind::Table => Color::srgb(1.0, 0.95, 0.3),
            PointKind::Pothole => Color::srgb(0.9, 0.35, 0.2),
            PointKind::Gate => Color::srgb(0.85, 0.55, 1.0),
            PointKind::Height => Color::srgb(0.3, 1.0, 1.0),
            PointKind::Other => Color::srgb(0.8, 0.8, 0.8),
        }
    }
}

/// The ways and point nodes by grid cell.
#[derive(Resource, Default)]
pub struct Index {
    revision: Option<u64>,
    origin: Vec2,
    cells_x: usize,
    cells_z: usize,
    ways: Vec<Vec<i64>>,
    points: Vec<Vec<i64>>,
}

impl Index {
    fn cell(&self, p: Vec2) -> (usize, usize) {
        let c = ((p - self.origin) / CELL).floor();
        (
            (c.x.max(0.0) as usize).min(self.cells_x - 1),
            (c.y.max(0.0) as usize).min(self.cells_z - 1),
        )
    }

    fn cells_around(&self, p: Vec2, radius: f32) -> impl Iterator<Item = usize> + '_ {
        let (lo, hi) = (self.cell(p - radius), self.cell(p + radius));
        (lo.1..=hi.1).flat_map(move |z| (lo.0..=hi.0).map(move |x| z * self.cells_x + x))
    }

    /// The ways with a segment in a cell within `radius` of `p`.
    pub fn ways_near(&self, p: Vec2, radius: f32) -> Vec<i64> {
        if self.cells_x == 0 {
            return Vec::new();
        }
        let mut seen = HashSet::new();
        self.cells_around(p, radius)
            .flat_map(|c| self.ways[c].iter().copied())
            .filter(|w| seen.insert(*w))
            .collect()
    }

    /// The point nodes (see `PointKind`) in cells within `radius` of `p`.
    pub fn points_near(&self, p: Vec2, radius: f32) -> Vec<i64> {
        if self.cells_x == 0 {
            return Vec::new();
        }
        self.cells_around(p, radius)
            .flat_map(|c| self.points[c].iter().copied())
            .collect()
    }
}

fn rebuild(doc: Option<Res<Doc>>, mut index: ResMut<Index>) {
    let Some(doc) = doc else {
        return;
    };
    if index.revision == Some(doc.revision) {
        return;
    }
    let started = std::time::Instant::now();
    // The grid covers every node, wherever it is.
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for id in doc.osm.nodes.keys() {
        if let Some(p) = doc.pos(*id) {
            lo = lo.min(p);
            hi = hi.max(p);
        }
    }
    if lo.x > hi.x {
        (lo, hi) = (Vec2::ZERO, Vec2::ONE);
    }
    let cells = ((hi - lo) / CELL).floor().as_uvec2() + 1;
    let (cells_x, cells_z) = (cells.x as usize, cells.y as usize);
    let mut fresh = Index {
        revision: Some(doc.revision),
        origin: lo,
        cells_x,
        cells_z,
        ways: vec![Vec::new(); cells_x * cells_z],
        points: vec![Vec::new(); cells_x * cells_z],
    };
    for (&id, way) in doc.osm.ways.iter().filter(|(_, w)| !w.meta.deleted()) {
        let points = doc.points(way);
        for pair in points.windows(2) {
            let (a, b) = (
                fresh.cell(pair[0].min(pair[1])),
                fresh.cell(pair[0].max(pair[1])),
            );
            for z in a.1..=b.1 {
                for x in a.0..=b.0 {
                    let list = &mut fresh.ways[z * cells_x + x];
                    if list.last() != Some(&id) {
                        list.push(id);
                    }
                }
            }
        }
    }
    for (&id, node) in doc.osm.nodes.iter().filter(|(_, n)| !n.meta.deleted()) {
        if PointKind::of(&node.meta.tags).is_some()
            && let Some(p) = doc.pos(id)
        {
            let (x, z) = fresh.cell(p);
            fresh.points[z * cells_x + x].push(id);
        }
    }
    *index = fresh;
    debug!("Map index rebuilt in {:?}", started.elapsed());
}
