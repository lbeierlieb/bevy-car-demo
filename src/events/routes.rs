//! A race's route on the map: from a few points it passes through (the server's list),
//! along the roads between them, the shortest way (tracks and narrow lanes counted
//! longer, so a route keeps to proper roads unless its points say otherwise).
//!
//! The roads are a graph like the traffic's network, but of every road (narrow ones and
//! tracks too, not streams): stretches between the points where roads meet. A route's
//! points are put on the nearest road; between two of them, Dijkstra from the first one's
//! stretch to the second one's.

use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap};

use bevy::prelude::*;

use crate::terrain::{RoadKind, Terrain};

/// A route's point must be this near a road (m).
const SNAP: f32 = 30.0;
/// What a metre counts for finding the way: on unpaved roads, on narrow ones (under
/// `NARROW` m).
const UNPAVED_COST: f32 = 3.0;
const NARROW: f32 = 3.5;
const NARROW_COST: f32 = 1.5;
/// A route turning back on itself at one of its points (its way in and out, over this
/// length, the way out against the way in by more than `TURN_BACK` in cosine) is refused:
/// a point past a junction, or on a spur.
const TURN_LOOK: f32 = 15.0;
const TURN_BACK: f32 = -0.7;
/// The line sent is simplified to within this of the roads' centrelines (m).
const SIMPLIFY: f32 = 0.3;

/// A route along the roads: its line, the road's width at each point, and each of the
/// points it was asked to pass through: its index in the line.
pub struct RouteLine {
    pub line: Vec<Vec2>,
    pub widths: Vec<f32>,
    pub marks: Vec<u16>,
}

/// A stretch of road between two junctions.
struct Stretch {
    ends: [u32; 2],
    points: Vec<Vec2>,
    /// Distance along it to each point (m).
    along: Vec<f32>,
    width: f32,
    /// A metre of it counts this much.
    cost: f32,
}

impl Stretch {
    fn length(&self) -> f32 {
        *self.along.last().unwrap()
    }

    /// Its points from `from` to `to` metres along it (either way round), both ends
    /// included.
    fn piece(&self, from: f32, to: f32) -> Vec<Vec2> {
        let point = |s: f32| {
            let i = (self.along.partition_point(|&a| a <= s)).clamp(1, self.points.len() - 1) - 1;
            let span = (self.along[i + 1] - self.along[i]).max(1e-6);
            let t = ((s - self.along[i]) / span).clamp(0.0, 1.0);
            self.points[i].lerp(self.points[i + 1], t)
        };
        let (lo, hi) = (from.min(to), from.max(to));
        let mut points = vec![point(lo)];
        for (&p, &a) in self.points.iter().zip(&self.along) {
            if a > lo && a < hi {
                points.push(p);
            }
        }
        points.push(point(hi));
        if from > to {
            points.reverse();
        }
        points
    }
}

/// Every road (but streams) as a graph.
pub struct Roads {
    stretches: Vec<Stretch>,
    /// The stretches at each node.
    nodes: Vec<Vec<u32>>,
}

/// A point on a stretch: which, and how far along it (m).
#[derive(Clone, Copy, Debug)]
struct OnRoad {
    stretch: u32,
    s: f32,
}

#[derive(PartialEq)]
struct Queued(f32, u32);

impl Eq for Queued {}

impl Ord for Queued {
    fn cmp(&self, other: &Self) -> Ordering {
        other.0.total_cmp(&self.0)
    }
}

impl PartialOrd for Queued {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Roads {
    pub fn new(terrain: &Terrain) -> Self {
        let roads: Vec<(&crate::terrain::Road, Vec<Vec2>)> = (terrain.roads.iter())
            .filter(|road| road.kind != RoadKind::Water)
            .map(|road| {
                let mut points = road.points.clone();
                points.dedup();
                (road, points)
            })
            .filter(|(_, points)| points.len() >= 2)
            .collect();
        // Points on more than one road, or used twice, and every road's ends are nodes.
        let key = |p: Vec2| ((p.x * 100.0).round() as i32, (p.y * 100.0).round() as i32);
        let mut uses: HashMap<(i32, i32), u32> = HashMap::new();
        for (_, points) in &roads {
            for (i, &p) in points.iter().enumerate() {
                let end = i == 0 || i == points.len() - 1;
                *uses.entry(key(p)).or_default() += if end { 2 } else { 1 };
            }
        }
        let mut graph = Roads {
            stretches: Vec::new(),
            nodes: Vec::new(),
        };
        let mut node_of: HashMap<(i32, i32), u32> = HashMap::new();
        let mut node = |graph: &mut Roads, p: Vec2| {
            *node_of.entry(key(p)).or_insert_with(|| {
                graph.nodes.push(Vec::new());
                graph.nodes.len() as u32 - 1
            })
        };
        for (road, points) in &roads {
            let mut cost = 1.0;
            if !road.kind.paved() {
                cost *= UNPAVED_COST;
            }
            if road.width < NARROW {
                cost *= NARROW_COST;
            }
            let mut start = 0;
            for i in 1..points.len() {
                if uses[&key(points[i])] < 2 {
                    continue;
                }
                let stretch = points[start..=i].to_vec();
                start = i;
                let mut along = vec![0.0];
                for pair in stretch.windows(2) {
                    along.push(along.last().unwrap() + pair[0].distance(pair[1]));
                }
                if *along.last().unwrap() < 0.1 {
                    continue;
                }
                let ends = [
                    node(&mut graph, stretch[0]),
                    node(&mut graph, stretch[stretch.len() - 1]),
                ];
                let index = graph.stretches.len() as u32;
                graph.nodes[ends[0] as usize].push(index);
                if ends[1] != ends[0] {
                    graph.nodes[ends[1] as usize].push(index);
                }
                graph.stretches.push(Stretch {
                    ends,
                    points: stretch,
                    along,
                    width: road.width,
                    cost,
                });
            }
        }
        graph
    }

