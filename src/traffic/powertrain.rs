//! What a traffic car's engine does, for its sound: a gearbox the driver shifts through
//! and the load the engine is under, worked out from how the car moves (no drivetrain is
//! simulated). Each chassis has its factory gears, final drive and wheels, and its base
//! engine's full-throttle torque curve (`Powertrain`, made when first needed); each
//! driver a `Shifter` that picks the gear by its own shift points and turns the force the
//! car needs at the wheels into rpm and load.

use std::collections::HashMap;
use std::f32::consts::PI;
use std::sync::{Arc, Mutex};

use crate::audio::EngineRun;
use crate::car::CarSetup;
use crate::catalogue::Shared;
use crate::physics::drivetrain::Drive;
use crate::physics::engine::Engine;
use crate::physics::engine::table::curve;

/// Share of the engine's torque that reaches the wheels.
const EFFICIENCY: f32 = 0.9;
/// Air density (kg/m³) and the tires' rolling resistance (share of the weight).
const AIR: f32 = 1.2;
const ROLLING: f32 = 0.012;
const GRAVITY: f32 = 9.81;
/// The torque curve has a point every this many rpm; each is measured after this long at
/// full throttle (the first longer, for the turbo to spool and the engine to settle).
const CURVE_STEP: f32 = 500.0;
const SETTLE: f32 = 0.75;
const FIRST_SETTLE: f32 = 2.0;
/// How quickly the engine's rpm and load follow (s).
const RPM_LAG: f32 = 0.12;
const LOAD_LAG: f32 = 0.1;
/// How quickly the turbo spools up and down (s), and how fast it spins at full load at
/// the redline (share of its limit).
const SPOOL_UP: f32 = 0.8;
const SPOOL_DOWN: f32 = 0.4;
const TURBO_SHARE: f32 = 0.75;
/// Pulling hard, a driver shifts up from somewhere between these shares of the band
/// between idle and the redline (the keener the higher); the less of the engine's torque
/// it would take, the earlier, down to the lowest the next gear allows when it takes none
/// and all the way up at `PULLING_HARD`; wanting more, later, up to `FLOORED` of the band
/// at twice that; it shifts down below idle plus `SHIFT_DOWN` (rpm).
const SHIFT_UP: (f32, f32) = (0.22, 0.45);
const PULLING_HARD: f32 = 0.4;
const FLOORED: f32 = 0.7;
const SHIFT_DOWN: f32 = 450.0;
/// The next gear must leave this much over the down shift point (rpm).
const SHIFT_MARGIN: f32 = 300.0;
/// Least time between shifts (s).
const SHIFT_HOLD: f32 = 1.0;
/// Pulling away, the clutch slips with the engine at idle plus this (rpm), more for keen
/// drivers.
const LAUNCH: (f32, f32) = (500.0, 1100.0);
/// The most of the engine's torque a driver uses, from the calmest to the keenest, but
/// as much as it takes to speed up at this at least (m/s²; in weak or heavy cars).
const POWER_SHARE: (f32, f32) = (0.45, 0.8);
const LEAST_PULL: f32 = 0.6;

/// A chassis' factory drivetrain, as far as its sound needs.
pub struct Powertrain {
    gears: Vec<f32>,
    final_drive: f32,
    /// Rolling radius of the driven wheels (m).
    radius: f32,
    idle: f32,
    redline: f32,
    /// How long a shift takes (s).
    shift_time: f32,
    /// Drag coefficient times frontal area (m²).
    drag_area: f32,
    /// The turbo's limit (rad/s), 0 without one.
    turbo_limit: f32,
    /// Full-throttle torque (rpm, N·m).
    torque: Vec<(f32, f32)>,
}

impl Powertrain {
    /// The factory drivetrain of `catalogue`'s `chassis` with its base engine (worked out
    /// once).
    pub fn factory(catalogue: &Shared, chassis: usize) -> Arc<Self> {
        type Made = HashMap<(u64, usize), Arc<Powertrain>>;
        static FACTORY: Mutex<Option<Made>> = Mutex::new(None);
        let key = (catalogue.fingerprint(), chassis);
        if let Some(made) = FACTORY
            .lock()
            .ok()
            .and_then(|made| made.as_ref()?.get(&key).cloned())
        {
            return made;
        }
        let made = Arc::new(Self::of(&CarSetup::factory_in(catalogue, chassis, 0)));
        if let Ok(mut all) = FACTORY.lock() {
            all.get_or_insert_default().insert(key, made.clone());
        }
        made
    }

