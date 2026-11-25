//! Each player's car as the server drives it, from the inputs the player sends: the
//! server is in charge of it. Ten times a second the player is sent its state
//! (`ToClient::Yours`); their game drives the car too, at once, and takes the server's
//! state up where it differs (see `predict`). Everyone else sees the server's car.
//!
//! The cars run on the server's clock, one step per server step (`Tick`), all at once. A
//! player's steps are numbered by their game: the car is driven through step
//! `tick + offset`, the offset set when their inputs first come so that they come `MARGIN`
//! steps before they're needed, and moved once a second by however late they came at the
//! worst (or much too early). How early they come is told back to the player
//! (`ToClient::Pace`), whose game then runs its clock a little faster or slower to keep
//! them coming just in time. An input that isn't there in time is guessed (the one
//! before, with nothing pressed); the buttons pressed in it, if it comes after all, are
//! pressed in the next step (R, a shift: none is lost).
//!
//! The server moves the car itself where the game's rules do (`put`: on joining, to an
//! event's spots, onto the grid, back after a round), telling the player's game, which
//! takes the state up whatever it had (`ToClient::Yours::moved`). Where the player's game
//! changes the car outside the simulation (built anew in a shop, refuelled), it says so
//! (`place`): the server takes up that state and drives the steps since again (trusted,
//! until the server does those things itself), but never where it would move the car:
//! a placement more than `REACH` from where the server has the car is refused (a car built
//! anew goes where the old one was, or where the server put it). Once a second the player's game also sends
//! its car's state after a numbered step (`check`), which is only compared with the
//! server's for the log (how many matched to the bit, how far off the others were).
//!
//! A car built of an engine whose maps don't come with the game is baked on a thread of
//! its own (seconds); meanwhile the player's own word is shown.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::prelude::*;

use super::client::CHECK_STEPS;
use super::protocol::Yours;
use crate::car::{Car, DriverInput, ResetRule, heading};
use crate::heli::HeliPose;
use crate::heli::rescue::{Rescue, fly};
use crate::physics::engine::maps::{EngineMaps, fingerprint};
use crate::sim::{STEP, step_car};
use crate::snapshot;
use crate::spec::CarSpec;
use crate::terrain::Terrain;

/// Inputs kept after they're driven, for driving the steps after a check again (steps:
/// two checks' worth).
const KEPT: u32 = 2 * CHECK_STEPS;
/// How many steps before they're needed a player's inputs should come: enough for the
/// way's jitter and the server's frames (two steps each), little enough not to delay the
/// car much once the server's in charge.
pub const MARGIN: i32 = 4;
/// Inputs coming this many steps late, or this many early, set the offset afresh (a game
/// that stalled, or a clock that ran away).
const TOO_LATE: i32 = 120;
const TOO_EARLY: i32 = 240;
/// Every this many server steps, the offset moves by however late the player's inputs
/// came at the worst (a game whose frames are long sends many steps at once, and needs a
/// margin of a frame), or however much earlier than `MARGIN` they came at the least if
/// that's more than `SLACK` (the frames got quicker).
const WINDOW: i32 = 120;
const SLACK: i32 = 12;
/// Without any input for this many steps, the copy is driven as if nobody's at the wheel.
const SILENT: u32 = 30;
/// The car's state goes to its player after every step whose number is a multiple of
/// this (ten times a second).
const YOURS_STEPS: u32 = 12;
/// How often each car's report is logged (s).
const REPORT: f64 = 10.0;
/// What the player's game shows while the rescue helicopter has their car (the first line
/// large).
const RESCUED: &str = "⚠ You are being rescued ⚠\nPlease do not resist!";
/// The most checks waiting for the copy to get to their step.
const MOST_WAITING: usize = 4;
/// How far a player's placement may be from where the server has the car (m), and how
/// much farther for each m/s it moves per second the placement is old.
const REACH: f32 = 2.0;

