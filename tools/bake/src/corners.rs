//! Curbed corners where roads meet: between each two neighbouring roads leaving a
//! junction, the inside of the turn is filled out to an arc tangent to both edges, so
//! the corner isn't a square notch with the terrain showing in it.

use std::collections::BTreeMap;

use crate::Point;
use crate::roads::{Grid, Road, RoadKind};

/// Corner radius by the narrower of the two roads' widths: tracks, streets, main roads (m).
fn radius(width: f64) -> f64 {
    if width < 3.5 {
        2.5
    } else if width < 6.0 {
        5.0
    } else {
        8.0
    }
}
/// A road's direction leaving a junction is taken to a point this far along it (m).
const AIM: f64 = 5.0;
/// Corners only between roads meeting at angles between these (radians): sharper ones
/// would need a long arc, flatter ones have nothing to fill.
const SHARPEST: f64 = 25.0 * std::f64::consts::PI / 180.0;
const FLATTEST: f64 = 160.0 * std::f64::consts::PI / 180.0;
/// Arc points at most this far apart in angle (radians).
const ARC_STEP: f64 = 10.0 * std::f64::consts::PI / 180.0;

/// A filled corner: a fan around its first point, the junction's node, out along one
/// road's centreline, across it to the arc, round the arc and back in along the other
/// road's centreline (see `corner`). Points are (x, z, height).
pub struct Corner {
    pub kind: RoadKind,
    pub points: Vec<(f64, f64, f64)>,
}

/// One road leaving a junction node.
struct Arm {
    road: usize,
    /// The road's point at the node and the way along it (+1 or -1).
    at: usize,
    step: isize,
    direction: Point,
    angle: f64,
    half: f64,
    /// How far the road goes on this way (m).
    length: f64,
}

impl Arm {
    /// The road's point `s` metres along the arm, and its centreline height there.
    fn along(&self, road: &Road, s: f64) -> (Point, f64) {
        let (p, h) = (&road.points, &road.heights);
        let mut i = self.at;
        let mut gone = 0.0;
        loop {
            let j = i as isize + self.step;
            if j < 0 || j as usize >= p.len() {
                return (p[i], h[i]);
            }
            let j = j as usize;
            let d = distance(p[i], p[j]);
            if gone + d >= s {
                let t = ((s - gone) / d.max(1e-9)).clamp(0.0, 1.0);
                let point = (
                    p[i].0 + (p[j].0 - p[i].0) * t,
                    p[i].1 + (p[j].1 - p[i].1) * t,
                );
                return (point, h[i] + (h[j] - h[i]) * t);
            }
            gone += d;
            i = j;
        }
    }
}

fn distance(a: Point, b: Point) -> f64 {
    (b.0 - a.0).hypot(b.1 - a.1)
}

fn rank(kind: RoadKind) -> u8 {
    match kind {
        RoadKind::Asphalt => 4,
        RoadKind::Cobbles => 3,
        RoadKind::Gravel => 2,
        RoadKind::Dirt => 1,
        RoadKind::Water => 0,
    }
}

