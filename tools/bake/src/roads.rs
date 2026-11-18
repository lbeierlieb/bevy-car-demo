//! Roads as their own geometry: centrelines smoothed through the mapped points, heights
//! along them (`profile.rs`), a crown across, and the terrain cut and filled to meet them.

use std::collections::{HashMap, HashSet};

use crate::Point;
use crate::profile::{self, Problem};

/// Spacing of the smoothed centreline (m).
pub const STEP: f64 = 1.0;
/// Beside the road's edge, the terrain stays level with it this far (m)...
const SHOULDER: f64 = 1.0;
/// ...and then slopes this steeply (rise over run) towards the natural ground.
const EMBANKMENT: f64 = 0.6;
/// How far beyond the shoulder the terrain may be reshaped (m).
const EMBANKMENT_REACH: f64 = 12.0;
/// The terrain under a road and its shoulders lies this far below the road's edge (m),
/// out of sight under the road's own surface.
const SINK: f64 = 0.08;
/// Under a bridge the ground stays this far below the deck's edges at least: the slab's
/// depth (`DECK_DEPTH` in the game's `terrain.rs`) and a little room (m)...
const UNDER_DECK: f64 = 1.05;
/// ...out to this far beyond the deck's edges (the parapets and a margin; m).
const UNDER_DECK_REACH: f64 = 0.8;
/// Weight of the lidar on a bridge, where it shows the ground under the deck.
const BRIDGE_TRUST: f64 = 1e-4;

/// A road's surface, or a waterway.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum RoadKind {
    Asphalt = 0,
    Gravel = 1,
    Water = 2,
    Cobbles = 3,
    /// Earth and grass.
    Dirt = 4,
}

pub struct Road {
    pub kind: RoadKind,
    pub width: f64,
    pub bridge: bool,
    /// Smoothed centreline, at most `STEP` apart, through every mapped point.
    pub points: Vec<Point>,
    /// The OSM node each point is, if it's a mapped one.
    pub nodes: Vec<Option<i64>>,
    /// Height of the centreline at each point (waterways: none).
    pub heights: Vec<f64>,
    /// How bumpy the surface is: the displacement spectrum at 0.1 cycles per metre, in
    /// 10⁻⁶ m³, as in ISO 8608 (class A is 16, each class four times the one before).
    pub roughness: f64,
}

/// Roughness (see `Road::roughness`) for OSM's `smoothness`, or by surface if it isn't
/// tagged: asphalt smooth (class A), stones a little rougher, tracks rougher still.
pub fn roughness(kind: RoadKind, smoothness: Option<&str>) -> f64 {
    match smoothness {
        Some("excellent") => 4.0,
        Some("good") => 16.0,
        Some("intermediate") => 64.0,
        Some("bad") => 256.0,
        Some("very_bad") => 1024.0,
        Some("horrible") => 4096.0,
        Some("very_horrible" | "impassable") => 8192.0,
        _ => match kind {
            RoadKind::Asphalt => 16.0,
            RoadKind::Cobbles => 64.0,
            RoadKind::Gravel => 128.0,
            RoadKind::Dirt => 512.0,
            RoadKind::Water => 0.0,
        },
    }
}

/// Something built into a road where a node is tagged so: speed bumps, humps and tables
/// across it (`traffic_calming`), and potholes (`hazard=pothole`).
pub struct Feature {
    pub kind: FeatureKind,
    /// The road it's on, how far along it and how far right of the centreline (m).
    pub road: usize,
    pub along: f64,
    pub right: f64,
    /// Length along the road (bumps) or diameter (potholes), and height (negative: depth).
    pub size: f64,
    pub height: f64,
}

#[derive(Clone, Copy)]
pub enum FeatureKind {
    /// A short bump with a rounded top.
    Bump = 0,
    /// A long, gentle hump.
    Hump = 1,
    /// A flat top with ramps.
    Table = 2,
    Pothole = 3,
}