/// Whether placements are taken up however far they move the car: only for screenshots
/// of the game's own server (`CAR_SIM_TRUST_PLACEMENTS=1`, which `tools/dev/shot.sh` sets
/// for `PLACE`).
fn trusts_placements() -> bool {
    static TRUSTS: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var("CAR_SIM_TRUST_PLACEMENTS").is_ok());
    *TRUSTS
}
const REACH_PER_SPEED: f32 = 1.5;
/// How much more fuel a player's placement may have than the server's car (L), and how
/// much more for each second it's old (what the engine could burn meanwhile).
const FUEL_SLACK: f32 = 0.05;
const FUEL_PER_SECOND: f32 = 0.1;

/// The server's simulation step, counted.
#[derive(Resource, Default)]
pub struct Tick(pub u32);

/// Engine maps the server has baked for players' cars, by their hardware's fingerprint.
#[derive(Resource, Default)]
pub struct BakedMaps(HashMap<u64, EngineMaps>);

/// How far step `a` is after step `b` (negative: before), the counters wrapping.
fn after(a: u32, b: u32) -> i32 {
    a.wrapping_sub(b) as i32
}

/// A player's car as the server drives it.
#[derive(Default)]
pub struct Shadow {
    /// What the car is built of, as the player last said.
    spec: Option<CarSpec>,
    /// Its engine's maps being baked (by their fingerprint), there when done.
    baking: Option<(u64, Baking)>,
    /// The copy (none until a check gave it a state), and the last step it was driven
    /// through.
    car: Option<Car>,
    done: u32,
    /// A car of the spec without a state yet (waiting for a check).
    built: Option<Car>,
    /// The player's step driven at server step 0 (wrapping): step `tick + offset` is
    /// driven at `tick`. None until their inputs come.
    offset: Option<u32>,
    /// Inputs by step: the ones to come and the last `KEPT` driven (with the guesses for
    /// the ones that didn't come in time, until they do).
    inputs: BTreeMap<u32, DriverInput>,
    /// The latest step an input came for.
    latest: Option<u32>,
    /// The steps driven with a guess for their input, and the buttons pressed in ones that
    /// came after all, for the next step.
    guesses: BTreeSet<u32>,
    late: DriverInput,
    /// When the step under way began (between `step` and `finish_step`).
    started: Option<Instant>,
    /// How early their inputs came lately: the least, in steps (for `ToClient::Pace`).
    early: Option<i32>,
    /// The server step the window for moving the offset began at, and how early the
    /// inputs came in it at the least.
    window: Option<(u32, i32)>,
    /// The copy's state after each check step it's driven through, not yet compared.
    ours: BTreeMap<u32, Vec<u8>>,
    /// The player's checks for steps the car hasn't got to yet.
    waiting: BTreeMap<u32, Vec<u8>>,
    /// The player's placements (their car put somewhere after a step), not taken up yet,
    /// and the latest taken up.
    placements: BTreeMap<u32, Vec<u8>>,
    placed: Option<u32>,
    /// The car's latest state to send the player.
    yours: Option<Yours>,
    /// How many times the server moved the car (`put`).
    moved: u32,
    /// Where a car without a state yet goes when it gets one, if it's elsewhere: where the
    /// server put it (or the car it replaces was).
    anchor: Option<Spot>,
    /// The player's own car while they drive one lent for an event's round: its spec and
    /// state (the player's game can't change the lent one's build meanwhile).
    own: Option<(Option<CarSpec>, Car)>,
    /// What R does with the car (the player's round's rules), and the rescue under way.
    pub rule: ResetRule,
    rescue: Rescue,
    /// Whether the player was last told the server has their car.
    taken: bool,
    /// Whether the car may be refuelled (stopped at a gas station).
    pub refuel: bool,
    report: Report,
}

/// Where the server puts a car: at rest at `at` (map x, z) facing `yaw`, with the
/// parking brake on if `parked` (else off: on a race's grid), held still for `hold` (s).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub at: Vec2,
    pub yaw: f32,
    pub parked: bool,
    pub hold: f32,
}

impl Spot {
    /// Puts `car` there, as it was otherwise.
    fn put(&self, car: &mut Car, terrain: &Terrain) {
        car.place(terrain, self.at, self.yaw);
        car.parking_brake = self.parked;
        car.hold = self.hold;
    }
}

