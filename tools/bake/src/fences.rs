//! Fences, walls, hedges and guard rails from the map's `barrier` ways, with openings
//! where gates are on them (gates are always open).

use std::collections::HashMap;

use crate::Point;
use crate::Tags;
use crate::roads::Road;

/// How a barrier looks (the game draws each kind its own way).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FenceKind {
    /// Wooden posts and two rails.
    Rails = 0,
    /// Thin metal posts and rails (railings, mesh and wire fences).
    Metal = 1,
    /// Closed boards or panels.
    Panel = 2,
    Wall = 3,
    Hedge = 4,
    GuardRail = 5,
}

pub struct Fence {
    pub way: i64,
    pub kind: FenceKind,
    /// Height above the ground (m).
    pub height: f64,
    pub points: Vec<Point>,
}

/// The kind and height of a barrier way, or `None` if it isn't one we build (retaining
/// walls, handrails, gates mapped as ways…). `height` in metres overrides the default.
pub fn style(tags: &Tags) -> Option<(FenceKind, f64)> {
    let tag = |k: &str| tags.get(k).map(String::as_str);
    let (kind, height) = match tag("barrier")? {
        "fence" => match tag("fence_type") {
            Some(
                "chain_link" | "metal" | "metal_bars" | "railing" | "wire" | "barbed_wire"
                | "electric" | "mesh",
            ) => (FenceKind::Metal, 1.4),
            Some("panel" | "concrete" | "board" | "palisade" | "plastic" | "brick") => {
                (FenceKind::Panel, 1.8)
            }
            _ => (FenceKind::Rails, 1.2),
        },
        "railing" => (FenceKind::Metal, 1.0),
        "wall" | "city_wall" => (FenceKind::Wall, 1.5),
        "jersey_barrier" => (FenceKind::Wall, 0.8),
        "hedge" => (FenceKind::Hedge, 1.5),
        "guard_rail" => (FenceKind::GuardRail, 0.75),
        _ => return None,
    };
    let height = tag("height")
        .and_then(|v| v.trim_end_matches('m').trim().parse::<f64>().ok())
        .unwrap_or(height)
        .clamp(0.3, 6.0);
    Some((kind, height))
}

/// How wide an opening a node with these tags makes in a barrier it's on (`width` in
/// metres overrides the default), or `None` if it makes none.
pub fn opening(tags: &Tags) -> Option<f64> {
    let tag = |k: &str| tags.get(k).map(String::as_str);
    let default = match tag("barrier")? {
        "gate" | "lift_gate" | "swing_gate" | "sliding_gate" | "entrance" | "cattle_grid"
        | "hampshire_gate" => 3.5,
        "wicket_gate" | "kissing_gate" => 1.2,
        _ => return None,
    };
    Some(
        tag("width")
            .and_then(|v| v.trim_end_matches('m').trim().parse::<f64>().ok())
            .unwrap_or(default)
            .clamp(0.5, 20.0),
    )
}

/// The pieces of a barrier through `points`, each with the opening width of the gate
/// there (if any), with the openings cut out. Pieces shorter than 30 cm are dropped.
pub fn cut(points: &[(Point, Option<f64>)]) -> Vec<Vec<Point>> {
    if points.len() < 2 {
        return Vec::new();
    }
    let mut along = vec![0.0];
    for w in points.windows(2) {
        along.push(along.last().unwrap() + (w[1].0.0 - w[0].0.0).hypot(w[1].0.1 - w[0].0.1));
    }
    let total = *along.last().unwrap();
    // The stretches between openings.
    let mut kept = vec![(0.0, total)];
    for (&(_, width), &s) in points.iter().zip(&along) {
        let Some(width) = width else { continue };
        let (lo, hi) = (s - width / 2.0, s + width / 2.0);
        kept = kept
            .into_iter()
            .flat_map(|(a, b): (f64, f64)| [(a, b.min(lo)), (a.max(hi), b)])
            .filter(|(a, b)| b > a)
            .collect();
    }
    let at = |s: f64| {
        let i = along
            .partition_point(|&a| a <= s)
            .clamp(1, points.len() - 1);
        let (a, b) = (points[i - 1].0, points[i].0);
        let t = ((s - along[i - 1]) / (along[i] - along[i - 1]).max(1e-9)).clamp(0.0, 1.0);
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    };
    kept.into_iter()
        .filter(|(a, b)| b - a >= 0.3)
        .map(|(lo, hi)| {
            let mut piece = vec![at(lo)];
            piece.extend(
                points
                    .iter()
                    .zip(&along)
                    .filter(|(_, s)| **s > lo && **s < hi)
                    .map(|(p, _)| p.0),
            );
            piece.push(at(hi));
            piece.dedup();
            piece
        })
        .filter(|piece| piece.len() >= 2)
        .collect()
}

