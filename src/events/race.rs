//! Races and time trials: an event's routes, the course a round drives (a route so many
//! laps), its gates and starting grid, the host's settings, how a race stands and how it
//! ended.
//!
//! A route is made in the race editor (`cargo run --bin races`): its grid spots, placed
//! and turned by hand, and its checkpoints, each a gate. Stopping on a race's marker asks
//! whether to drive a time trial (alone, in a round of one, in a realm of one) or race
//! (into the lobby there: the first one in opens it and sets things). Running, everyone
//! is put on the route's grid (pole is the spot nearest its first checkpoint) and held
//! for the countdown; the clock runs from the green light. The server judges who passed
//! which gate from the cars' states, in order (a gate missed has to be gone back for). A
//! sprint's gates are its checkpoints, the last the finish; a loop's lap is its
//! checkpoints from the second on and the first again (passing the first from the grid
//! is the start, not a gate). Once the first racer is through, the others have
//! `FINISH_GRACE` to finish. A time trial's times go on the server's list of best times
//! (for the session), per route and traffic.

use std::cmp::Ordering;

use bevy::prelude::*;

use crate::net::protocol::PlayerId;
use crate::net::wire::{Reader, Wire};

/// The most racers in a round (and grid spots on a route).
pub const MAX_RACERS: usize = 16;
/// The countdown before the green light (s).
pub const COUNTDOWN: f64 = 5.0;
/// Once the first racer is through, the others have this long (s).
pub const FINISH_GRACE: f64 = 60.0;
/// What the laps can be set to.
pub const LAPS: (u8, u8) = (1, 20);
/// A gate reaches this far past the road's edge on either side (m): running wide counts.
const GATE_REACH: f32 = 6.0;
/// A route's lap (a sprint's whole way) must be this long at least (m).
pub const SHORTEST: f32 = 500.0;
/// A car moving further than this between two of its states didn't drive through a gate
/// (it was put somewhere: m).
const JUMP: f32 = 60.0;
/// Best times kept per route and traffic.
pub const BEST_KEPT: usize = 10;

/// A place on a route's grid: where a car's middle stands, and the way it faces.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GridSpot {
    pub at: Vec2,
    /// Compass degrees (0 north, 90 east), as the server's spawn.
    pub heading: f32,
}

impl GridSpot {
    /// The way it faces in the map frame (x east, z south).
    pub fn forward(&self) -> Vec2 {
        let h = self.heading.to_radians();
        Vec2::new(h.sin(), -h.cos())
    }

    /// The yaw a car standing on it has.
    pub fn yaw(&self) -> f32 {
        yaw(self.forward())
    }

    /// The heading (compass degrees, 0..360) of a direction in the map frame.
    pub fn heading_of(forward: Vec2) -> f32 {
        forward.x.atan2(-forward.y).to_degrees().rem_euclid(360.0)
    }
}

/// Puts `grid` in order, pole first: by distance to `first`, the route's first
/// checkpoint.
pub fn order_grid(grid: &mut [GridSpot], first: Vec2) {
    grid.sort_by(|a, b| a.at.distance(first).total_cmp(&b.at.distance(first)));
}