impl Feature {
    /// The feature a node with these tags (`height`, `diameter` and `depth` in metres
    /// override the defaults) describes, without its place yet.
    pub fn kind_of(tags: &crate::Tags) -> Option<(FeatureKind, f64, f64)> {
        let number = |k: &str| {
            tags.get(k)
                .and_then(|v| v.trim_end_matches('m').trim().parse::<f64>().ok())
        };
        let (kind, size, height) = match tags.get("traffic_calming").map(String::as_str) {
            Some("bump") => (FeatureKind::Bump, 0.6, 0.08),
            Some("hump") => (FeatureKind::Hump, 3.7, 0.07),
            Some("table") => (FeatureKind::Table, 8.0, 0.07),
            _ if tags.get("hazard").is_some_and(|h| h == "pothole") => {
                let depth = number("depth").unwrap_or(0.06);
                return Some((
                    FeatureKind::Pothole,
                    number("diameter").unwrap_or(0.8),
                    -depth,
                ));
            }
            _ => return None,
        };
        Some((kind, size, number("height").unwrap_or(height)))
    }

    /// Puts a feature at `p` onto the nearest drivable road reaching there (if any).
    pub fn place(
        roads: &[Road],
        (kind, size, height): (FeatureKind, f64, f64),
        p: Point,
    ) -> Option<Self> {
        let mut best: Option<(f64, Feature)> = None;
        for (r, road) in roads.iter().enumerate().filter(|(_, r)| r.drivable()) {
            let mut along = 0.0;
            for pair in road.points.windows(2) {
                let ((x0, z0), (x1, z1)) = (pair[0], pair[1]);
                let (dx, dz) = (x1 - x0, z1 - z0);
                let length = dx.hypot(dz).max(1e-9);
                let t = (((p.0 - x0) * dx + (p.1 - z0) * dz) / (length * length)).clamp(0.0, 1.0);
                let off = (p.0 - x0 - dx * t).hypot(p.1 - z0 - dz * t);
                // Right of the road: the side its perpendicular (-dz, dx) points to.
                let right = ((p.0 - x0) * -dz + (p.1 - z0) * dx) / length;
                if off <= road.width / 2.0 + 1.0 && best.as_ref().is_none_or(|b| off < b.0) {
                    let feature = Feature {
                        kind,
                        road: r,
                        along: along + t * length,
                        right,
                        size,
                        height,
                    };
                    best = Some((off, feature));
                }
                along += length;
            }
        }
        best.map(|b| b.1)
    }
}

impl Road {
    /// A road through the mapped (node, point)s, smoothed with a centripetal Catmull-Rom
    /// spline: it passes through each of them (so roads still meet where they share one)
    /// without the corners.
    pub fn new(kind: RoadKind, width: f64, bridge: bool, mapped: &[(i64, Point)]) -> Self {
        let mut mapped = mapped.to_vec();
        mapped.dedup_by(|b, a| a.1 == b.1);
        let p: Vec<Point> = mapped.iter().map(|m| m.1).collect();
        let n = p.len();
        let mut points = vec![p[0]];
        let mut nodes = vec![Some(mapped[0].0)];
        for i in 0..n - 1 {
            // A closed way bends on through its ends; an open one is extended straight.
            let closed = n > 2 && p[0] == p[n - 1];
            let before = match i {
                0 if closed => p[n - 2],
                0 => mirror(p[1], p[0]),
                _ => p[i - 1],
            };
            let after = match i + 2 {
                j if j < n => p[j],
                _ if closed => p[1],
                _ => mirror(p[n - 2], p[n - 1]),
            };
            let span = [before, p[i], p[i + 1], after];
            let steps = (distance(p[i], p[i + 1]) / STEP).ceil().max(1.0) as usize;
            for s in 1..steps {
                points.push(catmull_rom(span, s as f64 / steps as f64));
                nodes.push(None);
            }
            points.push(p[i + 1]);
            nodes.push(Some(mapped[i + 1].0));
        }
        Road {
            kind,
            width,
            bridge,
            points,
            nodes,
            heights: Vec::new(),
            roughness: 0.0,
        }
    }

    /// The road surface falls this much per metre from the middle to the edges.
    pub fn crown(&self) -> f64 {
        match self.kind {
            RoadKind::Asphalt | RoadKind::Cobbles => 0.025,
            RoadKind::Gravel | RoadKind::Dirt => 0.03,
            RoadKind::Water => 0.0,
        }
    }

    /// The shortest rise and fall the road follows (m): main roads are built smoothest.
    fn wavelength(&self) -> f64 {
        match self.kind {
            RoadKind::Asphalt if self.width >= 6.0 => 80.0,
            RoadKind::Asphalt => 45.0,
            RoadKind::Cobbles => 30.0,
            _ => 25.0,
        }
    }