/// A stretch of a barrier standing on a road's surface.
pub struct OnRoad {
    pub fence: usize,
    pub road: usize,
    /// Its middle, and how long it is (m).
    pub at: Point,
    pub length: f64,
}

/// Barriers stay this far beyond the edges of roads they run along (m).
const CLEARANCE: f64 = 0.5;
/// A barrier runs along a road where their directions are within about 45°...
const ALONG: f64 = 0.7;
/// ...and is moved off it by at most this much (m).
const MAX_MOVE: f64 = 3.0;
/// Barriers are resampled this finely, so moved ones follow the road's bends (m).
const SPACING: f64 = 1.0;

/// The drivable roads that aren't off the ground, by the cells their segments reach.
struct RoadIndex<'a> {
    roads: &'a [Road],
    cells: HashMap<(i32, i32), Vec<(usize, usize)>>,
}

/// A point's place beside one road segment.
struct Beside {
    road: usize,
    /// The nearest point of the centreline, the road's direction there, and how far right
    /// of it the point is (m).
    foot: Point,
    direction: Point,
    right: f64,
    /// Half the road's width there.
    half: f64,
}

const CELL: f64 = 8.0;

fn cell(v: f64) -> i32 {
    (v / CELL).floor() as i32
}

impl<'a> RoadIndex<'a> {
    /// Segments are found up to `margin` beyond their road's edges.
    fn new(roads: &'a [Road], skip: &[bool], margin: f64) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<(usize, usize)>> = HashMap::new();
        for (r, road) in roads.iter().enumerate() {
            if !road.drivable() || skip[r] {
                continue;
            }
            let reach = road.widest() / 2.0 + margin;
            for (i, w) in road.points.windows(2).enumerate() {
                for cz in cell(w[0].1.min(w[1].1) - reach)..=cell(w[0].1.max(w[1].1) + reach) {
                    for cx in cell(w[0].0.min(w[1].0) - reach)..=cell(w[0].0.max(w[1].0) + reach) {
                        cells.entry((cx, cz)).or_default().push((r, i));
                    }
                }
            }
        }
        Self { roads, cells }
    }

    /// The segments `p` is within `margin` beyond the edge of (inside if negative).
    fn beside(&self, p: Point, margin: f64) -> impl Iterator<Item = Beside> + '_ {
        let here = self.cells.get(&(cell(p.0), cell(p.1)));
        here.into_iter().flatten().filter_map(move |&(r, i)| {
            let road = &self.roads[r];
            let ((x0, z0), (x1, z1)) = (road.points[i], road.points[i + 1]);
            let (dx, dz) = (x1 - x0, z1 - z0);
            let length = dx.hypot(dz).max(1e-9);
            let t = (((p.0 - x0) * dx + (p.1 - z0) * dz) / (length * length)).clamp(0.0, 1.0);
            let foot = (x0 + dx * t, z0 + dz * t);
            let off = (p.0 - foot.0).hypot(p.1 - foot.1);
            let direction = (dx / length, dz / length);
            let right = (p.0 - x0) * -direction.1 + (p.1 - z0) * direction.0;
            let half = (road.widths[i] + (road.widths[i + 1] - road.widths[i]) * t) / 2.0;
            (off < half + margin).then_some(Beside {
                road: r,
                half,
                foot,
                direction,
                right: if right.abs() < off * 0.5 {
                    off.copysign(right)
                } else {
                    right
                },
            })
        })
    }
}