/// How the copy followed the player's car lately.
#[derive(Default)]
struct Report {
    since: f64,
    checks: usize,
    /// Checks whose states were the same to the bit.
    exact: usize,
    /// Steps driven with a guess for an input that didn't come in time.
    guessed: usize,
    /// Times the offset was set afresh.
    anchored: usize,
    /// The farthest the copy was off (m), and the largest difference in speed (m/s) and
    /// engine speed (rpm).
    position: f32,
    velocity: f32,
    rpm: f32,
    steps: usize,
    time: Duration,
}

impl Shadow {
    /// The player's car is built of `spec` from now on.
    pub fn set_spec(&mut self, spec: CarSpec, baked: &mut BakedMaps) {
        if self.spec.as_ref() == Some(&spec) || self.own.is_some() {
            return;
        }
        // The new car goes where the old one is.
        if let Some(car) = &self.car {
            self.anchor = Some(Spot {
                at: car.body.position.xz(),
                yaw: heading(car.body.rotation),
                parked: car.parking_brake,
                hold: car.hold,
            });
        }
        self.car = None;
        self.built = None;
        self.baking = None;
        self.ours.clear();
        self.waiting.clear();
        self.placements.clear();
        let hardware = spec.parts.hardware();
        let print = fingerprint(&hardware);
        if let Some(maps) = (spec.stock_maps()).or_else(|| baked.0.get(&print).cloned()) {
            self.built = Some(spec_car(&spec, maps));
        } else {
            info!("Baking the maps of a player's engine ({print:016x})");
            self.baking = Some((print, bake(hardware)));
        }
        self.spec = Some(spec);
    }

    /// The driver's inputs in the steps up to `last`, the latest last, come at server
    /// step `tick`.
    pub fn take_inputs(&mut self, last: u32, inputs: &[DriverInput], tick: u32) {
        let first = last.wrapping_sub(inputs.len() as u32 - 1);
        for (i, input) in inputs.iter().enumerate() {
            let step = first.wrapping_add(i as u32);
            if self.guesses.remove(&step) {
                press_later(&mut self.late, input);
            }
            self.inputs.insert(step, *input);
        }
        // The first step new in this message (none: nothing new).
        let new = match self.latest {
            Some(latest) if after(last, latest) <= 0 => return,
            Some(latest) if after(last, latest) <= inputs.len() as i32 => latest.wrapping_add(1),
            _ => last,
        };
        self.latest = Some(last);
        // How early the first new one came: the steps until it's driven (a game whose
        // frames are long sends many at once, and the server mustn't run out between).
        let early = self
            .offset
            .map(|offset| after(new.wrapping_sub(offset), tick));
        match early {
            Some(early) if (-TOO_LATE..=TOO_EARLY).contains(&early) => {
                self.early = Some(self.early.map_or(early, |least| least.min(early)));
                let (start, least) = self.window.get_or_insert((tick, early));
                *least = (*least).min(early);
                if after(tick, *start) >= WINDOW {
                    let least = *least;
                    if !(0..=MARGIN + SLACK).contains(&least) {
                        // The car takes up the server's new numbering of its steps.
                        let offset = self
                            .offset
                            .expect("an offset")
                            .wrapping_add_signed(least - MARGIN);
                        self.offset = Some(offset);
                        self.report.anchored += 1;
                    }
                    self.window = None;
                }
            }
            _ => {
                // First inputs, or a game far off: from now on they come just in time.
                if early.is_some() {
                    self.report.anchored += 1;
                }
                self.offset = Some(last.wrapping_sub(tick.wrapping_add(MARGIN as u32)));
                self.early = Some(MARGIN);
                self.window = None;
            }
        }
        // Without a copy to drive, only the latest are any use.
        while self.car.is_none() && self.inputs.len() > KEPT as usize {
            self.inputs.pop_first();
        }
    }

    /// How early the player's inputs came since this was last asked (steps), if any came.
    pub fn take_early(&mut self) -> Option<i32> {
        self.early.take()
    }

    /// The player's car was put somewhere after `step`: its whole state then.
    pub fn place(&mut self, step: u32, state: Vec<u8>) {
        if self.placements.len() >= MOST_WAITING {
            self.placements.pop_first();
        }
        self.placements.insert(step, state);
    }