    fn of(setup: &CarSetup) -> Self {
        let engine = &setup.engine;
        let build = &engine.build;
        let redline = build.hardware.redline();
        let mut torque = vec![(0.0, 0.0)];
        let mut state = Engine::default();
        let mut rpm = CURVE_STEP;
        let mut settle = FIRST_SETTLE;
        while rpm <= redline + CURVE_STEP {
            for _ in 0..(settle * 120.0) as usize {
                state.step(build, 1.0, rpm, true, 1.0 / 120.0);
            }
            torque.push((rpm, state.torque(build, rpm).max(0.0)));
            rpm += CURVE_STEP;
            settle = SETTLE;
        }
        let radius = match engine.drive {
            Drive::Front => setup.front.tire.radius,
            Drive::Rear => setup.rear.tire.radius,
            Drive::All { .. } => (setup.front.tire.radius + setup.rear.tire.radius) / 2.0,
        };
        Self {
            gears: engine.gears.clone(),
            final_drive: engine.final_drive,
            radius,
            idle: build.ecu.idle_rpm,
            redline,
            shift_time: engine.shift_time + engine.engage_time,
            drag_area: setup.drag_area,
            turbo_limit: build.hardware.turbo.map_or(0.0, |turbo| turbo.max_speed()),
            torque,
        }
    }

    /// Most torque the engine makes at `rpm` (N·m).
    fn torque_at(&self, rpm: f32) -> f32 {
        curve(&self.torque, rpm)
    }

    /// The engine's rpm in `gear` (from 0) at `speed` (m/s), the clutch in.
    fn locked(&self, gear: usize, speed: f32) -> f32 {
        speed / self.radius * self.gears[gear] * self.final_drive * 30.0 / PI
    }

    /// The engine torque for a force at the wheels in `gear` (N·m).
    fn engine_torque(&self, gear: usize, force: f32) -> f32 {
        force * self.radius / (self.gears[gear] * self.final_drive * EFFICIENCY)
    }

    /// The force at the wheels a car of `mass` needs to speed up at `acceleration` going
    /// at `speed` up a `grade` (the sine of the slope) (N).
    fn force(&self, mass: f32, speed: f32, acceleration: f32, grade: f32) -> f32 {
        mass * (acceleration + GRAVITY * (grade + ROLLING))
            + 0.5 * AIR * self.drag_area * speed * speed
    }
}

/// A traffic driver's gear and what the engine does.
pub struct Shifter {
    powertrain: Arc<Powertrain>,
    /// How keen the driver is (0 to 1): shifts later, pulls away at higher rpm.
    eager: f32,
    gear: usize,
    /// Time since the last shift (s).
    since: f32,
    rpm: f32,
    load: f32,
    turbo: f32,
}

impl Shifter {
    pub fn new(powertrain: Arc<Powertrain>, eager: f32) -> Self {
        Self {
            rpm: powertrain.idle,
            powertrain,
            eager: eager.clamp(0.0, 1.0),
            gear: 0,
            since: SHIFT_HOLD,
            load: 0.0,
            turbo: 0.0,
        }
    }

    /// Starts off in the gear for `speed` (m/s), as if it had been driving.
    pub fn cruise(&mut self, speed: f32) {
        let p = self.powertrain.clone();
        while self.gear + 1 < p.gears.len() && p.locked(self.gear, speed) > self.up_rpm(0.2) {
            self.gear += 1;
        }
        self.rpm = p.locked(self.gear, speed).max(p.idle);
    }

    /// Where it shifts up with the engine making `demand` of what it could.
    fn up_rpm(&self, demand: f32) -> f32 {
        let p = self.powertrain.clone();
        let share = SHIFT_UP.0 + (SHIFT_UP.1 - SHIFT_UP.0) * self.eager;
        let band = p.redline - p.idle;
        let hard = p.idle + share * band;
        let easy = self.down_rpm() + 2.0 * SHIFT_MARGIN;
        let floored = p.idle + FLOORED * band;
        let t = demand / PULLING_HARD;
        if t < 1.0 {
            easy + (hard - easy).max(0.0) * t.max(0.0)
        } else {
            hard + (floored - hard).max(0.0) * (t - 1.0).min(1.0)
        }
    }

    fn down_rpm(&self) -> f32 {
        self.powertrain.idle + SHIFT_DOWN
    }

    /// The most of the engine's torque this driver uses.
    fn share(&self) -> f32 {
        POWER_SHARE.0 + (POWER_SHARE.1 - POWER_SHARE.0) * self.eager
    }

    /// The engine's rpm pulling away in first, the clutch slipping.
    fn launch(&self) -> f32 {
        self.powertrain.idle + LAUNCH.0 + (LAUNCH.1 - LAUNCH.0) * self.eager
    }