/// The corners at every node where three or more roads on the ground leave (none at
/// junctions with a bridge), each one shrunk or dropped where it would reach into a
/// building (`outlines`).
pub fn corners(roads: &[Road], outlines: &[&[Point]]) -> Vec<Corner> {
    // By node, in order: the same map always bakes to the same file.
    let mut arms: BTreeMap<i64, Vec<Arm>> = BTreeMap::new();
    let mut bridged: Vec<i64> = Vec::new();
    for (r, road) in roads.iter().enumerate().filter(|(_, r)| r.drivable()) {
        let p = &road.points;
        let total: f64 = p.windows(2).map(|w| distance(w[0], w[1])).sum();
        let mut gone = 0.0;
        for (i, node) in road.nodes.iter().enumerate() {
            if i > 0 {
                gone += distance(p[i - 1], p[i]);
            }
            let Some(node) = *node else { continue };
            if road.bridge {
                bridged.push(node);
                continue;
            }
            for (step, length) in [(-1isize, gone), (1, total - gone)] {
                if length < 0.5 {
                    continue;
                }
                let mut arm = Arm {
                    road: r,
                    at: i,
                    step,
                    direction: (0.0, 0.0),
                    angle: 0.0,
                    half: road.widths[i] / 2.0,
                    length,
                };
                let (aim, _) = arm.along(road, AIM.min(length));
                let d = distance(p[i], aim).max(1e-9);
                arm.direction = ((aim.0 - p[i].0) / d, (aim.1 - p[i].1) / d);
                arm.angle = arm.direction.1.atan2(arm.direction.0);
                arms.entry(node).or_default().push(arm);
            }
        }
    }
    for node in bridged {
        arms.remove(&node);
    }
    let mut corners = Vec::new();
    for (_, mut here) in arms {
        if here.len() < 3 {
            continue;
        }
        here.sort_by(|a, b| a.angle.total_cmp(&b.angle));
        for k in 0..here.len() {
            let (a, b) = (&here[k], &here[(k + 1) % here.len()]);
            let mut theta = b.angle - a.angle;
            if theta <= 0.0 {
                theta += std::f64::consts::TAU;
            }
            if !(SHARPEST..=FLATTEST).contains(&theta) {
                continue;
            }
            let wanted = radius((2.0 * a.half).min(2.0 * b.half));
            for shrink in [1.0, 0.6, 0.35] {
                if let Some(corner) = corner(roads, a, b, wanted * shrink, outlines) {
                    corners.push(corner);
                    break;
                }
            }
        }
    }
    corners
}

/// A point of an arm's centreline, from the node out: where it is, the way across to
/// the arm's left (unit), half the road's width and the centreline's height there.
#[derive(Clone, Copy)]
struct Station {
    at: Point,
    left: Point,
    half: f64,
    height: f64,
}

impl Station {
    fn lerp(&self, other: &Station, t: f64) -> Station {
        let mix = |a: f64, b: f64| a + (b - a) * t;
        let left = (
            mix(self.left.0, other.left.0),
            mix(self.left.1, other.left.1),
        );
        let length = left.0.hypot(left.1).max(1e-9);
        Station {
            at: (mix(self.at.0, other.at.0), mix(self.at.1, other.at.1)),
            left: (left.0 / length, left.1 / length),
            half: mix(self.half, other.half),
            height: mix(self.height, other.height),
        }
    }

    /// The point `off` metres to the left of the centreline (negative: right).
    fn off(&self, off: f64) -> Point {
        (self.at.0 + self.left.0 * off, self.at.1 + self.left.1 * off)
    }
}

impl Arm {
    /// Its centreline from the node out, `most` metres at most, across as the game draws
    /// the ribbon (square to the centreline through the points either side).
    fn stations(&self, road: &Road, most: f64) -> Vec<Station> {
        let (p, n) = (&road.points, road.points.len());
        let station = |i: usize| {
            let (a, b) = (p[i.saturating_sub(1)], p[(i + 1).min(n - 1)]);
            let (dx, dz) = (b.0 - a.0, b.1 - a.1);
            let length = dx.hypot(dz).max(1e-9);
            let s = self.step as f64;
            Station {
                at: p[i],
                left: (-dz * s / length, dx * s / length),
                half: road.widths[i] / 2.0,
                height: road.heights[i],
            }
        };
        let mut out = vec![station(self.at)];
        let (mut i, mut gone) = (self.at, 0.0);
        loop {
            let j = i as isize + self.step;
            if j < 0 || j as usize >= n {
                return out;
            }
            let j = j as usize;
            let d = distance(p[i], p[j]);
            if gone + d >= most {
                let t = ((most - gone) / d.max(1e-9)).clamp(0.0, 1.0);
                out.push(station(i).lerp(&station(j), t));
                return out;
            }
            out.push(station(j));
            gone += d;
            i = j;
        }
    }
}

/// Where segment `a`–`b` crosses `c`–`d`: how far along the first (0 to 1).
fn crossing(a: Point, b: Point, c: Point, d: Point) -> Option<f64> {
    let (r, s) = ((b.0 - a.0, b.1 - a.1), (d.0 - c.0, d.1 - c.1));
    let det = r.0 * s.1 - r.1 * s.0;
    if det.abs() < 1e-12 {
        return None;
    }
    let (qx, qz) = (c.0 - a.0, c.1 - a.1);
    let t = (qx * s.1 - qz * s.0) / det;
    let u = (qx * r.1 - qz * r.0) / det;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then_some(t)
}

