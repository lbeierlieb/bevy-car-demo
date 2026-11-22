//! What a traffic car's engine does, for its sound: a gearbox the driver shifts through
//! and the load the engine is under, worked out from how the car moves (no drivetrain is
//! simulated). Each chassis has its factory gears, final drive and wheels, and its base
//! engine's full-throttle torque curve (`Powertrain`, made when first needed); each
//! driver a `Shifter` that picks the gear by its own shift points and turns the force the
//! car needs at the wheels into rpm and load.

use std::f32::consts::PI;
use std::sync::OnceLock;

use crate::audio::EngineRun;
use crate::car::CarSetup;
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
/// it takes, the earlier, down to the lowest the next gear allows when it takes none and
/// all the way up at this share; it shifts down below idle plus `SHIFT_DOWN` (rpm).
const SHIFT_UP: (f32, f32) = (0.22, 0.45);
const PULLING_HARD: f32 = 0.4;
const SHIFT_DOWN: f32 = 450.0;
/// The next gear must leave this much over the down shift point (rpm).
const SHIFT_MARGIN: f32 = 300.0;
/// Least time between shifts (s).
const SHIFT_HOLD: f32 = 1.0;
/// Pulling away, the clutch slips with the engine at idle plus this (rpm), more for keen
/// drivers.
const LAUNCH: (f32, f32) = (500.0, 1100.0);

/// A chassis' factory drivetrain, as far as its sound needs.
pub struct Powertrain {
    gears: &'static [f32],
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
    /// The factory drivetrain of `chassis` (in `CHASSIS`) with its base engine.
    pub fn factory(chassis: usize) -> &'static Self {
        static FACTORY: [OnceLock<Powertrain>; 13] = [const { OnceLock::new() }; 13];
        FACTORY[chassis].get_or_init(|| Self::of(&CarSetup::factory(chassis, 0)))
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
            gears: engine.gears,
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
    powertrain: &'static Powertrain,
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
    pub fn new(powertrain: &'static Powertrain, eager: f32) -> Self {
        Self {
            powertrain,
            eager: eager.clamp(0.0, 1.0),
            gear: 0,
            since: SHIFT_HOLD,
            rpm: powertrain.idle,
            load: 0.0,
            turbo: 0.0,
        }
    }

    /// Starts off in the gear for `speed` (m/s), as if it had been driving.
    pub fn cruise(&mut self, speed: f32) {
        let p = self.powertrain;
        while self.gear + 1 < p.gears.len() && p.locked(self.gear, speed) > self.up_rpm(0.2) {
            self.gear += 1;
        }
        self.rpm = p.locked(self.gear, speed).max(p.idle);
    }

    /// Where it shifts up with the engine making `demand` of what it could.
    fn up_rpm(&self, demand: f32) -> f32 {
        let p = self.powertrain;
        let share = SHIFT_UP.0 + (SHIFT_UP.1 - SHIFT_UP.0) * self.eager;
        let hard = p.idle + share * (p.redline - p.idle);
        let easy = self.down_rpm() + 2.0 * SHIFT_MARGIN;
        easy + (hard - easy).max(0.0) * (demand / PULLING_HARD).clamp(0.0, 1.0)
    }

    fn down_rpm(&self) -> f32 {
        self.powertrain.idle + SHIFT_DOWN
    }

    /// One step of a car of `mass` going at `speed` (m/s), speeding up at `acceleration`
    /// (m/s², negative braking) up a `grade` (the sine of the slope).
    pub fn step(&mut self, mass: f32, speed: f32, acceleration: f32, grade: f32, dt: f32) {
        let p = self.powertrain;
        let force = p.force(mass, speed, acceleration, grade);
        self.since += dt;
        if speed < 0.5 {
            self.gear = 0;
        } else if self.since > SHIFT_HOLD {
            let demand = |gear: usize| {
                let rpm = p.locked(gear, speed).max(p.idle);
                p.engine_torque(gear, force) / p.torque_at(rpm).max(1.0)
            };
            let fits = |gear: usize| demand(gear) < 0.85;
            let (up, down) = (self.up_rpm(1.0), self.down_rpm());
            let next = self.gear + 1;
            if next < p.gears.len()
                && p.locked(self.gear, speed) > self.up_rpm(demand(self.gear))
                && p.locked(next, speed) > down + SHIFT_MARGIN
                && fits(next)
            {
                self.gear = next;
                self.since = 0.0;
            } else if self.gear > 0
                && (p.locked(self.gear, speed) < down
                    || (!fits(self.gear) && p.locked(self.gear - 1, speed) < up))
            {
                self.gear -= 1;
                self.since = 0.0;
            }
        }
        let shifting = self.since < p.shift_time;
        let locked = p.locked(self.gear, speed);
        // Pulling away in first, the clutch slips; anywhere else below idle it's apart.
        let launch = p.idle + LAUNCH.0 + (LAUNCH.1 - LAUNCH.0) * self.eager;
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
            load: self.load,
            turbo_speed: self.turbo,
        }
    }

    /// The gear it's in (from 0).
    #[cfg(test)]
    pub fn gear(&self) -> usize {
        self.gear
    }
}

/// An engine idling, for a car whose driver's gone.
pub fn idling(chassis: usize) -> EngineRun {
    EngineRun {
        running: true,
        rpm: Powertrain::factory(chassis).idle,
        load: 0.0,
        turbo_speed: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chassis::CHASSIS;

    /// Every chassis pulls away, shifts up through the gears with the rpm between idle
    /// and the redline, and cruises at 100 km/h in a high gear at a light load.
    #[test]
    fn traffic_shifts_through_the_gears() {
        let start = std::time::Instant::now();
        for chassis in 0..CHASSIS.len() {
            Powertrain::factory(chassis);
        }
        println!("torque curves in {:?}", start.elapsed());
        let dt = 1.0 / 60.0;
        for (chassis, model) in CHASSIS.iter().enumerate() {
            let p = Powertrain::factory(chassis);
            let mass = model.mass + 230.0;
            let peak = p.torque.iter().map(|t| t.1).fold(0.0, f32::max);
            for eager in [0.0, 1.0] {
                let mut shifter = Shifter::new(p, eager);
                let (mut speed, mut gear, mut shifts) = (0.0, 0, Vec::new());
                let (mut low, mut high) = (f32::INFINITY, 0.0f32);
                while speed < 100.0 / 3.6 {
                    speed += 1.4 * dt;
                    shifter.step(mass, speed, 1.4, 0.0, dt);
                    if shifter.gear() != gear {
                        assert_eq!(shifter.gear(), gear + 1, "{}", model.name);
                        gear = shifter.gear();
                        shifts.push((speed * 3.6).round());
                    }
                    low = low.min(shifter.rpm);
                    high = high.max(shifter.rpm);
                }
                for _ in 0..600 {
                    shifter.step(mass, speed, 0.0, 0.0, dt);
                }
                let run = shifter.run();
                println!(
                    "{:24} {peak:4.0} N·m, eager {eager}: shifts at {shifts:?} km/h, \
                     {low:.0}–{high:.0} rpm; 100 km/h in {} at {:.0} rpm, load {:.2}",
                    model.name,
                    shifter.gear() + 1,
                    run.rpm,
                    run.load
                );
                assert!(low >= p.idle - 1.0 && high < p.redline, "{}", model.name);
                assert!(shifter.gear() + 3 >= p.gears.len(), "{}", model.name);
                assert!(run.load > 0.05 && run.load < 0.5, "{}", model.name);
            }
        }
    }
}
