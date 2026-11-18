//! Roads nudged off buildings: mapped centrelines are often a metre or two off, and our
//! widths are guesses, so roads run into houses. Each road is moved sideways (by at most
//! `MAX_NUDGE`, tapering smoothly along it) until its edges clear the walls by
//! `CLEARANCE`. Points it shares with other roads stay where they are, so roads still
//! meet. What can't be cleared that way (too far, or the gap between the houses narrower
//! than the road) is returned as conflicts, to fix in the map.

use std::collections::{HashMap, HashSet};

use crate::Point;
use crate::roads::{Road, STEP};

/// Road edges stay this far from walls (m).
const CLEARANCE: f64 = 0.3;
/// The farthest a road is moved sideways (m).
const MAX_NUDGE: f64 = 2.0;
/// A move tapers off along the road over this many metres per metre moved, and at least
/// `MIN_TAPER` metres.
const TAPER: f64 = 12.0;
const MIN_TAPER: f64 = 8.0;
/// Moves fade out over this far towards points shared with other roads (m).
const PIN_TAPER: f64 = 10.0;
/// Each round moves the road where it still runs into something after the last.
const ROUNDS: usize = 3;
/// The road's surface itself (without the clearance) reaching less than this into a
/// building isn't a conflict (m).
const TOLERANCE: f64 = 0.1;
/// Cells of the building index (m).
const CELL: f64 = 16.0;

/// A road running into a building.
pub struct Conflict {
    pub road: usize,
    pub building: usize,
    /// The road's centre where it reaches in farthest.
    pub at: Point,
    /// How far the road's surface reaches into the building (m).
    pub overlap: f64,
    /// The room between buildings on either side, where it's narrower than the road (m).
    pub gap: Option<f64>,
    /// Whether it's near a point shared with another road, where it can't move.
    pub junction: bool,
}

/// Which buildings reach into which cells.
struct Index(HashMap<(i32, i32), Vec<usize>>);

impl Index {
    fn new(buildings: &[&[Point]]) -> Self {
        let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (b, outline) in buildings.iter().enumerate() {
            let (lo, hi) = bounds(outline);
            for cz in cell(lo.1)..=cell(hi.1) {
                for cx in cell(lo.0)..=cell(hi.0) {
                    cells.entry((cx, cz)).or_default().push(b);
                }
            }
        }
        Self(cells)
    }

    /// The buildings that may reach within `reach` of `p`, into `out`.
    fn near(&self, p: Point, reach: f64, out: &mut Vec<usize>) {
        out.clear();
        for cz in cell(p.1 - reach)..=cell(p.1 + reach) {
            for cx in cell(p.0 - reach)..=cell(p.0 + reach) {
                if let Some(here) = self.0.get(&(cx, cz)) {
                    out.extend(here);
                }
            }
        }
        out.sort_unstable();
        out.dedup();
    }
}

fn cell(v: f64) -> i32 {
    (v / CELL).floor() as i32
}

fn bounds(points: &[Point]) -> (Point, Point) {
    points.iter().fold(
        ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN)),
        |(lo, hi), p| {
            (
                (lo.0.min(p.0), lo.1.min(p.1)),
                (hi.0.max(p.0), hi.1.max(p.1)),
            )
        },
    )
}

/// One building across the road: from `lo` to `hi` metres right of the road's centre.
#[derive(Clone, Copy)]
struct Across {
    lo: f64,
    hi: f64,
    building: usize,
}

/// The buildings across a road at `p` (heading `t`, a unit vector), out to `reach` either
/// side: where the line across through `p` is inside them, and their corners within half
/// a step along (so one jutting in between two points counts too).
fn cross_section(
    buildings: &[&[Point]],
    index: &Index,
    p: Point,
    t: Point,
    reach: f64,
    near: &mut Vec<usize>,
    out: &mut Vec<Across>,
) {
    out.clear();
    index.near(p, reach + STEP, near);
    let mut crossings = Vec::new();
    for &b in near.iter() {
        // In road coordinates: along and right.
        let local: Vec<Point> = buildings[b]
            .iter()
            .map(|q| {
                let (dx, dz) = (q.0 - p.0, q.1 - p.1);
                (dx * t.0 + dz * t.1, dx * -t.1 + dz * t.0)
            })
            .collect();
        crossings.clear();
        for (i, &(sa, ua)) in local.iter().enumerate() {
            let (sb, ub) = local[(i + 1) % local.len()];
            if (sa > 0.0) != (sb > 0.0) {
                crossings.push(ua + (ub - ua) * -sa / (sb - sa));
            }
            if sa.abs() <= STEP / 2.0 {
                out.push(Across {
                    lo: ua,
                    hi: ua,
                    building: b,
                });
            }
        }
        crossings.sort_by(f64::total_cmp);
        for pair in crossings.as_chunks::<2>().0 {
            out.push(Across {
                lo: pair[0],
                hi: pair[1],
                building: b,
            });
        }
    }
    out.retain(|a| a.hi >= -reach && a.lo <= reach);
}