/// The corner between arm `a` and the next arm `b` counterclockwise, of radius `r`;
/// `None` if it doesn't fit within most of the arms' lengths or its arc reaches into a
/// building (`outlines`). Built on
/// the roads' edges as drawn, curves and all: the arc's centre is where the edges moved
/// out by the radius cross, its ends where it touches them. The outline goes from the
/// node out along `a`'s centreline, across to the arc's end on its edge, round the arc,
/// and back in along `b`'s, so all of the corner but what lies between the edges and
/// the arc is under the roads (and cut away with them).
fn corner(roads: &[Road], a: &Arm, b: &Arm, r: f64, outlines: &[&[Point]]) -> Option<Corner> {
    let (ra, rb) = (&roads[a.road], &roads[b.road]);
    // Far enough for the widest, sharpest corner, but most of each arm at most.
    let reach = |arm: &Arm| (0.8 * arm.length).min(80.0);
    let (sa, sb) = (a.stations(ra, reach(a)), b.stations(rb, reach(b)));
    if sa.len() < 2 || sb.len() < 2 {
        return None;
    }
    // The edges moved out by `r` (left of `a`, right of `b`): where they first cross,
    // by the stations' place along `a` and `b`.
    let centre = |r: f64| -> Option<((usize, f64), (usize, f64))> {
        let mut best: Option<((usize, f64), (usize, f64))> = None;
        for i in 0..sa.len() - 1 {
            let (p, q) = (sa[i].off(sa[i].half + r), sa[i + 1].off(sa[i + 1].half + r));
            for j in 0..sb.len() - 1 {
                let (u, v) = (
                    sb[j].off(-sb[j].half - r),
                    sb[j + 1].off(-sb[j + 1].half - r),
                );
                let (Some(t), Some(s)) = (crossing(p, q, u, v), crossing(u, v, p, q)) else {
                    continue;
                };
                if best.is_none_or(|((bi, bt), _)| (i as f64 + t) < bi as f64 + bt) {
                    best = Some(((i, t), (j, s)));
                }
            }
            if best.is_some() {
                break;
            }
        }
        best
    };
    let ((i, t), (j, s)) = centre(r)?;
    let (at_a, at_b) = (sa[i].lerp(&sa[i + 1], t), sb[j].lerp(&sb[j + 1], s));
    let c = at_a.off(at_a.half + r);
    let (ta, tb) = (at_a.off(at_a.half), at_b.off(-at_b.half));
    let mut outline = vec![sa[0].at];
    // The centrelines a few metres apart: close enough to stay under the roads.
    let thin = |stations: &[Station], last: usize| -> Vec<Point> {
        (1..=last)
            .filter(|k| k % 3 == 0)
            .map(|k| stations[k].at)
            .collect()
    };
    outline.extend(thin(&sa, i));
    outline.push(at_a.at);
    outline.push(ta);
    let (ca, cb) = (distance(c, ta), distance(c, tb));
    let (start, end) = (
        (ta.1 - c.1).atan2(ta.0 - c.0),
        (tb.1 - c.1).atan2(tb.0 - c.0),
    );
    let mut sweep = end - start;
    while sweep > std::f64::consts::PI {
        sweep -= std::f64::consts::TAU;
    }
    while sweep < -std::f64::consts::PI {
        sweep += std::f64::consts::TAU;
    }
    let steps = (sweep.abs() / ARC_STEP).ceil().max(2.0) as usize;
    for k in 1..steps {
        let f = k as f64 / steps as f64;
        let (angle, radius) = (start + sweep * f, ca + (cb - ca) * f);
        outline.push((c.0 + radius * angle.cos(), c.1 + radius * angle.sin()));
    }
    outline.push(tb);
    let arc = &outline[outline.len() - steps - 1..];
    if arc
        .iter()
        .any(|&p| outlines.iter().any(|o| crate::terrain::inside(o, p)))
    {
        return None;
    }
    outline.push(at_b.at);
    let mut back = thin(&sb, j);
    back.reverse();
    outline.extend(back);
    // Heights: each road's surface at the point nearest along its arm, weighted by how
    // near the point is to that road (on it, its own).
    let surface = |stations: &[Station], crown: f64, p: Point| -> (f64, f64) {
        let mut best = (f64::INFINITY, 0.0);
        for w in stations.windows(2) {
            let (a, b) = (w[0].at, w[1].at);
            let (dx, dz) = (b.0 - a.0, b.1 - a.1);
            let len2 = (dx * dx + dz * dz).max(1e-12);
            let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dz) / len2).clamp(0.0, 1.0);
            let on = w[0].lerp(&w[1], t);
            let off = distance(on.at, p);
            let beyond = (off - on.half).max(0.0);
            if beyond < best.0 || (beyond == 0.0 && best.0 == 0.0) {
                best = (beyond, on.height - crown * off.min(on.half));
            }
        }
        best
    };
    let height = |p: Point| {
        let (da, ha) = surface(&sa, ra.crown(), p);
        let (db, hb) = surface(&sb, rb.crown(), p);
        let (wa, wb) = (1.0 / (da + 0.05).powi(2), 1.0 / (db + 0.05).powi(2));
        (ha * wa + hb * wb) / (wa + wb)
    };
    let kind = if rank(ra.kind) >= rank(rb.kind) {
        ra.kind
    } else {
        rb.kind
    };
    Some(Corner {
        kind,
        points: outline.into_iter().map(|p| (p.0, p.1, height(p))).collect(),
    })
}