    /// The most a car of `mass` going at `speed` (m/s) up a `grade` (the sine of the
    /// slope) speeds up in the gear it's in with as much of the engine as the driver
    /// uses: less up hills and in heavy, weak or tall-geared cars, and nothing pulls
    /// while it shifts (m/s², negative when it slows down even so).
    pub fn most_acceleration(&self, mass: f32, speed: f32, grade: f32) -> f32 {
        let p = self.powertrain.clone();
        let drag = p.force(mass, speed, 0.0, grade);
        if self.since < p.shift_time {
            return -drag / mass;
        }
        let gear = self.gear;
        let mut rpm = p.locked(gear, speed).max(p.idle);
        if gear == 0 {
            rpm = rpm.max(self.launch());
        }
        let full = p.torque_at(rpm) * p.gears[gear] * p.final_drive * EFFICIENCY / p.radius;
        let calm = (self.share() * full - drag) / mass;
        calm.max(LEAST_PULL.min((full - drag) / mass))
    }

    /// One step of a car of `mass` going at `speed` (m/s), speeding up at `acceleration`
    /// (m/s², negative braking) up a `grade` (the sine of the slope), when the driver
    /// would like `wanted` (shifting down when the gear it's in can't give it).
    pub fn step(
        &mut self,
        mass: f32,
        speed: f32,
        (acceleration, wanted): (f32, f32),
        grade: f32,
        dt: f32,
    ) {
        let p = self.powertrain.clone();
        let force = p.force(mass, speed, acceleration, grade);
        let wanted = p.force(mass, speed, wanted, grade);
        self.since += dt;
        if speed < 0.5 {
            self.gear = 0;
        } else if self.since > SHIFT_HOLD {
            // How much of the engine's torque the driver would use in a gear.
            let demand = |gear: usize| {
                let rpm = p.locked(gear, speed).max(p.idle);
                p.engine_torque(gear, wanted) / p.torque_at(rpm).max(1.0)
            };
            let fits = |gear: usize| demand(gear) < self.share();
            let down = self.down_rpm();
            let next = self.gear + 1;
            let locked = p.locked(self.gear, speed);
            if next < p.gears.len()
                && locked > self.up_rpm(demand(self.gear))
                && p.locked(next, speed) > down + SHIFT_MARGIN
            {
                self.gear = next;
                self.since = 0.0;
            } else if self.gear > 0
                && (locked < down
                    || (!fits(self.gear)
                        && p.locked(self.gear - 1, speed)
                            < self.up_rpm(demand(self.gear - 1)) - SHIFT_MARGIN))
            {
                self.gear -= 1;
                self.since = 0.0;
            }
        }
        let shifting = self.since < p.shift_time;
        let locked = p.locked(self.gear, speed);
        // Pulling away in first, the clutch slips; anywhere else below idle it's apart.
        let launch = self.launch();
        let target = if self.gear == 0 && force > 0.0 && acceleration > 0.05 {
            locked.max(launch)
        } else {
            locked.max(p.idle)
        };
        let load = if shifting || force <= 0.0 || locked < p.idle && self.gear > 0 {
            0.0
        } else {
            (p.engine_torque(self.gear, force) / p.torque_at(self.rpm).max(1.0)).clamp(0.0, 1.0)
        };
        self.rpm += (target - self.rpm) * (1.0 - (-dt / RPM_LAG).exp());
        self.load += (load - self.load) * (1.0 - (-dt / LOAD_LAG).exp());
        if p.turbo_limit > 0.0 {
            let spin = TURBO_SHARE * p.turbo_limit * (self.rpm / p.redline).min(1.0).sqrt();
            let target = spin * (0.15 + 0.85 * self.load);
            let lag = if target > self.turbo {
                SPOOL_UP
            } else {
                SPOOL_DOWN
            };
            self.turbo += (target - self.turbo) * (1.0 - (-dt / lag).exp());
        }
    }

    /// What the engine does now.
    pub fn run(&self) -> EngineRun {
        EngineRun {
            running: true,
            rpm: self.rpm,
            fill: crate::physics::drivetrain::fill_for(self.load),
            turbo_speed: self.turbo,
        }
    }
}