    /// The car's latest state for the player, if there's a new one.
    pub fn take_yours(&mut self) -> Option<Yours> {
        self.yours.take()
    }

    /// What the car is built of, as the server has it.
    pub fn spec(&self) -> Option<&CarSpec> {
        self.spec.as_ref()
    }

    /// Moves the car to `spot`, as it was otherwise (a rescue under way lets go). A car
    /// without a state yet goes there when it gets one.
    pub fn put(&mut self, terrain: &Terrain, spot: Spot) {
        match &mut self.car {
            Some(car) => {
                spot.put(car, terrain);
                self.rescue.let_go();
                self.moved = self.moved.wrapping_add(1);
                self.ours.clear();
                self.tell(self.done);
            }
            None => self.anchor = Some(spot),
        }
    }

    /// The car's state after `step` for the player, now.
    fn tell(&mut self, step: u32) {
        let Some(car) = &self.car else {
            return;
        };
        self.taken = self.rescue.has_car();
        self.yours = Some(Yours {
            step,
            placed: self.placed,
            moved: self.moved,
            notice: self.taken.then(|| RESCUED.to_string()),
            state: snapshot::save(car),
        });
    }

    /// Whether the server has the car (the rescue helicopter is coming for it or carrying
    /// it): the player's input does nothing.
    pub fn taken(&self) -> bool {
        self.rescue.has_car()
    }

    /// The rescue helicopter's pose, while there is one, as of `time`.
    pub fn heli(&self, time: f64) -> Option<HeliPose> {
        self.rescue.pose(time)
    }

    /// Lends the player a car of `spec` (a factory build) at `spot`, keeping theirs until
    /// `give_back`. Without a car of theirs yet, or maps for it, it's only put there.
    pub fn lend(&mut self, spec: CarSpec, terrain: &Terrain, spot: Spot) -> Option<CarSpec> {
        let maps = spec.stock_maps();
        let (Some(maps), Some(own), None) = (maps, self.car.take(), &self.own) else {
            self.put(terrain, spot);
            return None;
        };
        let mut lent = spec_car(&spec, maps);
        lent.keep_settings(&own);
        self.own = Some((self.spec.replace(spec.clone()), own));
        self.baking = None;
        self.built = None;
        self.car = Some(lent);
        self.waiting.clear();
        self.placements.clear();
        self.put(terrain, spot);
        Some(spec)
    }

    /// Gives the player their own car back at `spot`, as it was when the lent one came;
    /// without one lent, only puts theirs there.
    pub fn give_back(&mut self, terrain: &Terrain, spot: Spot) -> Option<CarSpec> {
        let given = self.own.take().map(|(spec, own)| {
            self.spec = spec;
            self.baking = None;
            self.built = None;
            self.car = Some(own);
            self.waiting.clear();
            self.placements.clear();
            self.spec.clone()
        });
        self.put(terrain, spot);
        given.flatten()
    }

    /// Where the car is, as the server has it (none before it has a state).
    pub fn position(&self) -> Option<Vec3> {
        self.car.as_ref().map(|car| car.body.position)
    }

    /// The car as the server drives it, while the player's inputs come (a game that
    /// stopped sending them, a browser tab hidden, isn't driving it).
    pub fn car(&self) -> Option<&Car> {
        let latest = self.latest?;
        self.car
            .as_ref()
            .filter(|_| after(self.done, latest) <= SILENT as i32)
    }

    /// The player's car's whole state after `step`, as their game has it (for the log).
    pub fn check(&mut self, step: u32, state: Vec<u8>) {
        if self.waiting.len() >= MOST_WAITING {
            self.waiting.pop_first();
        }
        self.waiting.insert(step, state);
    }