    pub fn drivable(&self) -> bool {
        self.kind != RoadKind::Water
    }
}

fn distance(a: Point, b: Point) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

/// `p` mirrored through `centre`.
fn mirror(p: Point, centre: Point) -> Point {
    (2.0 * centre.0 - p.0, 2.0 * centre.1 - p.1)
}

/// The centripetal Catmull-Rom spline through `p[1]` and `p[2]` at `t` (0 to 1) between
/// them (Barry and Goldman's pyramid).
fn catmull_rom(p: [Point; 4], t: f64) -> Point {
    let mut k = [0.0; 4];
    for i in 1..4 {
        k[i] = k[i - 1] + distance(p[i - 1], p[i]).sqrt().max(1e-6);
    }
    let u = k[1] + (k[2] - k[1]) * t;
    let lerp = |a: Point, b: Point, ka: f64, kb: f64| {
        let w = (u - ka) / (kb - ka);
        (a.0 + (b.0 - a.0) * w, a.1 + (b.1 - a.1) * w)
    };
    let a1 = lerp(p[0], p[1], k[0], k[1]);
    let a2 = lerp(p[1], p[2], k[1], k[2]);
    let a3 = lerp(p[2], p[3], k[2], k[3]);
    let b1 = lerp(a1, a2, k[0], k[2]);
    let b2 = lerp(a2, a3, k[1], k[3]);
    lerp(b1, b2, k[1], k[2])
}

/// Unit direction along the road at point `i`, and the distance to the next point.
fn tangent(points: &[Point], i: usize) -> Point {
    let (a, b) = (
        points[i.saturating_sub(1)],
        points[(i + 1).min(points.len() - 1)],
    );
    let length = distance(a, b).max(1e-9);
    ((b.0 - a.0) / length, (b.1 - a.1) / length)
}