/// How far the road (`half` its width plus clearance, either side) must move right to
/// clear everything `across` it: `Ok` if it can within `MAX_NUDGE`, else `Err` with the
/// best it can do.
fn shift_needed(across: &[Across], half: f64) -> Result<f64, f64> {
    let clear = |s: f64| across.iter().all(|a| s + half <= a.lo || s - half >= a.hi);
    let candidates = std::iter::once(0.0)
        .chain(across.iter().flat_map(|a| [a.lo - half, a.hi + half]))
        .filter(|&s| clear(s));
    let best = candidates.min_by(|a: &f64, b| a.abs().total_cmp(&b.abs()));
    match best {
        Some(s) if s.abs() <= MAX_NUDGE => Ok(s),
        _ => {
            // Squeezed between houses: the middle of the gap; else as far as allowed.
            let (left, right) = gap(across);
            let s = if right - left < 2.0 * half || best.is_none() {
                (left + right) / 2.0
            } else {
                best.unwrap_or(0.0)
            };
            Err(s.clamp(-MAX_NUDGE, MAX_NUDGE))
        }
    }
}

/// The free stretch across the road around its centre: the nearest building edges left
/// and right (by which side of the centre each building's middle is).
fn gap(across: &[Across]) -> (f64, f64) {
    let mut gap = (-f64::INFINITY, f64::INFINITY);
    for a in across {
        if a.lo + a.hi < 0.0 {
            gap.0 = gap.0.max(a.hi);
        } else {
            gap.1 = gap.1.min(a.lo);
        }
    }
    gap
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Unit headings along a road.
fn headings(points: &[Point]) -> Vec<Point> {
    (0..points.len())
        .map(|i| {
            let (a, b) = (
                points[i.saturating_sub(1)],
                points[(i + 1).min(points.len() - 1)],
            );
            let length = (b.0 - a.0).hypot(b.1 - a.1).max(1e-9);
            ((b.0 - a.0) / length, (b.1 - a.1) / length)
        })
        .collect()
}

/// Moves the drivable roads that aren't bridges or tunnels (`skip`) off the buildings
/// (outlines, not closed) and returns where they still run into one, and how many roads
/// moved and how far at most.
pub fn nudge(
    roads: &mut [Road],
    skip: &[bool],
    buildings: &[&[Point]],
) -> (Vec<Conflict>, usize, f64) {
    let index = Index::new(buildings);
    // Points shared with another road (or the same one twice) stay.
    let mut seen: HashMap<i64, usize> = HashMap::new();
    for road in roads.iter().filter(|r| r.drivable()) {
        for node in road.nodes.iter().flatten() {
            *seen.entry(*node).or_default() += 1;
        }
    }
    let pinned: HashSet<i64> = seen.into_iter().filter(|e| e.1 > 1).map(|e| e.0).collect();
    let (mut near, mut across) = (Vec::new(), Vec::new());
    let (mut moved, mut farthest) = (0, 0.0_f64);
    let mut conflicts = Vec::new();
    for (r, road) in roads.iter_mut().enumerate() {
        if !road.drivable() || skip[r] {
            continue;
        }
        let original = road.points.clone();
        let heading = headings(&original);
        let mut along = vec![0.0];
        for w in original.windows(2) {
            along.push(along.last().unwrap() + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
        }
        // How much of a move each point takes: none at shared points.
        let pins: Vec<f64> = road
            .nodes
            .iter()
            .zip(&along)
            .filter(|(n, _)| n.is_some_and(|n| pinned.contains(&n)))
            .map(|(_, &s)| s)
            .collect();
        let freedom: Vec<f64> = along
            .iter()
            .map(|&s| {
                pins.iter()
                    .map(|&p| smoothstep((s - p).abs() / PIN_TAPER))
                    .fold(1.0, f64::min)
            })
            .collect();
        let half = road.width / 2.0 + CLEARANCE;
        let reach = half + MAX_NUDGE * 2.0;
        // The moves wanted, and those made (less near shared points).
        let (mut raw, mut shift) = (vec![0.0; original.len()], vec![0.0; original.len()]);
        for _ in 0..ROUNDS {
            let mut wanted = vec![0.0; original.len()];
            for (i, &p) in road.points.iter().enumerate() {
                cross_section(
                    buildings,
                    &index,
                    p,
                    heading[i],
                    reach,
                    &mut near,
                    &mut across,
                );
                wanted[i] = match shift_needed(&across, half) {
                    Ok(s) | Err(s) => s,
                };
            }
            if wanted.iter().all(|&s| s == 0.0) {
                break;
            }
            // Each wanted move, tapered along the road; the largest either way wins.
            let (mut right, mut left) = (vec![0.0_f64; wanted.len()], vec![0.0_f64; wanted.len()]);
            for (j, &s) in wanted.iter().enumerate().filter(|(_, s)| **s != 0.0) {
                let taper = (TAPER * s.abs()).max(MIN_TAPER);
                let mut reach_out = |i: usize| {
                    let gone = (along[i] - along[j]).abs();
                    if gone >= taper {
                        return false;
                    }
                    let s = s * smoothstep(1.0 - gone / taper);
                    right[i] = right[i].max(s);
                    left[i] = left[i].min(s);
                    true
                };
                for i in j..original.len() {
                    if !reach_out(i) {
                        break;
                    }
                }
                for i in (0..j).rev() {
                    if !reach_out(i) {
                        break;
                    }
                }
            }
            for i in 0..original.len() {
                raw[i] = (raw[i] + right[i] + left[i]).clamp(-MAX_NUDGE, MAX_NUDGE);
                shift[i] = raw[i] * freedom[i];
                let (p, t) = (original[i], heading[i]);
                road.points[i] = (p.0 - t.1 * shift[i], p.1 + t.0 * shift[i]);
            }
        }
        let most = shift.iter().fold(0.0_f64, |m, s| m.max(s.abs()));
        if most > 0.0 {
            moved += 1;
            farthest = farthest.max(most);
        }
        // What's left: the road's own surface in buildings, the deepest point of each.
        let mut worst: HashMap<usize, Conflict> = HashMap::new();
        let half = road.width / 2.0;
        for (i, &p) in road.points.iter().enumerate() {
            cross_section(
                buildings,
                &index,
                p,
                heading[i],
                road.width,
                &mut near,
                &mut across,
            );
            let (left, right) = gap(&across);
            let narrow = right - left < road.width && right > left;
            for a in across.iter().filter(|a| a.hi > -half && a.lo < half) {
                let overlap = (half - a.lo).min(a.hi + half).min(road.width);
                if overlap < TOLERANCE {
                    continue;
                }
                let conflict = Conflict {
                    road: r,
                    building: a.building,
                    at: p,
                    overlap,
                    gap: narrow.then_some(right - left),
                    junction: freedom[i] < 0.99,
                };
                match worst.get(&a.building) {
                    Some(c) if c.overlap >= overlap => {}
                    _ => {
                        worst.insert(a.building, conflict);
                    }
                }
            }
        }
        let mut worst: Vec<Conflict> = worst.into_values().collect();
        worst.sort_by_key(|c| c.building);
        conflicts.extend(worst);
    }
    (conflicts, moved, farthest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::roads::RoadKind;

    fn straight_road(width: f64) -> Road {
        let mapped: Vec<(i64, Point)> = (0..=10).map(|i| (i, (i as f64 * 10.0, 0.0))).collect();
        Road::new(RoadKind::Asphalt, width, false, &mapped)
    }

    fn square(x: f64, z: f64, side: f64) -> Vec<Point> {
        vec![(x, z), (x + side, z), (x + side, z + side), (x, z + side)]
    }

    #[test]
    fn a_road_moves_off_a_house_smoothly() {
        // A 5 m road along z = 0, a house from z = 1.5 (right of the road heading +x is
        // +z), so 1 m into it and 1.3 m with the clearance.
        let mut roads = vec![straight_road(5.0)];
        let house = square(45.0, 1.5, 10.0);
        let (conflicts, moved, farthest) = nudge(&mut roads, &[false], &[&house]);
        assert!(conflicts.is_empty(), "{} left", conflicts.len());
        assert_eq!(moved, 1);
        assert!((farthest - 1.3).abs() < 0.05, "moved {farthest} m");
        let points = &roads[0].points;
        let at = |x: f64| points.iter().find(|p| (p.0 - x).abs() < 0.5).unwrap().1;
        assert!(at(50.0) <= -1.29, "at the house: {}", at(50.0));
        // Back on its line well away, and never a sharp kink.
        assert!(at(10.0).abs() < 1e-9 && at(90.0).abs() < 1e-9);
        for w in points.windows(2) {
            assert!((w[1].1 - w[0].1).abs() / (w[1].0 - w[0].0) < 0.2);
        }
    }

    #[test]
    fn a_road_between_houses_too_close_is_a_conflict() {
        // 4 m between the houses, a 5 m road: centred, half a metre into each.
        let mut roads = vec![straight_road(5.0)];
        let (a, b) = (square(45.0, 2.0, 10.0), square(45.0, -12.0, 10.0));
        let (conflicts, _, _) = nudge(&mut roads, &[false], &[&a, &b]);
        assert_eq!(conflicts.len(), 2);
        for c in &conflicts {
            assert!((c.overlap - 0.5).abs() < 0.05, "overlap {}", c.overlap);
            assert!((c.gap.unwrap() - 4.0).abs() < 1e-6);
        }
    }

    #[test]
    fn shared_points_stay() {
        // The house is at the end, which another road shares: the road can't move there.
        let mut roads = vec![straight_road(5.0), straight_road(5.0)];
        roads[1].points = vec![(100.0, 0.0), (100.0, -10.0)];
        roads[1].nodes = vec![Some(10), None];
        let house = square(95.0, 1.5, 10.0);
        let (conflicts, _, _) = nudge(&mut roads, &[false, true], &[&house]);
        assert_eq!(roads[0].points.last(), Some(&(100.0, 0.0)));
        assert_eq!(conflicts.len(), 1);
    }
}