    /// Server step `tick`: drives the copy through the player's step for it (the rest
    /// of the step, after the cars met, is `finish_step`).
    pub fn step(&mut self, tick: u32, terrain: &Terrain) {
        let (Some(car), Some(offset)) = (&mut self.car, self.offset) else {
            return;
        };
        let step = tick.wrapping_add(offset);
        // (Not the step after the last when the offset was set afresh: the car goes on,
        // and the player's game takes up the server's numbering of its state.)
        let started = Instant::now();
        let input = match self.inputs.get(&step) {
            Some(&input) => input,
            None => {
                self.report.guessed += 1;
                let input = guess(&self.inputs, self.latest, step);
                self.inputs.insert(step, input);
                self.guesses.insert(step);
                input
            }
        };
        // What was pressed in steps that had to be guessed.
        let mut input = input;
        press_later(&mut input, &std::mem::take(&mut self.late));
        // R calls the rescue (as the rules say); nobody at the wheel lets it go.
        if input.away {
            self.rescue.let_go();
        } else if input.reset && !self.rescue.busy() {
            self.rescue.press(self.rule, car, terrain);
        }
        // While the server has the car, the player's input does nothing.
        let input = DriverInput {
            away: input.away || self.rescue.has_car(),
            ..input
        };
        car.drive(&input, STEP);
        step_car(car, terrain, STEP);
        self.done = step;
        self.started = Some(started);
    }

    /// The end of the step `step` began, once the cars have met: the rescue flies on, and
    /// the car's state is kept to compare and send.
    pub fn finish_step(&mut self, terrain: &Terrain) {
        let (Some(car), Some(started)) = (&mut self.car, self.started.take()) else {
            return;
        };
        let step = self.done;
        fly(&mut self.rescue, car, terrain, STEP);
        if step.is_multiple_of(CHECK_STEPS) {
            self.ours.insert(step, snapshot::save(car));
        }
        // The server takes the car, or gives it back (set down: a move), and says so at
        // once.
        let taken = self.rescue.has_car();
        if taken != self.taken {
            if !taken {
                self.moved = self.moved.wrapping_add(1);
            }
            self.tell(step);
        } else if step.is_multiple_of(YOURS_STEPS) {
            self.tell(step);
        }
        let report = &mut self.report;
        report.steps += 1;
        report.time += started.elapsed();
        self.inputs
            .retain(|&kept, _| after(step, kept) < KEPT as i32);
        self.ours.retain(|&kept, _| after(step, kept) < KEPT as i32);
        self.guesses.retain(|&kept| after(step, kept) < KEPT as i32);
    }

    /// Between server frames, `tick` the last server step: finishes baking, gives a copy
    /// its first state, compares the checks the copy has got to, and logs how it went now
    /// and then (as `who`'s car).
    pub fn update(
        &mut self,
        terrain: &Terrain,
        baked: &mut BakedMaps,
        tick: u32,
        now: f64,
        who: &str,
    ) {
        self.finish_baking(baked);
        self.take_up_placement(tick, terrain);
        self.compare();
        self.log(now, who);
    }

    /// Takes up the player's latest placement the car has got to (or any, for a car
    /// without a state yet), driving the steps since again.
    fn take_up_placement(&mut self, tick: u32, terrain: &Terrain) {
        let Some(offset) = self.offset else {
            return;
        };
        // The step the car is (or should be) through by now.
        let done = match &self.car {
            Some(_) => self.done,
            None => tick.wrapping_add(offset),
        };
        let fresh = self.car.is_none();
        let Some(&step) = (self.placements.keys())
            .rev()
            .find(|&&step| after(done, step) >= 0)
        else {
            return;
        };
        let state = self.placements.remove(&step).expect("the placement");
        self.placements.retain(|&later, _| after(later, step) > 0);
        if self.rescue.has_car() {
            // The server has the car: nothing the player's game says changes it.
            self.placed = Some(step);
            return;
        }
        let Some(mut car) = self.car.take().or_else(|| self.built.take()) else {
            return;
        };
        let was = (!fresh).then(|| car.clone());
        let anchor = if fresh { self.anchor } else { None };
        if !snapshot::load(&mut car, &state) {
            // Not a state of this car (from before it changed).
            if fresh {
                self.built = Some(car);
            } else {
                self.car = Some(car);
            }
            return;
        }
        self.placed = Some(step);
        let mut moved = false;
        if let Some(was) = was {
            // Not moved: where the server has it, give or take how far it went since.
            let age = after(done, step).max(0) as f32 * STEP;
            let speed = was.body.velocity.length().max(car.body.velocity.length());
            let reach = REACH + REACH_PER_SPEED * speed * age;
            if was.body.position.distance(car.body.position) > reach && !trusts_placements() {
                warn!("A player's game moved their car: refused");
                self.car = Some(was);
                return;
            }
            // Nor let it go early, nor fill it up away from a gas station.
            car.hold = if was.hold > 0.0 { was.hold + age } else { 0.0 };
            let most = was.fuel.litres() + FUEL_SLACK + FUEL_PER_SECOND * age;
            if !self.refuel && car.fuel.litres() > most {
                warn!("A player's game filled their car away from a gas station: refused");
                car.fuel = was.fuel;
                car.load_fuel();
            }
        } else if let Some(spot) = self.anchor.take() {
            // Held as the server says.
            car.hold = spot.hold;
        }
        if let Some(spot) = anchor.filter(|spot| car.body.position.xz().distance(spot.at) > REACH) {
            // Where the server has it.
            spot.put(&mut car, terrain);
            self.moved = self.moved.wrapping_add(1);
            moved = true;
        }
        // Driven on to where it's due from the inputs kept (a placement from long ago, a
        // car whose engine was baking: from where it was put).
        let since = after(done, step);
        if (0..=KEPT as i32).contains(&since) {
            self.ours.retain(|&kept, _| after(kept, step) <= 0);
            self.redrive(&mut car, step, done, terrain);
        }
        self.car = Some(car);
        self.done = done;
        if moved {
            self.tell(done);
        }
    }