/// Sets the heights of all drivable roads: smooth profiles that follow `lidar`, the
/// ground height at a point. The lidar is read across the road's width at each point and
/// the median taken, so a centreline mapped a little off onto a verge or a ditch still
/// finds the road.
pub fn profiles(roads: &mut [Road], lidar: impl Fn(Point) -> f64) {
    let mut problem = Problem::default();
    // Points that are the same mapped node share a height.
    let mut of_node: HashMap<i64, usize> = HashMap::new();
    let mut vars: Vec<Vec<usize>> = Vec::new();
    for road in roads.iter() {
        let mut ids = Vec::new();
        for &node in &road.nodes {
            let fresh = |problem: &mut Problem| {
                problem.count += 1;
                problem.count - 1
            };
            ids.push(match node {
                Some(node) => *of_node.entry(node).or_insert_with(|| fresh(&mut problem)),
                None => fresh(&mut problem),
            });
        }
        vars.push(ids);
    }
    for (road, ids) in roads.iter().zip(&vars).filter(|(r, _)| r.drivable()) {
        let p = &road.points;
        let gaps: Vec<f64> = p.windows(2).map(|w| distance(w[0], w[1])).collect();
        let stiffness = profile::stiffness(road.wavelength());
        for i in 0..p.len() {
            let length =
                (gaps.get(i.wrapping_sub(1)).unwrap_or(&0.0) + gaps.get(i).unwrap_or(&0.0)) / 2.0;
            let (tx, tz) = tangent(p, i);
            let mut across: Vec<f64> = [-0.4, -0.2, 0.0, 0.2, 0.4]
                .iter()
                .map(|f| {
                    let c = f * road.width;
                    lidar((p[i].0 - tz * c, p[i].1 + tx * c))
                })
                .collect();
            across.sort_by(f64::total_cmp);
            let trust = if road.bridge { BRIDGE_TRUST } else { 1.0 };
            problem.data.push((ids[i], trust * length, across[2]));
            if i > 0 && i + 1 < p.len() {
                problem.bends.push((
                    [ids[i - 1], ids[i], ids[i + 1]],
                    [gaps[i - 1], gaps[i]],
                    stiffness,
                ));
            }
        }
    }
    // Where a road ends, it runs on into another: the only other one there, or else
    // whichever leaves the shared point most nearly straight ahead (if any does). It
    // bends smoothly across, and two roads running on into each other are solved as one.
    struct Arm {
        road: usize,
        /// The road's point at the shared one, and the next point along the arm.
        at: usize,
        next: usize,
        direction: Point,
        length: f64,
        stiffness: f64,
    }
    let mut arms: HashMap<usize, Vec<Arm>> = HashMap::new();
    for (r, (road, ids)) in roads.iter().zip(&vars).enumerate() {
        if !road.drivable() {
            continue;
        }
        let p = &road.points;
        let stiffness = profile::stiffness(road.wavelength());
        for i in 0..p.len() {
            for j in [i.wrapping_sub(1), i + 1] {
                if j < p.len() && ids[j] != ids[i] {
                    let length = distance(p[i], p[j]).max(1e-9);
                    let direction = ((p[j].0 - p[i].0) / length, (p[j].1 - p[i].1) / length);
                    arms.entry(ids[i]).or_default().push(Arm {
                        road: r,
                        at: i,
                        next: j,
                        direction,
                        length,
                        stiffness,
                    });
                }
            }
        }
    }
    let is_end = |arm: &Arm| arm.at == 0 || arm.at == roads[arm.road].points.len() - 1;
    // Each road end (road, at) to the arm it runs on into: (road, at) of that arm.
    let mut runs_on: HashMap<(usize, usize), (usize, usize)> = HashMap::new();
    let mut joined = HashSet::new();
    for here in arms.values() {
        for arm in here.iter().filter(|a| is_end(a)) {
            let others = here.iter().filter(|a| !std::ptr::eq(*a, arm));
            let into = if here.len() == 2 && here.iter().all(is_end) {
                others.into_iter().next()
            } else {
                let dot =
                    |a: &&Arm| arm.direction.0 * a.direction.0 + arm.direction.1 * a.direction.1;
                others
                    .filter(|a| dot(a) < -0.85)
                    .min_by(|a, b| dot(a).total_cmp(&dot(b)))
            };
            let Some(into) = into else { continue };
            let ids = (&vars[arm.road], &vars[into.road]);
            let (from, node, to) = (ids.0[arm.next], ids.0[arm.at], ids.1[into.next]);
            if joined.insert((from.min(to), node, from.max(to))) {
                problem.bends.push((
                    [from, node, to],
                    [arm.length, into.length],
                    arm.stiffness.min(into.stiffness),
                ));
            }
            if is_end(into) && into.road != arm.road {
                runs_on.insert((arm.road, arm.at), (into.road, into.at));
            }
        }
    }
    // Chains of roads running on into each other (both ways), each road in one.
    let mut used = vec![false; roads.len()];
    for start in 0..roads.len() {
        if used[start] || !roads[start].drivable() {
            continue;
        }
        used[start] = true;
        let mut chain: Vec<usize> = vars[start].clone();
        for forward in [true, false] {
            let (mut road, mut end) = (
                start,
                if forward {
                    roads[start].points.len() - 1
                } else {
                    0
                },
            );
            while let Some(&(next, at)) = runs_on.get(&(road, end)) {
                if used[next] || runs_on.get(&(next, at)) != Some(&(road, end)) {
                    break;
                }
                used[next] = true;
                let mut ids = vars[next].clone();
                if at != 0 {
                    ids.reverse();
                }
                // `ids` now starts at the shared point.
                if forward {
                    chain.extend(&ids[1..]);
                } else {
                    ids.reverse();
                    ids.pop();
                    ids.extend(chain);
                    chain = ids;
                }
                road = next;
                end = if at == 0 {
                    roads[next].points.len() - 1
                } else {
                    0
                };
            }
        }
        problem.chains.push(chain);
    }
    let heights = problem.solve();
    for (road, ids) in roads.iter_mut().zip(&vars) {
        road.heights = if road.drivable() {
            ids.iter().map(|&i| heights[i]).collect()
        } else {
            Vec::new()
        };
    }
}

/// The terrain grid: vertices `cell` apart, `cells_x` × `cells_z` cells, centred on the
/// origin.
pub struct Grid {
    pub cells_x: usize,
    pub cells_z: usize,
    pub cell: f64,
}

impl Grid {
    fn half(&self) -> Point {
        (
            self.cells_x as f64 * self.cell / 2.0,
            self.cells_z as f64 * self.cell / 2.0,
        )
    }