    /// The point on a road nearest to `p`, and how far that is.
    fn snap(&self, p: Vec2) -> Option<(OnRoad, f32)> {
        let mut best: Option<(OnRoad, f32)> = None;
        for (index, stretch) in self.stretches.iter().enumerate() {
            for (i, pair) in stretch.points.windows(2).enumerate() {
                let (a, b) = (pair[0], pair[1]);
                let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-6)).clamp(0.0, 1.0);
                let distance = a.lerp(b, t).distance(p);
                if best.is_none_or(|(_, d)| distance < d) {
                    let s = stretch.along[i] + t * (stretch.along[i + 1] - stretch.along[i]);
                    best = Some((
                        OnRoad {
                            stretch: index as u32,
                            s,
                        },
                        distance,
                    ));
                }
            }
        }
        best
    }

    /// The cheapest way along the roads from `a` to `b`: its points, and the road's width
    /// at each.
    fn way(&self, a: OnRoad, b: OnRoad) -> Option<(Vec<Vec2>, Vec<f32>)> {
        let sa = &self.stretches[a.stretch as usize];
        let sb = &self.stretches[b.stretch as usize];
        // Straight along the one stretch both are on.
        let direct = (a.stretch == b.stretch).then(|| (a.s - b.s).abs() * sa.cost);
        // Through the nodes: from either end of a's stretch to either end of b's.
        let mut cost = vec![f32::INFINITY; self.nodes.len()];
        let mut came: Vec<Option<(u32, u32)>> = vec![None; self.nodes.len()];
        let mut queue = BinaryHeap::new();
        for (end, to_end) in [(0, a.s), (1, sa.length() - a.s)] {
            let node = sa.ends[end];
            let c = to_end * sa.cost;
            if c < cost[node as usize] {
                cost[node as usize] = c;
                queue.push(Queued(c, node));
            }
        }
        let goal = |node: u32| {
            (0..2)
                .filter(|&end| sb.ends[end] == node)
                .map(|end| {
                    let s = if end == 0 { b.s } else { sb.length() - b.s };
                    s * sb.cost
                })
                .fold(f32::INFINITY, f32::min)
        };
        let mut best: Option<(f32, u32)> = None;
        while let Some(Queued(c, node)) = queue.pop() {
            if c > cost[node as usize] || best.is_some_and(|(b, _)| c >= b) {
                continue;
            }
            let total = c + goal(node);
            if best.is_none_or(|(b, _)| total < b) {
                best = Some((total, node));
            }
            for &index in &self.nodes[node as usize] {
                let stretch = &self.stretches[index as usize];
                let other = if stretch.ends[0] == node {
                    stretch.ends[1]
                } else {
                    stretch.ends[0]
                };
                let next = c + stretch.length() * stretch.cost;
                if next < cost[other as usize] {
                    cost[other as usize] = next;
                    came[other as usize] = Some((node, index));
                    queue.push(Queued(next, other));
                }
            }
        }
        let width_of = |stretch: &Stretch, n: usize| vec![stretch.width; n];
        if let Some(direct) = direct
            && best.is_none_or(|(b, _)| direct <= b)
        {
            let points = sa.piece(a.s, b.s);
            let widths = width_of(sa, points.len());
            return Some((points, widths));
        }
        let (_, last) = best?;
        // Back from the last node to the first, then forwards.
        let mut chain = Vec::new();
        let mut node = last;
        while let Some((before, index)) = came[node as usize] {
            chain.push((before, index, node));
            node = before;
        }
        chain.reverse();
        let first = node;
        let mut points = Vec::new();
        let mut widths = Vec::new();
        let mut add = |piece: Vec<Vec2>, width: f32| {
            for p in piece {
                if points.last() != Some(&p) {
                    points.push(p);
                    widths.push(width);
                }
            }
        };
        // a to the first node.
        let to = if sa.ends[0] == first {
            0.0
        } else {
            sa.length()
        };
        add(sa.piece(a.s, to), sa.width);
        for (from, index, _) in chain {
            let stretch = &self.stretches[index as usize];
            let (s0, s1) = if stretch.ends[0] == from {
                (0.0, stretch.length())
            } else {
                (stretch.length(), 0.0)
            };
            add(stretch.piece(s0, s1), stretch.width);
        }
        // The last node to b.
        let from = if sb.ends[0] == last { 0.0 } else { sb.length() };
        add(sb.piece(from, b.s), sb.width);
        Some((points, widths))
    }

    /// The route through `via` along the roads (back to the first point if `looped`): its
    /// line (a loop's last point is its first), the road's width at each point, and where
    /// each of `via` is in the line (a loop's first again at the end).
    pub fn route(&self, via: &[Vec2], looped: bool) -> Result<RouteLine, String> {
        if via.len() < 2 {
            return Err("needs at least two points".into());
        }
        let snapped: Vec<OnRoad> = (via.iter().enumerate())
            .map(|(i, &p)| match self.snap(p) {
                Some((on, distance)) if distance <= SNAP => Ok(on),
                Some((_, distance)) => Err(format!(
                    "checkpoint {} ({:.0} {:.0}) is {distance:.0} m from the nearest road",
                    i + 1,
                    p.x,
                    p.y
                )),
                None => Err("there are no roads".into()),
            })
            .collect::<Result<_, _>>()?;
        let mut legs: Vec<(usize, usize)> = (0..via.len() - 1).map(|i| (i, i + 1)).collect();
        if looped {
            legs.push((via.len() - 1, 0));
        }
        let mut line: Vec<Vec2> = Vec::new();
        let mut widths = Vec::new();
        // Where each point is in the line.
        let mut at_point = vec![0];
        for (i, j) in legs {
            let (points, w) = self.way(snapped[i], snapped[j]).ok_or_else(|| {
                format!(
                    "no way along the roads from checkpoint {} to {}",
                    i + 1,
                    j + 1
                )
            })?;
            for (p, w) in points.into_iter().zip(w) {
                if line.last() != Some(&p) {
                    line.push(p);
                    widths.push(w);
                }
            }
            at_point.push(line.len() - 1);
        }
        // No turning back at a point.
        let turns: Vec<usize> = if looped {
            (0..via.len()).collect()
        } else {
            (1..via.len() - 1).collect()
        };
        for k in turns {
            let i = at_point[k];
            let (before, after) = if looped && k == 0 {
                (line.len() - 1, 0)
            } else {
                (i, i)
            };
            // The way in, and the way out.
            let came = direction(&line, before, -1).map(|back| -back);
            let ahead = direction(&line, after, 1);
            if let (Some(came), Some(ahead)) = (came, ahead)
                && came.dot(ahead) < TURN_BACK
            {
                return Err(format!(
                    "the route turns back at checkpoint {} ({:.0} {:.0}): move it, or add one to say \
                     which way",
                    k + 1,
                    via[k].x,
                    via[k].y
                ));
            }
        }
        let keep = simplify(&line, SIMPLIFY, &at_point);
        if keep.len() > usize::from(u16::MAX) {
            return Err("too long".into());
        }
        let marks = (at_point.iter())
            .map(|&i| keep.partition_point(|&k| k < i) as u16)
            .collect();
        let line: Vec<Vec2> = keep.iter().map(|&i| line[i]).collect();
        let widths = keep.iter().map(|&i| widths[i]).collect();
        Ok(RouteLine {
            line,
            widths,
            marks,
        })
    }
}