    /// Compares the player's checks the car has got to with its own states, for the log.
    fn compare(&mut self) {
        let Some(car) = &self.car else {
            return;
        };
        let done = self.done;
        let ready: Vec<u32> = (self.waiting.keys())
            .copied()
            .filter(|&step| after(done, step) >= 0)
            .collect();
        for step in ready {
            let theirs = self.waiting.remove(&step).expect("a waiting check");
            let Some(ours) = self.ours.remove(&step) else {
                continue;
            };
            let report = &mut self.report;
            report.checks += 1;
            if ours == theirs {
                report.exact += 1;
                continue;
            }
            let (mut a, mut b) = (car.clone(), car.clone());
            if snapshot::load(&mut a, &ours) && snapshot::load(&mut b, &theirs) {
                let off = |x: Vec3, y: Vec3| x.distance(y);
                report.position = (report.position).max(off(a.body.position, b.body.position));
                report.velocity = (report.velocity).max(off(a.body.velocity, b.body.velocity));
                report.rpm = (report.rpm).max((a.drivetrain.rpm - b.drivetrain.rpm).abs());
            }
        }
    }

    /// Drives `car` from after step `from` through step `to` again, from the inputs kept.
    fn redrive(&mut self, car: &mut Car, from: u32, to: u32, terrain: &Terrain) {
        let mut step = from;
        while after(to, step) > 0 {
            step = step.wrapping_add(1);
            let input = match self.inputs.get(&step) {
                Some(&input) => input,
                None => {
                    let input = guess(&self.inputs, self.latest, step);
                    self.inputs.insert(step, input);
                    input
                }
            };
            car.drive(&input, STEP);
            step_car(car, terrain, STEP);
            if step.is_multiple_of(CHECK_STEPS) {
                self.ours.insert(step, snapshot::save(car));
            }
        }
    }

    fn finish_baking(&mut self, baked: &mut BakedMaps) {
        let Some((print, baking)) = &self.baking else {
            return;
        };
        let Some(maps) = baking.lock().ok().and_then(|mut done| done.take()) else {
            return;
        };
        info!("Baked the maps of a player's engine ({print:016x})");
        baked.0.insert(*print, maps.clone());
        if let Some(spec) = &self.spec {
            self.built = Some(spec_car(spec, maps));
        }
        self.baking = None;
    }

