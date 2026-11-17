//! The roads as a network the traffic drives on: stretches of road between junctions, a
//! lane each way, and a speed limit along each stretch.
//!
//! OSM roads share their points where they meet, so wherever a point is on more than one
//! road (or a road ends) there's a junction. The limits follow German defaults, judged
//! from what the map has: the road's width (the bake set it from the road's class),
//! whether it's paved, and how many houses are around (built-up areas).

use std::collections::HashMap;

use bevy::prelude::*;

use crate::terrain::{RoadKind, Terrain};

/// Roads narrower than this (service roads, driveways, tracks) get no traffic (m).
const MIN_WIDTH: f32 = 4.0;
/// How far lanes are from the centreline: a quarter of the road's width, but enough for
/// two cars to pass on a narrow road and no more than a lane on a wide one (m).
const LANE_LEAST: f32 = 1.05;
const LANE_MOST: f32 = 2.6;
/// Spacing of the speed limits along a stretch (m).
pub const LIMIT_STEP: f32 = 20.0;
/// Within this of a point on a road, this many houses make it a built-up area (m).
const BUILT_RADIUS: f32 = 90.0;
const BUILT_HOUSES: usize = 8;
/// Built-up areas on roads narrower than this are residential streets: 30 (m).
const RESIDENTIAL_WIDTH: f32 = 5.5;

const fn kmh(v: f32) -> f32 {
    v / 3.6
}

/// A stretch of road between two junctions (or a junction and a dead end).
pub struct Edge {
    /// The nodes at its start and its end.
    pub ends: [u32; 2],
    pub width: f32,
    /// Length of the centreline (m).
    pub length: f32,
    /// The lane from `ends[0]` to `ends[1]`, and the one back.
    pub lanes: [Lane; 2],
    /// Speed limit (m/s) every `LIMIT_STEP` along the centreline from `ends[0]`.
    pub limits: Vec<f32>,
}

impl Edge {
    /// The speed limit `s` along the centreline from `ends[0]`.
    pub fn limit(&self, s: f32) -> f32 {
        let i = ((s / LIMIT_STEP).round().max(0.0) as usize).min(self.limits.len() - 1);
        self.limits[i]
    }
}

/// A road's lane: a line kept to the right of the centreline.
pub struct Lane {
    pub points: Vec<Vec2>,
    /// Distance of each point from the first (m).
    pub at: Vec<f32>,
}

impl Lane {
    fn new(points: Vec<Vec2>) -> Self {
        let mut at = vec![0.0];
        for pair in points.windows(2) {
            at.push(at.last().unwrap() + pair[0].distance(pair[1]));
        }
        Self { points, at }
    }

    pub fn length(&self) -> f32 {
        *self.at.last().unwrap()
    }

    /// The segment `s` along the lane is on.
    fn segment(&self, s: f32) -> usize {
        self.at
            .partition_point(|&a| a <= s)
            .clamp(1, self.points.len() - 1)
            - 1
    }

    /// The lane's direction `s` along it (unit).
    pub fn direction(&self, s: f32) -> Vec2 {
        let i = self.segment(s);
        (self.points[i + 1] - self.points[i]).normalize_or(Vec2::NEG_Y)
    }

    /// The point `s` along the lane (clamped to it).
    pub fn point(&self, s: f32) -> Vec2 {
        let i = self.segment(s);
        let span = (self.at[i + 1] - self.at[i]).max(1e-6);
        let t = ((s - self.at[i]) / span).clamp(0.0, 1.0);
        self.points[i].lerp(self.points[i + 1], t)
    }
}

/// A junction or a dead end, and the stretches that meet there.
pub struct Node {
    pub edges: Vec<u32>,
}

/// One stretch driven one way: from `ends[0]` to `ends[1]` if `forward`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leg {
    pub edge: u32,
    pub forward: bool,
}