/// An engine idling, for a car whose driver's gone.
pub fn idling(catalogue: &Shared, chassis: usize) -> EngineRun {
    EngineRun {
        running: true,
        rpm: Powertrain::factory(catalogue, chassis).idle,
        fill: crate::physics::drivetrain::fill_for(0.0),
        turbo_speed: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::drivetrain::firing_load;

    /// A driver wanting `wanted` (m/s²) up to `top` (m/s) on a `grade` for `seconds`:
    /// the speeds it reached each second, and the gears it shifted into on the way.
    fn drive(shifter: &mut Shifter, mass: f32, grade: f32, top: f32, seconds: f32) -> Drive {
        let dt = 1.0 / 60.0;
        let mut drive = Drive::default();
        let mut speed = 0.0;
        let mut gear = shifter.gear;
        for i in 0..(seconds / dt) as usize {
            let wanted = 1.4 * (1.0 - (speed / top).powi(4));
            let a = wanted.min(shifter.most_acceleration(mass, speed, grade));
            let before = speed;
            speed = (speed + a * dt).max(0.0);
            shifter.step(mass, speed, ((speed - before) / dt, wanted), grade, dt);
            if shifter.gear != gear {
                gear = shifter.gear;
                drive.shifts.push((gear + 1, (speed * 3.6).round()));
            }
            drive.low = drive.low.min(shifter.rpm);
            drive.high = drive.high.max(shifter.rpm);
            if drive.to_100.is_none() && speed >= 100.0 / 3.6 {
                drive.to_100 = Some(i as f32 * dt);
            }
        }
        drive.speed = speed;
        drive
    }

    struct Drive {
        shifts: Vec<(usize, f32)>,
        low: f32,
        high: f32,
        to_100: Option<f32>,
        speed: f32,
    }

    impl Default for Drive {
        fn default() -> Self {
            Self {
                shifts: Vec::new(),
                low: f32::INFINITY,
                high: 0.0,
                to_100: None,
                speed: 0.0,
            }
        }
    }

    /// Every chassis pulls away, shifts up through the gears with the rpm between idle
    /// and the redline, gets to 100 km/h (the weak and heavy ones later) and cruises
    /// in a high gear at a light load; up a 10 % slope it still does 50.
    #[test]
    fn traffic_shifts_through_the_gears() {
        let start = std::time::Instant::now();
        let catalogue = crate::catalogue::builtin();
        for chassis in 0..catalogue.chassis.len() {
            Powertrain::factory(&catalogue, chassis);
        }
        println!("torque curves in {:?}", start.elapsed());
        let mut times = Vec::new();
        for (chassis, model) in catalogue.chassis.iter().enumerate() {
            let p = Powertrain::factory(&catalogue, chassis);
            let mass = model.mass + 230.0;
            let peak = p.torque.iter().map(|t| t.1).fold(0.0, f32::max);
            for eager in [0.0, 1.0] {
                let run = drive(
                    &mut Shifter::new(p.clone(), eager),
                    mass,
                    0.0,
                    130.0 / 3.6,
                    50.0,
                );
                let mut shifter = Shifter::new(p.clone(), eager);
                drive(&mut shifter, mass, 0.0, 100.0 / 3.6, 80.0);
                let cruise = shifter.run();
                let mut climber = Shifter::new(p.clone(), eager);
                let climb = drive(&mut climber, mass, 0.1, 60.0 / 3.6, 40.0);
                println!(
                    "{:10} {peak:4.0} N·m, eager {eager}: 0–100 in {:4.1} s, shifts {:?}, \
                     {:.0}–{:.0} rpm; at 100 in {} at {:.0} rpm, load {:.2}; \
                     up 10 %: {:.0} km/h in {}",
                    model.name,
                    run.to_100.unwrap_or(f32::NAN),
                    run.shifts,
                    run.low,
                    run.high,
                    shifter.gear + 1,
                    cruise.rpm,
                    firing_load(cruise.fill),
                    climb.speed * 3.6,
                    climber.gear + 1,
                );
                assert!(
                    run.low >= p.idle - 1.0 && run.high < p.redline,
                    "{}",
                    model.name
                );
                assert!(
                    run.shifts.windows(2).all(|w| w[1].0 == w[0].0 + 1),
                    "{} shifts only up",
                    model.name
                );
                assert!(run.to_100.is_some_and(|t| t < 40.0), "{}", model.name);
                assert!(shifter.gear + 3 >= p.gears.len(), "{}", model.name);
                let load = firing_load(cruise.fill);
                assert!(load > 0.05 && load < 0.5, "{}", model.name);
                assert!(climb.speed > 50.0 / 3.6, "{}", model.name);
                times.push((model.name.as_str(), eager, run.to_100.unwrap()));
            }
        }
        let time = |name, eager| {
            (times.iter())
                .find(|t| t.0 == name && t.1 == eager)
                .unwrap()
                .2
        };
        assert!(time("Vireo", 1.0) < time("Vireo", 0.0));
    }
}