/// Points along `points`, at most `SPACING` apart, through all of them.
fn resample(points: &[Point]) -> Vec<Point> {
    let mut out = vec![points[0]];
    for w in points.windows(2) {
        let length = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        let steps = (length / SPACING).ceil().max(1.0) as usize;
        for k in 1..=steps {
            let t = k as f64 / steps as f64;
            out.push((
                w[0].0 + (w[1].0 - w[0].0) * t,
                w[0].1 + (w[1].1 - w[0].1) * t,
            ));
        }
    }
    out
}

/// Moves barriers running along the drivable roads that aren't off the ground (`skip`)
/// out to `CLEARANCE` beyond their edges: mapped guard rails and fences are often a metre
/// off, and our road widths are guesses. Barriers crossing a road stay. Returns how many
/// were moved.
pub fn clear_roads(fences: &mut [Fence], roads: &[Road], skip: &[bool]) -> usize {
    let index = RoadIndex::new(roads, skip, CLEARANCE);
    let mut moved = 0;
    for fence in fences.iter_mut() {
        let points = resample(&fence.points);
        let n = points.len();
        let mut changed = false;
        let mut out = points.clone();
        for (i, p) in out.iter_mut().enumerate() {
            let (a, b) = (points[i.saturating_sub(1)], points[(i + 1).min(n - 1)]);
            let length = (b.0 - a.0).hypot(b.1 - a.1).max(1e-9);
            let heading = ((b.0 - a.0) / length, (b.1 - a.1) / length);
            // Twice, in case the first move puts it onto another road.
            for _ in 0..2 {
                let deepest = index
                    .beside(*p, CLEARANCE)
                    .filter(|s| {
                        (s.direction.0 * heading.0 + s.direction.1 * heading.1).abs() > ALONG
                    })
                    .map(|s| (s.half + CLEARANCE - s.right.abs(), s))
                    .filter(|(push, s)| *push <= MAX_MOVE && s.right != 0.0)
                    .max_by(|a, b| a.0.total_cmp(&b.0));
                let Some((_, s)) = deepest else { break };
                let out_dist = s.half + CLEARANCE;
                let side = s.right.signum() * out_dist;
                *p = (
                    s.foot.0 - s.direction.1 * side,
                    s.foot.1 + s.direction.0 * side,
                );
                changed = true;
            }
        }
        if changed {
            fence.points = out;
            moved += 1;
        }
    }
    moved
}

/// Bridges' parapets stand in for the barriers mapped along them: those stretches (within
/// `ON_BRIDGE` beyond a deck's edge, running along it) are dropped, or they'd stand on
/// the ground under the deck.
pub fn drop_on_bridges(fences: Vec<Fence>, roads: &[Road]) -> Vec<Fence> {
    const ON_BRIDGE: f64 = 1.5;
    let not_bridges: Vec<bool> = roads.iter().map(|r| !r.bridge).collect();
    let index = RoadIndex::new(roads, &not_bridges, ON_BRIDGE);
    let mut kept = Vec::new();
    for fence in fences {
        let points = resample(&fence.points);
        let n = points.len();
        let on: Vec<bool> = (0..n)
            .map(|i| {
                let (a, b) = (points[i.saturating_sub(1)], points[(i + 1).min(n - 1)]);
                let length = (b.0 - a.0).hypot(b.1 - a.1).max(1e-9);
                let heading = ((b.0 - a.0) / length, (b.1 - a.1) / length);
                index
                    .beside(points[i], ON_BRIDGE)
                    .any(|s| (s.direction.0 * heading.0 + s.direction.1 * heading.1).abs() > ALONG)
            })
            .collect();
        if !on.contains(&true) {
            kept.push(fence);
            continue;
        }
        let mut run: Vec<Point> = Vec::new();
        for (p, on) in points.into_iter().zip(on) {
            if !on {
                run.push(p);
            } else if run.len() >= 2 {
                kept.push(Fence {
                    points: std::mem::take(&mut run),
                    ..fence
                });
            } else {
                run.clear();
            }
        }
        if run.len() >= 2 {
            kept.push(Fence {
                points: run,
                ..fence
            });
        }
    }
    kept
}

