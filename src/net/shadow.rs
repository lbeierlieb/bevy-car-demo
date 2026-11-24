//! The server's own copy of each player's car, driven by the inputs the player sends,
//! to see how closely it follows the car the player drives (shadow mode: the player's
//! game is still in charge, nobody sees the copy). Once a second the player sends their
//! car's whole state after a numbered step (`client::CHECK_STEPS`); the copy's state after
//! the same step is compared with it and logged now and then (how many matched to the
//! bit, how far off the others were), then the copy takes the player's state and the
//! steps since are driven again from the inputs kept. That's what's needed later to put
//! the server in charge: the same car, the same inputs, the same results.
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
/// The most steps one copy is driven in a frame: a player who sent a burst (after a
/// stall) is caught up over a few frames.
const MOST_STEPS: usize = 60;
/// A step whose input never came is driven with the one before once this many later ones
/// are here (they don't come in order, but never this far out).
const GIVE_UP: usize = 24;
/// How often each copy's report is logged (s).
const REPORT: f64 = 10.0;
/// The most checks waiting for the copy to get to their step.
const MOST_WAITING: usize = 4;

/// Engine maps the server has baked for players' cars, by their hardware's fingerprint.
#[derive(Resource, Default)]
pub struct BakedMaps(HashMap<u64, EngineMaps>);

/// A player's car as the server drives it.
#[derive(Default)]
pub struct Shadow {
    /// What the car is built of, as the player last said.
    spec: Option<CarSpec>,
    /// Its engine's maps being baked (by their fingerprint), there when done.
    baking: Option<(u64, Baking)>,
    /// The copy, and the next step to drive it through (none until a check gave it a state).
    car: Option<Car>,
    next: u32,
    /// A car of the spec without a state yet (waiting for a check).
    built: Option<Car>,
    /// Inputs by step, those not driven yet and the last `KEPT` driven.
    inputs: BTreeMap<u32, DriverInput>,
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
    /// Steps driven with a stand-in for an input that didn't come.
    guessed: usize,
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

    /// The driver's inputs in the steps up to `last`, the latest last.
    pub fn take_inputs(&mut self, last: u32, inputs: &[DriverInput]) {
        let first = last.wrapping_sub(inputs.len() as u32 - 1);
        for (i, input) in inputs.iter().enumerate() {
            let step = first.wrapping_add(i as u32);
            // Ones already driven are kept as they were.
            if self.car.is_none() || step >= self.next {
                self.inputs.insert(step, *input);
            }
        }
        // Without a copy to drive, only the latest are any use.
        while self.car.is_none() && self.inputs.len() > KEPT as usize {
            self.inputs.pop_first();
        }
    }

    /// The player's car's whole state after `step`.
    pub fn check(&mut self, step: u32, state: Vec<u8>) {
        if self.car.is_none() {
            // The first state for the car built: it starts from here.
            if let Some(mut car) = self.built.take() {
                if snapshot::load(&mut car, &state) {
                    self.car = Some(car);
                    self.next = step.wrapping_add(1);
                } else {
                    // Not a state of this car (it changed meanwhile): wait for another.
                    self.built = Some(car);
                }
            }
            return;
        }
        if self.waiting.len() >= MOST_WAITING {
            self.waiting.pop_first();
        }
        self.waiting.insert(step, state);
    }

    /// Drives the copy through the steps whose inputs are here, compares it with the
    /// player's checks, and logs how it went now and then (as `who`'s car).
    pub fn run(&mut self, terrain: &Terrain, baked: &mut BakedMaps, now: f64, who: &str) {
        self.finish_baking(baked);
        let dt = STEP;
        let started = Instant::now();
        let mut driven = 0;
        while let Some(car) = &mut self.car
            && driven < MOST_STEPS
        {
            let input = match self.inputs.get(&self.next) {
                Some(&input) => input,
                None if self.inputs.range(self.next..).count() > GIVE_UP => {
                    self.report.guessed += 1;
                    let before = self.inputs.range(..self.next).next_back();
                    before.map_or_else(DriverInput::default, |(_, &input)| input)
                }
                None => break,
            };
            car.drive(&input, dt);
            step_car(car, terrain, dt);
            if self.next.is_multiple_of(CHECK_STEPS) {
                self.ours.insert(self.next, snapshot::save(car));
            }
            self.inputs.insert(self.next, input);
            self.next = self.next.wrapping_add(1);
            driven += 1;
        }
        self.report.steps += driven;
        self.report.time += started.elapsed();
        self.compare(terrain);
        let next = self.next;
        self.inputs
            .retain(|&step, _| step.wrapping_add(KEPT) >= next);
        self.ours.retain(|&step, _| step.wrapping_add(KEPT) >= next);
        self.log(now, who);
    }

    /// Compares the checks the copy has got to with its own states, and takes up the
    /// player's state from the latest, driving the steps since again.
    fn compare(&mut self, terrain: &Terrain) {
        let Some(car) = &mut self.car else {
            return;
        };
        let ready: Vec<u32> = (self.waiting.keys())
            .copied()
            .filter(|&step| step < self.next)
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
            }
            latest = Some((step, theirs));
        }
        // The player's game is in charge: the copy goes on from its state.
        let Some((step, state)) = latest else {
            return;
        };
        if !snapshot::load(car, &state) {
            return;
        }
        let dt = STEP;
        let mut at = step.wrapping_add(1);
        while at != self.next {
            let Some(input) = self.inputs.get(&at) else {
                break;
            };
            car.drive(input, dt);
            step_car(car, terrain, dt);
            at = at.wrapping_add(1);
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
                 without their input",
                report.exact,
                report.checks,
                report.position,
                report.velocity,
                report.rpm,
                report.steps,
                report.guessed,
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