#[derive(Resource, Default)]
pub struct Network {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Network {
    pub fn new(terrain: &Terrain) -> Self {
        // (width, unpaved, centreline)
        let roads: Vec<(f32, bool, Vec<Vec2>)> = terrain
            .roads
            .iter()
            .filter(|road| road.kind != RoadKind::Water && road.width >= MIN_WIDTH)
            .map(|road| {
                let mut points = road.points.clone();
                points.dedup();
                (road.width, road.kind == RoadKind::Unpaved, points)
            })
            .filter(|(_, _, points)| points.len() >= 2)
            .collect();

        // Points on more than one road, or used twice, and every road's ends are nodes.
        let key = |p: Vec2| ((p.x * 100.0).round() as i32, (p.y * 100.0).round() as i32);
        let mut uses: HashMap<(i32, i32), u32> = HashMap::new();
        for (_, _, points) in &roads {
            for (i, &p) in points.iter().enumerate() {
                let end = i == 0 || i == points.len() - 1;
                *uses.entry(key(p)).or_default() += if end { 2 } else { 1 };
            }
        }
        let mut network = Network::default();
        let mut node_of: HashMap<(i32, i32), u32> = HashMap::new();
        let mut node = |network: &mut Network, p: Vec2| {
            *node_of.entry(key(p)).or_insert_with(|| {
                network.nodes.push(Node { edges: Vec::new() });
                network.nodes.len() as u32 - 1
            })
        };
        let houses = Houses::new(terrain);
        for &(width, unpaved, ref points) in &roads {
            let mut start = 0;
            for i in 1..points.len() {
                if uses[&key(points[i])] < 2 {
                    continue;
                }
                let stretch = &points[start..=i];
                start = i;
                let length: f32 = stretch.windows(2).map(|w| w[0].distance(w[1])).sum();
                if length < 0.5 {
                    continue;
                }
                let ends = [
                    node(&mut network, stretch[0]),
                    node(&mut network, stretch[stretch.len() - 1]),
                ];
                let edge = network.edges.len() as u32;
                network.nodes[ends[0] as usize].edges.push(edge);
                if ends[1] != ends[0] {
                    network.nodes[ends[1] as usize].edges.push(edge);
                }
                let offset = (width / 4.0).clamp(LANE_LEAST, LANE_MOST);
                let forward = offset_line(stretch, offset);
                let back: Vec<Vec2> = stretch.iter().rev().copied().collect();
                let backward = offset_line(&back, offset);
                let limits = limits(stretch, length, width, unpaved, &houses);
                network.edges.push(Edge {
                    ends,
                    width,
                    length,
                    lanes: [Lane::new(forward), Lane::new(backward)],
                    limits,
                });
            }
        }
        network
    }

    pub fn lane(&self, leg: Leg) -> &Lane {
        &self.edges[leg.edge as usize].lanes[usize::from(!leg.forward)]
    }

    /// The speed limit `s` along a leg's lane.
    pub fn limit(&self, leg: Leg, s: f32) -> f32 {
        let edge = &self.edges[leg.edge as usize];
        let along = s / self.lane(leg).length().max(1e-3) * edge.length;
        edge.limit(if leg.forward {
            along
        } else {
            edge.length - along
        })
    }

    /// The node a leg ends at.
    pub fn end(&self, leg: Leg) -> u32 {
        self.edges[leg.edge as usize].ends[usize::from(leg.forward)]
    }

    /// The legs that leave the node `after` ends at, other than back the way it came
    /// (a loop's way round counts once).
    pub fn next(&self, after: Leg) -> impl Iterator<Item = Leg> + '_ {
        let node = self.end(after);
        self.nodes[node as usize]
            .edges
            .iter()
            .filter(move |&&edge| edge != after.edge)
            .map(move |&edge| Leg {
                edge,
                forward: self.edges[edge as usize].ends[0] == node,
            })
    }
}

/// A line `offset` to the right of `points` (in the xz plane, x east and z south, seen
/// from above with y up: right of the direction of travel).
fn offset_line(points: &[Vec2], offset: f32) -> Vec<Vec2> {
    let right = |d: Vec2| Vec2::new(-d.y, d.x);
    let n = points.len();
    (0..n)
        .map(|i| {
            let before = (i > 0).then(|| (points[i] - points[i - 1]).normalize_or_zero());
            let after = (i + 1 < n).then(|| (points[i + 1] - points[i]).normalize_or_zero());
            let direction = match (before, after) {
                (Some(a), Some(b)) => (a + b).normalize_or(b),
                (Some(a), None) => a,
                (None, Some(b)) => b,
                (None, None) => Vec2::X,
            };
            // Mitred, but not by more than twice on a sharp corner.
            let stretch = before
                .map(|a| 1.0 / a.dot(direction).max(0.5))
                .unwrap_or(1.0);
            points[i] + right(direction) * offset * stretch
        })
        .collect()
}