    /// The vertex columns or rows between `lo` and `hi` along x (`along_z` false) or z.
    fn span(&self, lo: f64, hi: f64, along_z: bool) -> std::ops::RangeInclusive<usize> {
        let (half, count) = if along_z {
            (self.half().1, self.cells_z)
        } else {
            (self.half().0, self.cells_x)
        };
        let index = |v: f64| ((v + half) / self.cell).clamp(0.0, count as f64) as usize;
        index(lo)..=index(hi)
    }

    fn vertex(&self, ix: usize, iz: usize) -> Point {
        let half = self.half();
        (
            ix as f64 * self.cell - half.0,
            iz as f64 * self.cell - half.1,
        )
    }
}

/// Cuts and fills the terrain `heights` (one per grid vertex) to the drivable roads that
/// aren't bridges: under a road and its shoulders it lies `SINK` below the road's surface,
/// beyond them it slopes at `EMBANKMENT` up or down to the natural ground. Each vertex
/// follows the road whose edge is nearest.
pub fn conform(roads: &[Road], heights: &mut [f64], grid: &Grid) {
    let n = grid.cells_x + 1;
    // Per vertex: (distance beyond the nearest road's edge, that road's surface there).
    let mut nearest = vec![(f64::INFINITY, 0.0); heights.len()];
    for road in roads.iter().filter(|r| r.drivable() && !r.bridge) {
        let reach = road.width / 2.0 + SHOULDER + EMBANKMENT_REACH;
        for (i, pair) in road.points.windows(2).enumerate() {
            let ((x0, z0), (x1, z1)) = (pair[0], pair[1]);
            let (dx, dz) = (x1 - x0, z1 - z0);
            let len2 = (dx * dx + dz * dz).max(1e-9);
            for iz in grid.span(z0.min(z1) - reach, z0.max(z1) + reach, true) {
                for ix in grid.span(x0.min(x1) - reach, x0.max(x1) + reach, false) {
                    let (px, pz) = grid.vertex(ix, iz);
                    let t = (((px - x0) * dx + (pz - z0) * dz) / len2).clamp(0.0, 1.0);
                    let off = (px - x0 - dx * t).hypot(pz - z0 - dz * t);
                    let edge = off - road.width / 2.0;
                    let slot = &mut nearest[iz * n + ix];
                    if edge < slot.0 {
                        let centre = road.heights[i] * (1.0 - t) + road.heights[i + 1] * t;
                        let surface = centre - road.crown() * off.min(road.width / 2.0);
                        *slot = (edge, surface);
                    }
                }
            }
        }
    }
    for (h, &(edge, surface)) in heights.iter_mut().zip(&nearest) {
        if edge.is_infinite() {
            continue;
        }
        let level = surface - SINK;
        let run = (edge - SHOULDER).max(0.0);
        *h = level + (*h - level).clamp(-run * EMBANKMENT, run * EMBANKMENT);
    }
}

/// Lowers the terrain under bridges to `UNDER_DECK` below their decks, where it would
/// otherwise come up through them: approach embankments reaching under a deck's ends, and
/// small bridges over ditches, whose lidar is at road level but for the ditch. Only
/// alongside the deck, not beyond its ends, where the approaches carry on.
pub fn carve_under_bridges(roads: &[Road], heights: &mut [f64], grid: &Grid) {
    let n = grid.cells_x + 1;
    for road in roads.iter().filter(|r| r.drivable() && r.bridge) {
        let reach = road.width / 2.0 + UNDER_DECK_REACH;
        let edge = |i: usize| road.heights[i] - road.crown() * road.width / 2.0;
        for (i, pair) in road.points.windows(2).enumerate() {
            let ((x0, z0), (x1, z1)) = (pair[0], pair[1]);
            let (dx, dz) = (x1 - x0, z1 - z0);
            let len2 = (dx * dx + dz * dz).max(1e-9);
            for iz in grid.span(z0.min(z1) - reach, z0.max(z1) + reach, true) {
                for ix in grid.span(x0.min(x1) - reach, x0.max(x1) + reach, false) {
                    let (px, pz) = grid.vertex(ix, iz);
                    let t = ((px - x0) * dx + (pz - z0) * dz) / len2;
                    let off = (px - x0 - dx * t).hypot(pz - z0 - dz * t);
                    if (0.0..=1.0).contains(&t) && off <= reach {
                        let limit = edge(i) * (1.0 - t) + edge(i + 1) * t - UNDER_DECK;
                        let h = &mut heights[iz * n + ix];
                        *h = h.min(limit);
                    }
                }
            }
        }
    }
}
