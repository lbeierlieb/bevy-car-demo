//! The server's own copy of each player's car, driven by the inputs the player sends,
//! to see how closely it follows the car the player drives (shadow mode: the player's
//! game is still in charge, nobody sees the copy).
//!
//! The copies run on the server's clock, one step per server step (`Tick`), all at once,
//! as they will once the server is in charge and cars meet on it. A player's steps are
//! numbered by their game: the copy drives step `tick + offset`, the offset set when their
//! inputs first come so that they come `MARGIN` steps before they're needed. How early
//! they come is told back to the player (`ToClient::Pace`), whose game then runs its
//! clock a little faster or slower to keep them coming just in time. An input that isn't
//! there in time is guessed (the one before), as it will be when the server's in charge.
//!
//! Once a second the player sends their car's whole state after a numbered step
//! (`client::CHECK_STEPS`); the copy's state after the same step is compared with it and
//! logged now and then (how many matched to the bit, how far off the others were), then
//! the copy takes the player's state and the steps since are driven again from the inputs
//! kept (the real ones, where some came too late).
//!
//! A car built of an engine whose maps don't come with the game is baked on a thread of
//! its own (seconds); meanwhile there's no copy.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::prelude::*;

use super::client::CHECK_STEPS;
use crate::car::{Car, DriverInput};
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
const TOO_LATE: i32 = 30;
const TOO_EARLY: i32 = 120;
/// Without any input for this many steps, the copy is driven as if nobody's at the wheel.
const SILENT: u32 = 30;
/// How often each copy's report is logged (s).
const REPORT: f64 = 10.0;
/// The most checks waiting for the copy to get to their step.
const MOST_WAITING: usize = 4;

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
    /// How early their inputs came lately: the least, in steps (for `ToClient::Pace`).
    early: Option<i32>,
    /// The copy's state after each check step it's driven through, not yet compared.
    ours: BTreeMap<u32, Vec<u8>>,
    /// The player's checks for steps the copy hasn't got to yet.
    waiting: BTreeMap<u32, Vec<u8>>,
    report: Report,
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
        if self.spec.as_ref() == Some(&spec) {
            return;
        }
        self.car = None;
        self.built = None;
        self.baking = None;
        self.ours.clear();
        self.waiting.clear();
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
            self.inputs.insert(first.wrapping_add(i as u32), *input);
        }
        if self.latest.is_none_or(|latest| after(last, latest) > 0) {
            self.latest = Some(last);
        }
        // How early the latest came: the steps until it's driven.
        let early = self
            .offset
            .map(|offset| after(last.wrapping_sub(offset), tick));
        match early {
            Some(early) if (-TOO_LATE..=TOO_EARLY).contains(&early) => {
                self.early = Some(self.early.map_or(early, |least| least.min(early)));
            }
            _ => {
                // First inputs, or a game far off: from now on they come just in time.
                if early.is_some() {
                    self.report.anchored += 1;
                    // The copy's steps jump: it waits for the next check.
                    if let Some(car) = self.car.take() {
                        self.built = Some(car);
                    }
                }
                self.offset = Some(last.wrapping_sub(tick.wrapping_add(MARGIN as u32)));
                self.early = Some(MARGIN);
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

    /// The player's car's whole state after `step`.
    pub fn check(&mut self, step: u32, state: Vec<u8>) {
        if self.waiting.len() >= MOST_WAITING {
            self.waiting.pop_first();
        }
        self.waiting.insert(step, state);
    }

    /// Server step `tick`: drives the copy through the player's step for it.
    pub fn step(&mut self, tick: u32, terrain: &Terrain) {
        let (Some(car), Some(offset)) = (&mut self.car, self.offset) else {
            return;
        };
        let step = tick.wrapping_add(offset);
        if after(step, self.done) != 1 {
            // Not the step after the last (the offset changed): wait for a check.
            self.built = self.car.take();
            return;
        }
        let started = Instant::now();
        let input = match self.inputs.get(&step) {
            Some(&input) => input,
            None => {
                self.report.guessed += 1;
                let input = guess(&self.inputs, self.latest, step);
                self.inputs.insert(step, input);
                input
            }
        };
        car.drive(&input, STEP);
        step_car(car, terrain, STEP);
        self.done = step;
        if step.is_multiple_of(CHECK_STEPS) {
            self.ours.insert(step, snapshot::save(car));
        }
        let report = &mut self.report;
        report.steps += 1;
        report.time += started.elapsed();
        self.inputs
            .retain(|&kept, _| after(step, kept) < KEPT as i32);
        self.ours.retain(|&kept, _| after(step, kept) < KEPT as i32);
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
        self.start(tick, terrain);
        self.compare(terrain);
        self.log(now, who);
    }

    /// A copy waiting for a state takes the latest check, and is driven from it to the
    /// step it's due at (from the inputs kept, guessing the ones not here).
    fn start(&mut self, tick: u32, terrain: &Terrain) {
        if self.car.is_some() || self.built.is_none() {
            return;
        }
        // The step the copy should have been driven through by now.
        let Some(done) = self.offset.map(|offset| tick.wrapping_add(offset)) else {
            return;
        };
        // The latest check the copy's due step has got to (later ones wait for it).
        self.waiting
            .retain(|&step, _| after(done, step) <= KEPT as i32);
        let Some(&step) = (self.waiting.keys())
            .rev()
            .find(|&&step| after(done, step) >= 0)
        else {
            return;
        };
        let state = self.waiting.remove(&step).expect("the check");
        self.waiting.retain(|&later, _| after(later, step) > 0);
        let mut car = self.built.take().expect("a car built");
        if !snapshot::load(&mut car, &state) {
            // Not a state of this car (it changed meanwhile): wait for another.
            self.built = Some(car);
            return;
        }
        self.ours.clear();
        self.redrive(&mut car, step, done, terrain);
        self.car = Some(car);
        self.done = done;
    }

    /// Compares the checks the copy has got to with its own states, and takes up the
    /// player's state from the latest, driving the steps since again.
    fn compare(&mut self, terrain: &Terrain) {
        let Some(car) = &self.car else {
            return;
        };
        let done = self.done;
        let ready: Vec<u32> = (self.waiting.keys())
            .copied()
            .filter(|&step| after(done, step) >= 0)
            .collect();
        let mut latest = None;
        for step in ready {
            let theirs = self.waiting.remove(&step).expect("a waiting check");
            let Some(ours) = self.ours.remove(&step) else {
                continue;
            };
            let report = &mut self.report;
            report.checks += 1;
            if ours == theirs {
                report.exact += 1;
            } else {
                let mut a = car.clone();
                let mut b = car.clone();
                if snapshot::load(&mut a, &ours) && snapshot::load(&mut b, &theirs) {
                    let off = |x: Vec3, y: Vec3| x.distance(y);
                    report.position = (report.position).max(off(a.body.position, b.body.position));
                    report.velocity = (report.velocity).max(off(a.body.velocity, b.body.velocity));
                    let rpm = (a.drivetrain.rpm - b.drivetrain.rpm).abs();
                    report.rpm = report.rpm.max(rpm);
                }
                latest = Some((step, theirs));
            }
        }
        // The player's game is in charge: the copy goes on from its state.
        let Some((step, state)) = latest else {
            return;
        };
        let mut car = self.car.take().expect("a copy");
        if snapshot::load(&mut car, &state) {
            self.redrive(&mut car, step, done, terrain);
        }
        self.car = Some(car);
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
                 without their input in time, the offset set afresh {} times",
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

    /// How many of the checks so far matched to the bit, of how many (for tests).
    #[cfg(test)]
    pub fn matched(&self) -> (usize, usize) {
        (self.report.exact, self.report.checks)
    }
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
            ..input
        },
        _ => DriverInput {
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
