//! Races and time trials: an event's routes, the course a round drives (a route one way
//! or the other, so many laps), its checkpoints and starting grid, the host's settings,
//! how a race stands and how it ended.
//!
//! Stopping on a race's marker asks whether to drive a time trial (alone, in a round of
//! one, in a realm of one) or race (into the lobby there: the first one in opens it and
//! sets things). Running, everyone is put on a grid at the route's start and held for the
//! countdown; the clock runs from the green light. The course has gates (checkpoints)
//! along it, every `GATE_SPACING` at most; the server judges who passed which from the
//! cars' states, in order (a gate missed has to be gone back for). A loop's lap ends at
//! its start; a sprint ends at its far end. Once the first racer is through, the others
//! have `FINISH_GRACE` to finish. A time trial's times go on the server's list of best
//! times (for the session), per route, direction and traffic.

use std::cmp::Ordering;

use bevy::prelude::*;

use crate::net::protocol::PlayerId;
use crate::net::wire::{Reader, Wire};

/// The most racers in a round (the grid's length).
pub const MAX_RACERS: usize = 16;
/// The countdown before the green light (s).
pub const COUNTDOWN: f64 = 5.0;
/// Once the first racer is through, the others have this long (s).
pub const FINISH_GRACE: f64 = 60.0;
/// What the laps can be set to.
pub const LAPS: (u8, u8) = (1, 20);
/// Gates are at most this far apart along the course (m).
pub const GATE_SPACING: f32 = 150.0;
/// A gate reaches this far past the road's edge on either side (m): running wide counts.
const GATE_REACH: f32 = 6.0;
/// The grid: the back row this far along the course from its start, rows this far
/// apart, side by side on roads at least `GRID_PAIRS` wide (m), else one behind the other
/// `GRID_SINGLE` apart.
const GRID_FRONT: f32 = 12.0;
const GRID_ROW: f32 = 6.0;
const GRID_PAIRS: f32 = 5.0;
const GRID_SINGLE: f32 = 9.0;
/// The first gate is past the longest grid by this (m).
const FIRST_GATE_GAP: f32 = 40.0;
/// A route must be this long at least (m).
pub const SHORTEST: f32 = 500.0;
/// A car moving further than this between two of its states didn't drive through a gate
/// (it was put somewhere: m).
const JUMP: f32 = 60.0;
/// Best times kept per route, direction and traffic.
pub const BEST_KEPT: usize = 10;

/// Where the first gate is along the course (m).
fn first_gate() -> f32 {
    GRID_FRONT + (MAX_RACERS - 1) as f32 * GRID_SINGLE + FIRST_GATE_GAP
}

/// A route of a race event: as the server's list has it (`via`, the points it passes
/// through) and along the roads (`line`, with the road's width at each point; a loop's
/// last point is its first).
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub name: String,
    /// Round and round (laps), or from one end to the other.
    pub looped: bool,
    pub via: Vec<Vec2>,
    pub line: Vec<Vec2>,
    pub widths: Vec<f32>,
}

impl Route {
    /// Its length along the roads (one lap; m).
    pub fn length(&self) -> f32 {
        self.line.windows(2).map(|w| w[0].distance(w[1])).sum()
    }

    /// "Loop" or "Sprint", for people.
    pub fn kind(&self) -> &'static str {
        if self.looped { "Loop" } else { "Sprint" }
    }
}

/// A checkpoint: a line across the course at `s` metres along it, `half` either side of
/// `point`, crossed forwards (along `forward`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gate {
    pub s: f32,
    pub point: Vec2,
    pub forward: Vec2,
    /// The road's width there (m).
    pub width: f32,
    pub half: f32,
}

impl Gate {
    /// Whether a car going from `from` to `to` passed through it forwards.
    pub fn crossed(&self, from: Vec2, to: Vec2) -> bool {
        let (a, b) = (
            (from - self.point).dot(self.forward),
            (to - self.point).dot(self.forward),
        );
        if !(a < 0.0 && b >= 0.0) || from.distance(to) > JUMP {
            return false;
        }
        let at = from.lerp(to, a / (a - b));
        let side = Vec2::new(-self.forward.y, self.forward.x);
        (at - self.point).dot(side).abs() <= self.half
    }