/// Where barriers stand on the drivable roads that aren't off the ground (`skip`): more
/// than 10 cm inside their edges, checked every half metre.
pub fn on_roads(fences: &[Fence], roads: &[Road], skip: &[bool]) -> Vec<OnRoad> {
    const STEP: f64 = 0.5;
    const INSIDE: f64 = 0.1;
    let index = RoadIndex::new(roads, skip, 0.0);
    let road_at = |p: Point| index.beside(p, -INSIDE).next().map(|s| s.road);
    let mut found = Vec::new();
    for (f, fence) in fences.iter().enumerate() {
        // Runs of samples on the same road: (road, first sample, last sample, length).
        let mut run: Option<(usize, Point, Point, f64)> = None;
        let mut flush = |run: &mut Option<(usize, Point, Point, f64)>| {
            if let Some((road, a, b, length)) = run.take() {
                found.push(OnRoad {
                    fence: f,
                    road,
                    at: ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0),
                    length,
                });
            }
        };
        for w in fence.points.windows(2) {
            let length = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
            let steps = (length / STEP).ceil().max(1.0) as usize;
            for k in 0..steps {
                let t = (k as f64 + 0.5) / steps as f64;
                let p = (
                    w[0].0 + (w[1].0 - w[0].0) * t,
                    w[0].1 + (w[1].1 - w[0].1) * t,
                );
                match (road_at(p), run.as_mut()) {
                    (Some(r), Some(current)) if current.0 == r => {
                        current.2 = p;
                        current.3 += length / steps as f64;
                    }
                    (hit, _) => {
                        flush(&mut run);
                        run = hit.map(|r| (r, p, p, length / steps as f64));
                    }
                }
            }
        }
        flush(&mut run);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gates_open_barriers() {
        // A fence 20 m along x with a 3.5 m gate at 8 m (a node) and one at its end.
        let points = [
            ((0.0, 0.0), None),
            ((8.0, 0.0), Some(3.5)),
            ((20.0, 0.0), Some(4.0)),
        ];
        let pieces = cut(&points);
        assert_eq!(
            pieces,
            vec![
                vec![(0.0, 0.0), (6.25, 0.0)],
                vec![(9.75, 0.0), (18.0, 0.0)]
            ]
        );
        assert!(cut(&points[..1]).is_empty());
    }

    #[test]
    fn barriers_along_a_road_move_off_it_and_crossing_ones_stay() {
        let mapped: Vec<(i64, Point)> = vec![(1, (0.0, 0.0)), (2, (50.0, 0.0))];
        let roads = vec![Road::new(
            crate::roads::RoadKind::Asphalt,
            6.0,
            false,
            &mapped,
        )];
        let fence = |points| Fence {
            way: 1,
            kind: FenceKind::GuardRail,
            height: 0.75,
            points,
        };
        let mut fences = vec![
            fence(vec![(10.0, -2.5), (40.0, -2.0)]),
            fence(vec![(25.0, -5.0), (25.0, 5.0)]),
        ];
        assert_eq!(clear_roads(&mut fences, &roads, &[false]), 1);
        assert!(fences[0].points.iter().all(|p| (p.1 + 3.5).abs() < 1e-9));
        assert_eq!(fences[1].points.len(), 2);
        let found = on_roads(&fences, &roads, &[false]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].fence, 1);
    }

    #[test]
    fn barriers_on_a_road_are_found() {
        let mapped: Vec<(i64, Point)> = vec![(1, (0.0, 0.0)), (2, (50.0, 0.0))];
        let roads = vec![Road::new(
            crate::roads::RoadKind::Asphalt,
            6.0,
            false,
            &mapped,
        )];
        let fence = |points| Fence {
            way: 1,
            kind: FenceKind::GuardRail,
            height: 0.75,
            points,
        };
        // One along the edge (2.95 m out: on it by less than 10 cm), one 2 m in for
        // 10 m, one across.
        let fences = vec![
            fence(vec![(5.0, 2.95), (45.0, 2.95)]),
            fence(vec![(10.0, 1.0), (20.0, 1.0), (30.0, 6.0)]),
            fence(vec![(40.0, -5.0), (40.0, 5.0)]),
        ];
        let found = on_roads(&fences, &roads, &[false]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].fence, 1);
        assert!(found[0].length > 10.0 && found[0].length < 15.0);
        assert_eq!(found[1].fence, 2);
        assert!((found[1].length - 5.8).abs() < 0.3, "{}", found[1].length);
    }
}