/// A route of a race event: as the server's list has it (`grid`, `checkpoints`) and along
/// the roads (`line`, with the road's width at each point; a loop's last point is its
/// first; `marks`, each checkpoint's place in the line).
#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub name: String,
    /// Round and round (laps), or from one end to the other.
    pub looped: bool,
    /// Pole first.
    pub grid: Vec<GridSpot>,
    pub checkpoints: Vec<Vec2>,
    pub line: Vec<Vec2>,
    pub widths: Vec<f32>,
    /// Each checkpoint's index in `line` (a loop's has its first again at the end).
    pub marks: Vec<u16>,
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

    /// How many can race on it.
    pub fn spots(&self) -> usize {
        self.grid.len().min(MAX_RACERS)
    }

    /// The route as lines of an events file (under a `race` line).
    pub fn text(&self) -> String {
        let kind = if self.looped { "loop" } else { "sprint" };
        let mut text = format!("route {kind} {}\n", self.name);
        for spot in &self.grid {
            text += &format!(
                "  grid {:.1} {:.1} {:.0}\n",
                spot.at.x, spot.at.y, spot.heading
            );
        }
        for p in &self.checkpoints {
            text += &format!("  {:.1} {:.1}\n", p.x, p.y);
        }
        text
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

/// A gate faces the way the line goes over this far before and after it (m).
const GATE_LOOK: f32 = 10.0;

/// The gates of a route's checkpoints (all of them, in order: a loop's first at both
/// ends of its line), each across the line at its mark, facing the way the line goes
/// there (the way in and out over `GATE_LOOK` averaged).
pub fn checkpoint_gates(route: &Route) -> Vec<Gate> {
    let line = &route.line;
    let n = line.len();
    if n < 2 {
        return Vec::new();
    }
    let mut at = vec![0.0];
    for pair in line.windows(2) {
        at.push(at.last().unwrap() + pair[0].distance(pair[1]));
    }
    let closed = route.looped && line.first() == line.last();
    // From point `i`, the way to the first point `GATE_LOOK` away going `step` (round a
    // closed loop), or to the last there is.
    let look = |i: usize, step: isize| {
        let from = line[i];
        let mut j = i as isize;
        let mut to = from;
        for _ in 0..n {
            j += step;
            if j < 0 || j >= n as isize {
                if !closed {
                    break;
                }
                // Past the end of a loop's line is past its start (the same point).
                j = if j < 0 { n as isize - 2 } else { 1 };
            }
            to = line[j as usize];
            if to.distance(from) >= GATE_LOOK {
                break;
            }
        }
        (to - from).normalize_or_zero()
    };
    (route.marks.iter())
        .map(|&mark| {
            let i = usize::from(mark).min(n - 1);
            let (into, out) = (-look(i, -1), look(i, 1));
            let along = if out == Vec2::ZERO { into } else { out };
            let forward = (into + out).normalize_or(along.normalize_or(Vec2::NEG_Y));
            let width = route.widths[i];
            Gate {
                s: at[i],
                point: line[i],
                forward,
                width,
                half: width / 2.0 + GATE_REACH,
            }
        })
        .collect()
}

/// A route driven so many laps: its line from its first checkpoint, its gates and grid.
#[derive(Clone, Debug)]
pub struct Course {
    pub looped: bool,
    pub laps: u16,
    pub line: Vec<Vec2>,
    pub widths: Vec<f32>,
    /// Distance along the line to each point (m).
    pub at: Vec<f32>,
    /// One lap's gates, in order; the last is the finish (a loop's: its first
    /// checkpoint's).
    pub gates: Vec<Gate>,
    /// Pole first.
    pub grid: Vec<GridSpot>,
}

impl Course {
    /// `route`, `laps` times round if it's a loop.
    pub fn new(route: &Route, laps: u8) -> Self {
        let mut at = vec![0.0];
        for pair in route.line.windows(2) {
            at.push(at.last().unwrap() + pair[0].distance(pair[1]));
        }
        let mut gates = checkpoint_gates(route);
        if route.looped && !gates.is_empty() {
            // The first checkpoint ends each lap (its gate at the line's end).
            gates.remove(0);
        }
        Self {
            looped: route.looped,
            laps: if route.looped {
                u16::from(laps.max(1))
            } else {
                1
            },
            line: route.line.clone(),
            widths: route.widths.clone(),
            at,
            gates,
            grid: route.grid.clone(),
        }
    }

    /// One lap's length (m).
    pub fn length(&self) -> f32 {
        self.at.last().copied().unwrap_or(0.0)
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

    /// Where the racer in `slot` (0: pole) starts, and the yaw they face.
    pub fn grid(&self, slot: usize) -> (Vec2, f32) {
        match self.grid.get(slot).or(self.grid.last()) {
            Some(spot) => (spot.at, spot.yaw()),
            None => {
                let (point, forward, _) = self.point(0.0);
                (point, yaw(forward))
            }
        }
    }

    /// Where a racer who passed `passed` gates is put back (R): at the last gate they
    /// passed, in the right-hand half of the road, or on their grid spot if none.
    pub fn reset_spot(&self, passed: usize, slot: usize) -> (Vec2, f32) {
        if passed == 0 {
            return self.grid(slot);
        }
        let gate = self.next(passed - 1);
        let right = Vec2::new(-gate.forward.y, gate.forward.x);
        let lane = (gate.width / 4.0).min(2.0);
        (gate.point + right * lane, gate.yaw())
    }

    /// How far along one lap the point of the line nearest to `p` is, looking only within
    /// `from..to` metres along (m), the line's direction there, and how far `p` is from
    /// it (m).
    pub fn nearest(&self, p: Vec2, from: f32, to: f32) -> Option<(f32, Vec2, f32)> {
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
    }
}

/// What the host sets (and a time trial's driver).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// Which of the event's routes.
    pub route: u8,
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
    /// The route, as it was called.
    pub route: String,
    pub entries: Vec<Entry>,
}

impl Results {
    pub fn of(standing: &Standing, route: String) -> Self {
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
        Self { route, entries }
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

impl Wire for GridSpot {
    fn put(&self, out: &mut Vec<u8>) {
        self.at.put(out);
        self.heading.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            at: r.get()?,
            heading: r.get()?,
        })
    }
}

impl Wire for Route {
    fn put(&self, out: &mut Vec<u8>) {
        self.name.put(out);
        self.looped.put(out);
        self.grid.put(out);
        self.checkpoints.put(out);
        self.line.put(out);
        self.widths.put(out);
        self.marks.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let route = Self {
            name: r.get()?,
            looped: r.get()?,
            grid: r.get()?,
            checkpoints: r.get()?,
            line: r.get()?,
            widths: r.get()?,
            marks: r.get()?,
        };
        let n = route.line.len();
        let marks_fit = (route.marks.iter()).all(|&m| usize::from(m) < n)
            && route.marks.windows(2).all(|w| w[0] <= w[1]);
        (route.line.len() == route.widths.len()
            && (n == 0 || n >= 2)
            && marks_fit
            && route.grid.len() <= MAX_RACERS)
            .then_some(route)
    }
}

impl Wire for Settings {
    fn put(&self, out: &mut Vec<u8>) {
        self.route.put(out);
        self.laps.put(out);
        self.traffic.put(out);
        self.collisions.put(out);
        self.resets.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            route: r.get()?,
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
        self.entries.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            route: r.get()?,
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

    /// A square loop, 400 m a side, on a 6 m road, its checkpoints in the middle of each
    /// side (the first on the north side, going east), two grid spots before the first.
    pub fn square() -> Route {
        let line = vec![
            Vec2::new(200.0, 0.0),
            Vec2::new(400.0, 0.0),
            Vec2::new(400.0, 200.0),
            Vec2::new(400.0, 400.0),
            Vec2::new(200.0, 400.0),
            Vec2::new(0.0, 400.0),
            Vec2::new(0.0, 200.0),
            Vec2::new(0.0, 0.0),
            Vec2::new(200.0, 0.0),
        ];
        Route {
            name: "Square".into(),
            looped: true,
            grid: vec![
                GridSpot {
                    at: Vec2::new(190.0, 1.5),
                    heading: 90.0,
                },
                GridSpot {
                    at: Vec2::new(184.0, -1.5),
                    heading: 90.0,
                },
            ],
            checkpoints: vec![line[0], line[2], line[4], line[6]],
            widths: vec![6.0; line.len()],
            marks: vec![0, 2, 4, 6, 8],
            line,
        }
    }

    #[test]
    fn a_loops_gates_are_its_checkpoints_the_first_last() {
        let route = square();
        let course = Course::new(&route, 2);
        assert_eq!(course.length(), 1600.0);
        let s: Vec<f32> = course.gates.iter().map(|g| g.s).collect();
        assert_eq!(
            s,
            [400.0, 800.0, 1200.0, 1600.0],
            "the lap ends at the first"
        );
        assert_eq!(course.gates[3].point, route.checkpoints[0]);
        assert_eq!(course.total(), 8);
        assert!(course.ends_lap(4) && !course.ends_lap(3));
        // Each faces the way the road goes there: the first east, the second south.
        assert!(course.gates[3].forward.distance(Vec2::X) < 1e-5);
        assert!(course.gates[0].forward.distance(Vec2::Y) < 1e-5);
        // Crossed forwards only, and within its reach.
        let gate = course.gates[0];
        let side = Vec2::new(-gate.forward.y, gate.forward.x);
        let (behind, ahead) = (gate.point - gate.forward, gate.point + gate.forward);
        assert!(gate.crossed(behind, ahead));
        assert!(!gate.crossed(ahead, behind));
        assert!(gate.crossed(behind + side * 8.0, ahead + side * 8.0));
        assert!(!gate.crossed(behind + side * 10.0, ahead + side * 10.0));
    }

    #[test]
    fn a_sprints_gates_are_all_its_checkpoints() {
        let mut route = square();
        route.looped = false;
        route.line.pop();
        route.widths.pop();
        route.marks.pop();
        let course = Course::new(&route, 3);
        assert_eq!(course.laps, 1);
        let s: Vec<f32> = course.gates.iter().map(|g| g.s).collect();
        assert_eq!(s, [0.0, 400.0, 800.0, 1200.0]);
        // At an end, the gate faces along the line's end.
        assert!(course.gates[0].forward.distance(Vec2::X) < 1e-5);
    }

    #[test]
    fn the_grid_is_the_routes_pole_first() {
        let mut route = square();
        route.grid.reverse();
        order_grid(&mut route.grid, route.checkpoints[0]);
        assert_eq!(
            route.grid,
            square().grid,
            "pole is nearest the first checkpoint"
        );
        let course = Course::new(&route, 1);
        let (pole, yaw) = course.grid(0);
        assert_eq!(pole, Vec2::new(190.0, 1.5));
        // East: yaw -90°.
        assert!((yaw + std::f32::consts::FRAC_PI_2).abs() < 1e-4);
        assert_eq!(course.reset_spot(0, 1).0, Vec2::new(184.0, -1.5));
        // Past a gate, R puts the car right of the road's middle there.
        let (back, _) = course.reset_spot(1, 1);
        assert!((back - Vec2::new(398.5, 200.0)).length() < 1e-4, "{back}");
        let heading = GridSpot::heading_of(Vec2::new(0.0, 1.0));
        assert!((heading - 180.0).abs() < 1e-4);
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