impl Corner {
    fn outline(&self) -> Vec<Point> {
        self.points.iter().map(|p| (p.0, p.1)).collect()
    }

    /// The surface height at `p` if it's inside (by the fan's triangles).
    fn height_at(&self, p: Point) -> Option<f64> {
        let e = self.points[0];
        for w in self.points[1..].windows(2) {
            let (b, c) = (w[0], w[1]);
            let det = (b.0 - e.0) * (c.1 - e.1) - (c.0 - e.0) * (b.1 - e.1);
            if det.abs() < 1e-12 {
                continue;
            }
            let u = ((p.0 - e.0) * (c.1 - e.1) - (c.0 - e.0) * (p.1 - e.1)) / det;
            let v = ((b.0 - e.0) * (p.1 - e.1) - (p.0 - e.0) * (b.1 - e.1)) / det;
            if u >= -1e-9 && v >= -1e-9 && u + v <= 1.0 + 1e-9 {
                return Some(e.2 + u * (b.2 - e.2) + v * (c.2 - e.2));
            }
        }
        None
    }

    /// The nearest point of the outline to `p`: its distance and the height there.
    fn nearest_edge(&self, p: Point) -> (f64, f64) {
        let n = self.points.len();
        let mut best = (f64::INFINITY, 0.0);
        for i in 0..n {
            let (a, b) = (self.points[i], self.points[(i + 1) % n]);
            let (dx, dz) = (b.0 - a.0, b.1 - a.1);
            let len2 = (dx * dx + dz * dz).max(1e-12);
            let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dz) / len2).clamp(0.0, 1.0);
            let d = (p.0 - a.0 - dx * t).hypot(p.1 - a.1 - dz * t);
            if d < best.0 {
                best = (d, a.2 + (b.2 - a.2) * t);
            }
        }
        best
    }

    /// Per grid vertex within `reach` of the corner: (index, distance beyond its outline
    /// (negative inside), the surface height there).
    pub fn near_vertices(&self, grid: &Grid, reach: f64) -> Vec<(usize, f64, f64)> {
        let outline = self.outline();
        let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
        for p in &outline {
            lo = (lo.0.min(p.0), lo.1.min(p.1));
            hi = (hi.0.max(p.0), hi.1.max(p.1));
        }
        let mut out = Vec::new();
        grid.each_vertex(
            (lo.0 - reach, lo.1 - reach),
            (hi.0 + reach, hi.1 + reach),
            |i, p| {
                let (d, edge_height) = self.nearest_edge(p);
                match self.height_at(p) {
                    Some(h) => out.push((i, -d, h)),
                    None if d <= reach => out.push((i, d, edge_height)),
                    None => {}
                }
            },
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_t_junction_gets_two_curbed_corners() {
        // A 6 m road along x, a 4 m one leaving it towards +z from its middle node.
        let mut main = Road::new(
            RoadKind::Asphalt,
            6.0,
            false,
            &[(1, (-50.0, 0.0)), (2, (0.0, 0.0)), (3, (50.0, 0.0))],
        );
        let mut side = Road::new(
            RoadKind::Gravel,
            4.0,
            false,
            &[(2, (0.0, 0.0)), (4, (0.0, 50.0))],
        );
        main.heights = vec![10.0; main.points.len()];
        side.heights = vec![10.0; side.points.len()];
        let corners = corners(&[main, side], &[]);
        assert_eq!(corners.len(), 2);
        for corner in &corners {
            // Asphalt, the better of the two; the street's radius (5 m) fits.
            assert!(corner.kind == RoadKind::Asphalt);
            let (e, rest) = corner.points.split_first().unwrap();
            // From the node, out along the main road's centreline to (±7, 0).
            assert!(e.0.abs() < 1e-9 && e.1.abs() < 1e-9);
            let x = rest
                .iter()
                .map(|p| p.0)
                .find(|x| x.abs() > 1e-9)
                .unwrap()
                .signum();
            assert!(
                rest.iter()
                    .any(|p| (p.0 - x * 7.0).abs() < 1e-9 && p.1.abs() < 1e-9)
            );
            // The arc runs from 5 m along the main road's edge to 5 m up the side road's.
            assert!(
                rest.iter()
                    .any(|p| (p.0.abs() - 7.0).abs() < 1e-9 && (p.1 - 3.0).abs() < 1e-9)
            );
            assert!(
                rest.iter()
                    .any(|p| (p.0.abs() - 2.0).abs() < 1e-9 && (p.1 - 8.0).abs() < 1e-9)
            );
            // All at the roads' height less the crown at their edges.
            assert!(corner.points.iter().all(|p| (p.2 - 10.0).abs() < 0.1));
            // The middle of the arc is inside, the corner's centre beyond the arc isn't.
            assert!(corner.height_at((x * 2.5, 3.5)).is_some());
            assert!(corner.height_at((x * 7.0, 8.0)).is_none());
        }
    }

    #[test]
    fn a_corner_on_a_curved_road_touches_its_edge() {
        // A 6 m road bending round a 30 m circle, a 4 m one leaving it outwards (+z) from
        // the middle of the bend, both level.
        let bend: Vec<(i64, Point)> = (-20..=20)
            .map(|k| {
                let a = k as f64 * 0.04;
                (100 + k, (30.0 * a.sin(), 30.0 - 30.0 * a.cos()))
            })
            .collect();
        let mut main = Road::new(RoadKind::Asphalt, 6.0, false, &bend);
        let mut side = Road::new(
            RoadKind::Asphalt,
            4.0,
            false,
            &[(100, (0.0, 0.0)), (200, (0.0, -40.0))],
        );
        main.heights = vec![10.0; main.points.len()];
        side.heights = vec![10.0; side.points.len()];
        let corners = corners(&[main, side], &[]);
        assert_eq!(corners.len(), 2);
        let off_main = |p: (f64, f64, f64)| (30.0 - (p.0).hypot(p.1 - 30.0)).abs();
        for corner in &corners {
            // Every arc point is off both roads, the arc's ends on their edges.
            let arc: Vec<_> = corner
                .points
                .iter()
                .copied()
                .filter(|&p| off_main(p) > 3.0 - 0.01 && p.0.abs() > 2.0 - 0.01)
                .collect();
            assert!(arc.len() >= 3, "{:?}", corner.points);
            let on_main = arc
                .iter()
                .map(|&p| (off_main(p) - 3.0).abs())
                .fold(9.0, f64::min);
            let on_side = arc
                .iter()
                .map(|&p| (p.0.abs() - 2.0).abs())
                .fold(9.0, f64::min);
            assert!(on_main < 0.01 && on_side < 0.01, "{on_main} {on_side}");
        }
    }
}