/// The speed limits every `LIMIT_STEP` along a stretch of road.
fn limits(points: &[Vec2], length: f32, width: f32, unpaved: bool, houses: &Houses) -> Vec<f32> {
    let count = (length / LIMIT_STEP).round() as usize + 1;
    let lane = Lane::new(points.to_vec());
    (0..count)
        .map(|i| {
            let at = lane.point(i as f32 * LIMIT_STEP);
            let mut limit = if width >= 6.0 {
                kmh(100.0)
            } else if width >= 4.5 {
                kmh(80.0)
            } else {
                kmh(60.0)
            };
            if unpaved {
                limit = limit.min(kmh(50.0));
            }
            if houses.count(at, BUILT_RADIUS) >= BUILT_HOUSES {
                limit = limit.min(if width < RESIDENTIAL_WIDTH {
                    kmh(30.0)
                } else {
                    kmh(50.0)
                });
            }
            limit
        })
        .collect()
}

/// The houses' middles in a grid, for counting them around a point.
struct Houses {
    origin: Vec2,
    columns: usize,
    rows: usize,
    cells: Vec<Vec<Vec2>>,
}

impl Houses {
    const CELL: f32 = 50.0;

    fn new(terrain: &Terrain) -> Self {
        let half = terrain.half_size();
        let origin = -half;
        let columns = (2.0 * half.x / Self::CELL).ceil() as usize + 1;
        let rows = (2.0 * half.y / Self::CELL).ceil() as usize + 1;
        let mut houses = Self {
            origin,
            columns,
            rows,
            cells: vec![Vec::new(); columns * rows],
        };
        for building in &terrain.buildings {
            let n = building.outline.len().max(1) as f32;
            let middle = building.outline.iter().copied().sum::<Vec2>() / n;
            if let Some(cell) = houses.cell(middle) {
                houses.cells[cell].push(middle);
            }
        }
        houses
    }

    fn cell(&self, p: Vec2) -> Option<usize> {
        let c = ((p - self.origin) / Self::CELL).floor();
        (c.x >= 0.0 && c.y >= 0.0 && (c.x as usize) < self.columns && (c.y as usize) < self.rows)
            .then(|| c.y as usize * self.columns + c.x as usize)
    }

    fn count(&self, p: Vec2, radius: f32) -> usize {
        let low = ((p - radius - self.origin) / Self::CELL)
            .floor()
            .max(Vec2::ZERO);
        let high = ((p + radius - self.origin) / Self::CELL).floor();
        let mut count = 0;
        for row in low.y as usize..=(high.y.max(0.0) as usize).min(self.rows - 1) {
            for column in low.x as usize..=(high.x.max(0.0) as usize).min(self.columns - 1) {
                count += self.cells[row * self.columns + column]
                    .iter()
                    .filter(|h| h.distance_squared(p) < radius * radius)
                    .count();
            }
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roads_make_a_network_with_limits() {
        let terrain = Terrain::baked();
        let start = std::time::Instant::now();
        let network = Network::new(&terrain);
        let elapsed = start.elapsed();
        let total: f32 = network.edges.iter().map(|e| e.length).sum();
        let mut by_limit: HashMap<u32, f32> = HashMap::new();
        for edge in &network.edges {
            for &limit in &edge.limits {
                *by_limit.entry((limit * 3.6).round() as u32).or_default() +=
                    edge.length / edge.limits.len() as f32;
            }
        }
        let mut by_limit: Vec<_> = by_limit.into_iter().collect();
        by_limit.sort_by_key(|&(limit, _)| limit);
        let junctions = network.nodes.iter().filter(|n| n.edges.len() > 2).count();
        let dead_ends = network.nodes.iter().filter(|n| n.edges.len() == 1).count();
        println!(
            "{} edges, {:.0} km, {} nodes ({junctions} junctions, {dead_ends} dead ends) in {elapsed:?}",
            network.edges.len(),
            total / 1000.0,
            network.nodes.len()
        );
        for (limit, length) in by_limit {
            println!("{limit:>4} km/h: {:.1} km", length / 1000.0);
        }
        assert!(junctions > 100 && total > 50e3);
        // Every lane is as long as its stretch, give or take the corners.
        for edge in &network.edges {
            for lane in &edge.lanes {
                assert!((lane.length() - edge.length).abs() < 0.3 * edge.length + 5.0);
            }
        }
    }

    #[test]
    fn lanes_keep_right() {
        let line = [Vec2::ZERO, Vec2::new(0.0, -10.0), Vec2::new(10.0, -10.0)];
        // Heading north (-z), the right is east (+x).
        let lane = Lane::new(offset_line(&line, 1.5));
        assert!((lane.points[0] - Vec2::new(1.5, 0.0)).length() < 1e-5);
        // Turning right onto east, the right is south (+z): the inside of the corner.
        assert!((lane.points[2] - Vec2::new(10.0, -8.5)).length() < 1e-5);
        assert!((lane.point(lane.length()) - lane.points[2]).length() < 1e-5);
    }
}