    /// The yaw a car facing through it has.
    pub fn yaw(&self) -> f32 {
        yaw(self.forward)
    }
}

fn yaw(forward: Vec2) -> f32 {
    (-forward.x).atan2(-forward.y)
}

/// A route driven one way, so many laps: its line from the start, and its gates.
#[derive(Clone, Debug)]
pub struct Course {
    pub looped: bool,
    pub laps: u16,
    pub line: Vec<Vec2>,
    pub widths: Vec<f32>,
    /// Distance along the line to each point (m).
    pub at: Vec<f32>,
    /// One lap's gates, in order; the last is the finish (a loop's: its start).
    pub gates: Vec<Gate>,
}

impl Course {
    /// `route` one way (`reverse`: the other), `laps` times round if it's a loop.
    pub fn new(route: &Route, reverse: bool, laps: u8) -> Self {
        let (mut line, mut widths) = (route.line.clone(), route.widths.clone());
        if reverse {
            line.reverse();
            widths.reverse();
        }
        let mut at = vec![0.0];
        for pair in line.windows(2) {
            at.push(at.last().unwrap() + pair[0].distance(pair[1]));
        }
        let mut course = Self {
            looped: route.looped,
            laps: if route.looped {
                u16::from(laps.max(1))
            } else {
                1
            },
            line,
            widths,
            at,
            gates: Vec::new(),
        };
        let length = course.length();
        let first = first_gate().min(length * 0.5);
        let count = ((length - first) / GATE_SPACING).ceil().max(1.0) as usize;
        course.gates = (0..=count)
            .map(|i| {
                let s = first + (length - first) * i as f32 / count as f32;
                let (point, forward, width) = course.point(s);
                Gate {
                    s,
                    point,
                    forward,
                    width,
                    half: width / 2.0 + GATE_REACH,
                }
            })
            .collect();
        course
    }

    /// One lap's length (m).
    pub fn length(&self) -> f32 {
        *self.at.last().unwrap()
    }

    /// The gates to pass, all laps.
    pub fn total(&self) -> usize {
        self.gates.len() * usize::from(self.laps)
    }

    /// The gate after `passed` of them.
    pub fn next(&self, passed: usize) -> &Gate {
        &self.gates[passed % self.gates.len()]
    }

    /// Whether passing gate number `passed` (counting from 1) ends a lap.
    pub fn ends_lap(&self, passed: usize) -> bool {
        passed > 0 && passed.is_multiple_of(self.gates.len())
    }

    /// The point `s` along the line (clamped to it), the line's direction there and the
    /// road's width.
    pub fn point(&self, s: f32) -> (Vec2, Vec2, f32) {
        let n = self.line.len();
        let i = self.at.partition_point(|&a| a <= s).clamp(1, n - 1) - 1;
        let span = (self.at[i + 1] - self.at[i]).max(1e-6);
        let t = ((s - self.at[i]) / span).clamp(0.0, 1.0);
        let forward = (self.line[i + 1] - self.line[i]).normalize_or(Vec2::NEG_Y);
        let width = self.widths[i] + (self.widths[i + 1] - self.widths[i]) * t;
        (self.line[i].lerp(self.line[i + 1], t), forward, width)
    }

    /// Where the racer in `slot` (0: the front) of `count` starts, and the yaw they face.
    pub fn grid(&self, slot: usize, count: usize) -> (Vec2, f32) {
        let (_, _, width) = self.point(GRID_FRONT);
        let pairs = width >= GRID_PAIRS;
        let rows = |n: usize| if pairs { n / 2 } else { n };
        let gap = if pairs { GRID_ROW } else { GRID_SINGLE };
        // The back row at the front's place along the course, plus the rows ahead of it.
        let behind = rows(count.max(1) - 1) - rows(slot);
        let s = GRID_FRONT + behind as f32 * gap;
        let (point, forward, width) = self.point(s);
        let side = if pairs {
            let left = slot.is_multiple_of(2);
            let offset = (width / 4.0).min(1.6);
            if left { -offset } else { offset }
        } else {
            0.0
        };
        let right = Vec2::new(-forward.y, forward.x);
        (point + right * side, yaw(forward))
    }

