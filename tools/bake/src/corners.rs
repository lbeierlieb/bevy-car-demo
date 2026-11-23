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
/// The corner reaches this far in under each road's edge, so no seam opens between them
/// where the road bends (m).
const OVERLAP: f64 = 0.3;
/// Corners only between roads meeting at angles between these (radians): sharper ones
/// would need a long arc, flatter ones have nothing to fill.
const SHARPEST: f64 = 25.0 * std::f64::consts::PI / 180.0;
const FLATTEST: f64 = 160.0 * std::f64::consts::PI / 180.0;
/// Arc points at most this far apart in angle (radians).
const ARC_STEP: f64 = 10.0 * std::f64::consts::PI / 180.0;

/// A filled corner: a fan around its first point, the corner of the two edges (pulled in
/// under the roads by `OVERLAP`), through the arc. Points are (x, z, height).
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
                if let Some(corner) = corner(roads, a, b, theta, wanted * shrink)
                    && !corner.points.iter().any(|&(x, z, _)| {
                        outlines.iter().any(|o| crate::terrain::inside(o, (x, z)))
                    })
                {
                    corners.push(corner);
                    break;
                }
            }
        }
    }
    corners
}

/// The corner between arm `a` and the next arm `b` counterclockwise (`theta` apart), of
/// radius `wanted` or as much as the roads' lengths allow; `None` if too small.
fn corner(roads: &[Road], a: &Arm, b: &Arm, theta: f64, wanted: f64) -> Option<Corner> {
    let (ra, rb) = (&roads[a.road], &roads[b.road]);
    let node = ra.points[a.at];
    let (sin, cos) = theta.sin_cos();
    let (da, db) = (a.direction, b.direction);
    // Towards the space between them: left of `a`, right of `b`.
    let pa = (-da.1, da.0);
    let pb = (db.1, -db.0);
    // How far along each arm the edges' corner is, and the arc's tangent points for
    // radius r: linear in r.
    let along_a = |r: f64| (b.half + a.half * cos + r * (1.0 + cos)) / sin;
    let along_b = |r: f64| (a.half + b.half * cos + r * (1.0 + cos)) / sin;
    if along_a(0.0) < 0.0 || along_b(0.0) < 0.0 {
        return None;
    }
    // At most most of each arm's length.
    let most = |length: f64, half: f64, other: f64| {
        (0.8 * length * sin - other - half * cos) / (1.0 + cos)
    };
    let r = wanted
        .min(most(a.length, a.half, b.half))
        .min(most(b.length, b.half, a.half));
    if r < 0.5 {
        return None;
    }
    let at = |d: Point, p: Point, s: f64, off: f64| {
        (node.0 + d.0 * s + p.0 * off, node.1 + d.1 * s + p.1 * off)
    };
    let (ua, ub) = (along_a(r), along_b(r));
    let ta = at(da, pa, ua, a.half);
    let tb = at(db, pb, ub, b.half);
    let centre = at(da, pa, ua, a.half + r);
    // The corner of the two edges pulled in under the roads.
    let inner = |half: f64, other: f64| (other + half * cos) / sin;
    let (ha, hb) = (a.half - OVERLAP, b.half - OVERLAP);
    let e = at(da, pa, inner(ha, hb), ha);
    let mut outline = vec![e, at(da, pa, ua, ha), ta];
    let (start, end) = (
        (ta.1 - centre.1).atan2(ta.0 - centre.0),
        (tb.1 - centre.1).atan2(tb.0 - centre.0),
    );
    let mut sweep = end - start;
    while sweep > std::f64::consts::PI {
        sweep -= std::f64::consts::TAU;
    }
    while sweep < -std::f64::consts::PI {
        sweep += std::f64::consts::TAU;
    }
    let steps = (sweep.abs() / ARC_STEP).ceil().max(2.0) as usize;
    for s in 1..steps {
        let angle = start + sweep * s as f64 / steps as f64;
        outline.push((centre.0 + r * angle.cos(), centre.1 + r * angle.sin()));
    }
    outline.push(tb);
    outline.push(at(db, pb, ub, hb));
    // Heights: each road's edge height where the point is along it, weighted by how near
    // the point is to that road's edge.
    let height = |p: Point| {
        let side = |arm: &Arm, road: &Road, d: Point, q: Point| {
            let rel = (p.0 - node.0, p.1 - node.1);
            let s = (rel.0 * d.0 + rel.1 * d.1).max(0.0);
            let off = (rel.0 * q.0 + rel.1 * q.1 - arm.half).abs();
            let (_, centre) = arm.along(road, s);
            let edge = centre - road.crown() * arm.half;
            (edge, 1.0 / (off + 0.1))
        };
        let (h1, w1) = side(a, ra, da, pa);
        let (h2, w2) = side(b, rb, db, pb);
        (h1 * w1 + h2 * w2) / (w1 + w2)
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
            // The edges' corner pulled in under the roads, at (±(2 - 0.3), 3 - 0.3).
            assert!((e.0.abs() - 1.7).abs() < 1e-9 && (e.1 - 2.7).abs() < 1e-9);
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
            let x = e.0.signum();
            assert!(corner.height_at((x * 2.5, 3.5)).is_some());
            assert!(corner.height_at((x * 7.0, 8.0)).is_none());
        }
    }
}