/// The way the line goes from point `i`, forwards (`step` 1) or where it came from (−1),
/// over `TURN_LOOK`.
fn direction(line: &[Vec2], i: usize, step: isize) -> Option<Vec2> {
    let from = line[i];
    let mut j = i as isize;
    loop {
        j += step;
        let p = *line.get(usize::try_from(j).ok()?)?;
        if p.distance(from) >= TURN_LOOK {
            return Some((p - from).normalize());
        }
    }
}

/// The points of `line` to keep for it to stay within `tolerance` (Douglas–Peucker),
/// `kept` among them.
fn simplify(line: &[Vec2], tolerance: f32, kept: &[usize]) -> Vec<usize> {
    let mut keep = vec![false; line.len()];
    keep[0] = true;
    keep[line.len() - 1] = true;
    for &i in kept {
        keep[i] = true;
    }
    let fixed: Vec<usize> = (0..line.len()).filter(|&i| keep[i]).collect();
    let mut stack: Vec<(usize, usize)> = fixed.windows(2).map(|w| (w[0], w[1])).collect();
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = (line[a], line[b]);
        let far = (a + 1..b)
            .map(|i| {
                let p = line[i];
                let t =
                    ((p - pa).dot(pb - pa) / (pb - pa).length_squared().max(1e-6)).clamp(0.0, 1.0);
                (i, pa.lerp(pb, t).distance(p))
            })
            .max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((i, distance)) = far
            && distance > tolerance
        {
            keep[i] = true;
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    (0..line.len()).filter(|&i| keep[i]).collect()
}