    /// Where a racer who passed `passed` gates is put back (R): at the last gate they
    /// passed, in the right-hand half of the road, or on the grid at `slot` if none.
    pub fn reset_spot(&self, passed: usize, slot: usize, count: usize) -> (Vec2, f32) {
        if passed == 0 {
            return self.grid(slot, count);
        }
        let gate = self.next(passed - 1);
        let right = Vec2::new(-gate.forward.y, gate.forward.x);
        let lane = (gate.width / 4.0).min(2.0);
        (gate.point + right * lane, gate.yaw())
    }

    /// How far along one lap the point of the line nearest to `p` is, looking only within
    /// `from..to` metres along (m), and the line's direction there.
    pub fn nearest(&self, p: Vec2, from: f32, to: f32) -> Option<(f32, Vec2)> {
        (self.line.windows(2).zip(self.at.windows(2)))
            .filter(|(_, at)| at[1] >= from && at[0] <= to)
            .map(|(pair, at)| {
                let (a, b) = (pair[0], pair[1]);
                let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-6)).clamp(0.0, 1.0);
                let distance = a.lerp(b, t).distance(p);
                (
                    at[0] + t * (at[1] - at[0]),
                    (b - a).normalize_or(Vec2::NEG_Y),
                    distance,
                )
            })
            .min_by(|x, y| x.2.total_cmp(&y.2))
            .map(|(s, forward, _)| (s, forward))
    }
}

/// What the host sets (and a time trial's driver).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Which of the event's routes.
    pub route: u8,
    /// The other way round.
    pub reverse: bool,
    /// How many laps, on a loop.
    pub laps: u8,
    /// Whether there's traffic (the round's own).
    pub traffic: bool,
    /// Whether the racers' cars run into each other (else they're ghosts to each other,
    /// though not to traffic). Races only.
    pub collisions: bool,
    /// Whether R calls the rescue helicopter, which puts the car back at the last gate it
    /// passed (its flight is the penalty). Races only: a time trial is restarted instead.
    pub resets: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            route: 0,
            reverse: false,
            laps: 3,
            traffic: false,
            collisions: true,
            resets: true,
        }
    }
}

impl Settings {
    /// Within what can be set, for an event with `routes` routes.
    pub fn clamped(self, routes: usize) -> Self {
        Self {
            route: self.route.min(routes.saturating_sub(1).min(255) as u8),
            laps: self.laps.clamp(LAPS.0, LAPS.1),
            ..self
        }
    }
}

/// A racer's way through a round under way.
#[derive(Clone, Debug, PartialEq)]
pub struct Racer {
    pub player: PlayerId,
    /// As they were called.
    pub name: String,
    /// The gates passed, all laps.
    pub gates: u16,
    /// When they passed the last of them (s, the server's clock; the green light's time
    /// before the first).
    pub at: f64,
    /// Their laps' times, those done (a sprint's one is the whole way; s).
    pub laps: Vec<f32>,
    /// Their time from the green light to the finish, once through (s).
    pub finished: Option<f32>,
    /// They left the round.
    pub left: bool,
}

impl Racer {
    pub fn best_lap(&self) -> Option<f32> {
        self.laps.iter().copied().min_by(f32::total_cmp)
    }
}

/// How a race (or time trial) under way stands.
#[derive(Clone, Debug, PartialEq)]
pub struct Standing {
    /// In the order they came into the round.
    pub racers: Vec<Racer>,
    /// When the round ends, once the first racer is through (s, the server's clock).
    pub closes: Option<f64>,
}