    fn log(&mut self, now: f64, who: &str) {
        let report = &mut self.report;
        if now - report.since < REPORT {
            return;
        }
        if report.checks > 0 || report.steps > 0 {
            let cost = report.time.as_secs_f64() * 1e6 / report.steps.max(1) as f64;
            info!(
                "{who}'s car on the server: {} of {} checks to the bit, the others up to \
                 {:.3} m, {:.3} m/s and {:.0} rpm off; {} steps ({cost:.0} µs each), {} \
                 without their input in time, the offset moved {} times",
                report.exact,
                report.checks,
                report.position,
                report.velocity,
                report.rpm,
                report.steps,
                report.guessed,
                report.anchored,
            );
        }
        *report = Report {
            since: now,
            ..Report::default()
        };
    }

    /// Whether nobody's at the wheel, by the latest input (away in a shop's room).
    pub fn away(&self) -> bool {
        (self.latest.and_then(|latest| self.inputs.get(&latest))).is_some_and(|input| input.away)
    }

    /// The car as the server drives it, while the player's inputs come, to push about.
    pub fn car_mut(&mut self) -> Option<&mut Car> {
        let latest = self.latest?;
        let done = self.done;
        self.car
            .as_mut()
            .filter(|_| after(done, latest) <= SILENT as i32)
    }

    /// How many of the checks so far matched to the bit, of how many (for tests).
    #[cfg(test)]
    pub fn matched(&self) -> (usize, usize) {
        (self.report.exact, self.report.checks)
    }
}

/// Adds the buttons pressed in `from` to `to`.
fn press_later(to: &mut DriverInput, from: &DriverInput) {
    to.reset |= from.reset;
    to.ignition |= from.ignition;
    to.handbrake_toggle |= from.handbrake_toggle;
    to.shift = to.shift.saturating_add(from.shift);
}

/// A stand-in for the input of `step`, which didn't come in time: the one before, with
/// nothing pressed (presses count once); nobody at the wheel if the player's been silent.
fn guess(inputs: &BTreeMap<u32, DriverInput>, latest: Option<u32>, step: u32) -> DriverInput {
    let silent = latest.is_none_or(|latest| after(step, latest) > SILENT as i32);
    let before = inputs.range(..step).next_back().map(|(_, &input)| input);
    match before {
        Some(input) if !silent => DriverInput {
            handbrake_toggle: false,
            shift: 0,
            ignition: false,
            reset: false,
            ..input
        },
        Some(input) => DriverInput {
            away: true,
            traction_control: input.traction_control,
            abs: input.abs,
            ..default()
        },
        None => DriverInput {
            away: true,
            ..default()
        },
    }
}

/// A car of `spec` with `maps`, anywhere (a check gives it its state).
fn spec_car(spec: &CarSpec, maps: EngineMaps) -> Car {
    Car::new(0, spec.setup_with(maps), Vec3::ZERO, Quat::IDENTITY)
}

/// Maps being baked on a thread of their own: there once done.
type Baking = Arc<Mutex<Option<EngineMaps>>>;

/// Bakes `hardware`'s maps on a thread of its own.
fn bake(hardware: crate::physics::engine::hardware::EngineHardware) -> Baking {
    let baking = Baking::default();
    let done = baking.clone();
    std::thread::spawn(move || {
        let maps = EngineMaps::bake(&hardware);
        if let Ok(mut done) = done.lock() {
            *done = Some(maps);
        }
    });
    baking
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::CarSetup;

    #[test]
    fn a_press_in_an_input_that_came_late_is_pressed_in_the_next_step() {
        let terrain = Terrain::flat();
        let setup = CarSetup::default();
        let spec = CarSpec::of(&setup);
        let mut shadow = Shadow {
            car: Some(spec_car(&spec, spec.stock_maps().unwrap())),
            offset: Some(0),
            ..default()
        };
        shadow.take_inputs(1, &[DriverInput::default()], 0);
        // Step 2's input isn't there in time: guessed, without R.
        for tick in 1..=2 {
            shadow.step(tick, &terrain);
            shadow.finish_step(&terrain);
        }
        assert!(!shadow.taken());
        // It comes after all, R pressed: pressed in step 3.
        let pressed = DriverInput {
            reset: true,
            ..default()
        };
        shadow.take_inputs(3, &[pressed, DriverInput::default()], 2);
        shadow.step(3, &terrain);
        shadow.finish_step(&terrain);
        assert!(shadow.taken(), "R was lost");
    }
}