impl Standing {
    /// The racers in the order they're placed: those through by their time, then by the
    /// gates passed and who passed the last one first; those who left last.
    pub fn order(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.racers.len()).collect();
        order.sort_by(|&a, &b| {
            let (a, b) = (&self.racers[a], &self.racers[b]);
            (a.left.cmp(&b.left)).then_with(|| match (a.finished, b.finished) {
                (Some(x), Some(y)) => x.total_cmp(&y),
                (Some(_), None) => Ordering::Less,
                (None, Some(_)) => Ordering::Greater,
                (None, None) => (b.gates.cmp(&a.gates)).then(a.at.total_cmp(&b.at)),
            })
        });
        order
    }
}

/// A racer's result.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    /// Their time, if they finished (s).
    pub time: Option<f32>,
    pub best_lap: Option<f32>,
    pub left: bool,
}

/// How a race or time trial ended: everyone in the order they placed.
#[derive(Clone, Debug, PartialEq)]
pub struct Results {
    /// The route, as it was called, and whether it was driven the other way.
    pub route: String,
    pub reverse: bool,
    pub entries: Vec<Entry>,
}

impl Results {
    pub fn of(standing: &Standing, route: String, reverse: bool) -> Self {
        let entries = (standing.order().into_iter())
            .map(|i| {
                let racer = &standing.racers[i];
                Entry {
                    name: racer.name.clone(),
                    time: racer.finished,
                    best_lap: racer.best_lap(),
                    left: racer.left,
                }
            })
            .collect();
        Self {
            route,
            reverse,
            entries,
        }
    }

    /// For people, in a line.
    pub fn text(&self) -> String {
        match self.entries.first() {
            Some(Entry {
                name,
                time: Some(time),
                ..
            }) => format!("{name} won in {}", clock(*time)),
            _ => "Nobody finished".into(),
        }
    }
}

/// A time as minutes, seconds and tenths.
pub fn clock(seconds: f32) -> String {
    let tenths = (seconds.max(0.0) * 10.0).round() as u32;
    format!("{}:{:02}.{}", tenths / 600, tenths / 10 % 60, tenths % 10)
}

/// A best time on the server's list.
#[derive(Clone, Debug, PartialEq)]
pub struct Best {
    pub name: String,
    /// A loop's lap, a sprint's whole way (s).
    pub time: f32,
}

impl Wire for Route {
    fn put(&self, out: &mut Vec<u8>) {
        self.name.put(out);
        self.looped.put(out);
        self.via.put(out);
        self.line.put(out);
        self.widths.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let route = Self {
            name: r.get()?,
            looped: r.get()?,
            via: r.get()?,
            line: r.get()?,
            widths: r.get()?,
        };
        (route.line.len() == route.widths.len() && (route.line.is_empty() || route.line.len() >= 2))
            .then_some(route)
    }
}

impl Wire for Settings {
    fn put(&self, out: &mut Vec<u8>) {
        self.route.put(out);
        self.reverse.put(out);
        self.laps.put(out);
        self.traffic.put(out);
        self.collisions.put(out);
        self.resets.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            route: r.get()?,
            reverse: r.get()?,
            laps: r.get()?,
            traffic: r.get()?,
            collisions: r.get()?,
            resets: r.get()?,
        })
    }
}

impl Wire for Racer {
    fn put(&self, out: &mut Vec<u8>) {
        self.player.put(out);
        self.name.put(out);
        self.gates.put(out);
        self.at.put(out);
        self.laps.put(out);
        self.finished.put(out);
        self.left.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            player: r.get()?,
            name: r.get()?,
            gates: r.get()?,
            at: r.get()?,
            laps: r.get()?,
            finished: r.get()?,
            left: r.get()?,
        })
    }
}

impl Wire for Standing {
    fn put(&self, out: &mut Vec<u8>) {
        self.racers.put(out);
        self.closes.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            racers: r.get()?,
            closes: r.get()?,
        })
    }
}

impl Wire for Entry {
    fn put(&self, out: &mut Vec<u8>) {
        self.name.put(out);
        self.time.put(out);
        self.best_lap.put(out);
        self.left.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            name: r.get()?,
            time: r.get()?,
            best_lap: r.get()?,
            left: r.get()?,
        })
    }
}

impl Wire for Results {
    fn put(&self, out: &mut Vec<u8>) {
        self.route.put(out);
        self.reverse.put(out);
        self.entries.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            route: r.get()?,
            reverse: r.get()?,
            entries: r.get()?,
        })
    }
}

impl Wire for Best {
    fn put(&self, out: &mut Vec<u8>) {
        self.name.put(out);
        self.time.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            name: r.get()?,
            time: r.get()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square loop, 400 m a side, on a 6 m road.
    pub fn square() -> Route {
        let corners = [
            Vec2::new(0.0, 0.0),
            Vec2::new(400.0, 0.0),
            Vec2::new(400.0, 400.0),
            Vec2::new(0.0, 400.0),
            Vec2::new(0.0, 0.0),
        ];
        Route {
            name: "Square".into(),
            looped: true,
            via: corners[..4].to_vec(),
            line: corners.to_vec(),
            widths: vec![6.0; 5],
        }
    }

    #[test]
    fn a_course_has_gates_to_its_finish() {
        let course = Course::new(&square(), false, 2);
        assert_eq!(course.length(), 1600.0);
        let last = course.gates.last().unwrap();
        assert_eq!(last.s, 1600.0, "the lap ends at the start");
        assert!(course.gates[0].s >= first_gate() - 0.01);
        for pair in course.gates.windows(2) {
            assert!(pair[1].s - pair[0].s <= GATE_SPACING + 0.01);
        }
        assert_eq!(course.total(), 2 * course.gates.len());
        assert!(course.ends_lap(course.gates.len()));
        // Crossed forwards only, and within its reach.
        let gate = course.gates[1];
        let side = Vec2::new(-gate.forward.y, gate.forward.x);
        let (behind, ahead) = (gate.point - gate.forward, gate.point + gate.forward);
        assert!(gate.crossed(behind, ahead));
        assert!(!gate.crossed(ahead, behind));
        assert!(gate.crossed(behind + side * 8.0, ahead + side * 8.0));
        assert!(!gate.crossed(behind + side * 10.0, ahead + side * 10.0));
        // The other way round, the same start.
        let back = Course::new(&square(), true, 1);
        assert_eq!(back.line[0], Vec2::ZERO);
        assert_eq!(back.point(10.0).1, Vec2::Y, "first along the west side");
    }

    #[test]
    fn the_grid_is_behind_the_front_row_and_apart() {
        let course = Course::new(&square(), false, 1);
        let spots: Vec<(Vec2, f32)> = (0..MAX_RACERS)
            .map(|i| course.grid(i, MAX_RACERS))
            .collect();
        for (i, a) in spots.iter().enumerate() {
            for b in &spots[i + 1..] {
                assert!(a.0.distance(b.0) > 2.5, "{a:?} {b:?}");
            }
            assert!(a.0.x < course.gates[0].s, "{a:?} past the first gate");
        }
        // Pole ahead of the second row; facing along the road (east: yaw -90°).
        assert!(spots[0].0.x > spots[2].0.x);
        assert!((spots[0].1 + std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        // Whoever comes, the front row is at the front.
        assert!((course.grid(0, 2).0.x - GRID_FRONT).abs() < 0.01);
    }

    #[test]
    fn the_standing_orders_finishers_then_progress() {
        let racer = |player, gates, at, finished, left| Racer {
            player,
            name: format!("{player}"),
            gates,
            at,
            laps: Vec::new(),
            finished,
            left,
        };
        let standing = Standing {
            racers: vec![
                racer(1, 5, 30.0, None, false),
                racer(2, 9, 50.0, Some(80.0), false),
                racer(3, 5, 29.0, None, false),
                racer(4, 9, 49.0, None, true),
                racer(5, 9, 55.0, Some(75.0), false),
            ],
            closes: None,
        };
        assert_eq!(standing.order(), [4, 1, 2, 0, 3]);
        assert_eq!(clock(83.46), "1:23.5");
    }
}
