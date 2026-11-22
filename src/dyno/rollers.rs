//! A chassis dyno with a roller under each axle, the rollers coupled, so a car with any
//! drive sits on it. It's an inertia dyno: the rollers stand in for the car's mass, and the
//! power at the wheels is what it takes to spin them up (their inertia times their
//! acceleration times their speed). A run goes up through the gears on part throttle,
//! revving each only as far as the next gear needs to land just below the start of the
//! pull, and once in fourth (or the top gear, if lower) pulls at full throttle to the rev
//! limiter.
//!
//! The car runs its own drivetrain and engine, so a run wears it, burns its fuel, and can
//! break it. The power at the crank is the engine's own; the wheels get it less the
//! gearbox and differentials' losses and the tires' rolling on the rollers, and some goes
//! to spinning up the engine and the wheels.

use std::f32::consts::TAU;

use super::chart::HORSEPOWER;
use crate::car::Car;
use crate::physics::drivetrain::Drive;

/// The dyno's step (s).
pub const STEP: f32 = 1.0 / 120.0;
/// How long the engine idles before the run (s).
const IDLE: f32 = 1.5;
/// The gear pulled in (or the top gear if lower), where it starts sampling (rpm, or this
/// far above idle if higher) after opening the throttle this much lower (rpm), how far
/// below the rev limiter a gear is shifted up at the latest and the pull ends (rpm), and
/// how often it samples (rpm).
const PULL_GEAR: i8 = 4;
const START_RPM: f32 = 2000.0;
const RUN_IN: f32 = 300.0;
const START_ABOVE_IDLE: f32 = 700.0;
const SHIFT_MARGIN: f32 = 400.0;
const END_MARGIN: f32 = 100.0;
const SAMPLE_RPM: f32 = 100.0;
/// The throttle going up through the gears.
const WARM_UP_THROTTLE: f32 = 0.4;
/// The longest a pull may take (s): a car that can't get to the rev limiter (a slipping
/// clutch, a dying engine) is stopped.
const LONGEST_PULL: f32 = 60.0;
/// How hard the rollers' brake slows them after the pull, as the car's deceleration
/// (m/s²).
const ROLLER_BRAKE: f32 = 7.0;
/// Torque lost in the gearbox and differentials, as a share of what goes through: with
/// two driven axles, more.
const GEAR_LOSS: f32 = 0.06;
const AWD_GEAR_LOSS: f32 = 0.1;
/// Air the dyno cell's fan blows through the radiator (m/s).
const FAN_AIRSPEED: f32 = 20.0;
/// How quickly the live power reading follows (s).
const READING_LAG: f32 = 0.25;
/// How quickly a hold brings the rollers to its speed, at the tires (m/s²).
const HOLD_RATE: f32 = 6.0;
const GRAVITY: f32 = 9.81;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// Idling in neutral.
    Idle,
    /// Up through the gears on part throttle, to the gear before the pull gear.
    Gears,
    /// Shifting into the pull gear.
    Settle,
    /// Full throttle to the rev limiter, sampling.
    Pull,
    /// Off the throttle in neutral, the rollers braked to a stop.
    /// Held at an rpm in the pull gear by the brake, at a set throttle, until stopped.
    Hold,
    Coast,
    Done,
}

/// A point of a pull: engine rpm (from the rollers' speed and the gear), torque at the
/// crank (N·m) and power at the wheels (W).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Sample {
    pub rpm: f32,
    pub crank: f32,
    pub wheel: f32,
}

impl Sample {
    /// Engine speed (rad/s).
    fn omega(&self) -> f32 {
        self.rpm * TAU / 60.0
    }

    pub fn crank_hp(&self) -> f32 {
        self.crank * self.omega() / HORSEPOWER
    }

    pub fn wheel_hp(&self) -> f32 {
        self.wheel / HORSEPOWER
    }

    /// The power at the wheels over the engine's speed: torque at the wheels as an engine
    /// would make it (N·m), as dyno sheets show it.
    pub fn wheel_torque(&self) -> f32 {
        self.wheel / self.omega()
    }
}

/// What a hold holds: the engine's speed in the pull gear (rpm) and the throttle (0..1).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Hold {
    pub rpm: f32,
    pub throttle: f32,
}

impl Default for Hold {
    fn default() -> Self {
        Self {
            rpm: 3000.0,
            throttle: 0.5,
        }
    }
}

/// A run on the rollers, step by step.
pub struct Rollers {
    pub stage: Stage,
    /// Time in the stage (s).
    time: f32,
    /// The rollers' speed as the wheels' (rad/s), and how far they've turned (rad).
    pub omega: f32,
    pub turned: f32,
    /// The gear pulled in, and the engine speeds the pull starts and ends at, and where
    /// the warm-up shifts at the latest (rpm).
    pub gear: i8,
    start: f32,
    end: f32,
    shift: f32,
    /// The rollers' inertia (the car's mass at the tires), the wheels', the tires'
    /// rolling resistance (N·m) and the share of torque the gears lose.
    inertia: f32,
    wheels: f32,
    rolling: f32,
    gear_loss: f32,
    radius: f32,
    /// The power reading (W): what spins up the rollers, smoothed.
    pub power: f32,
    /// The rollers' speed and the time at the last sample, which the next one measures
    /// the acceleration from.
    last: (f32, f32),
    clock: f32,
    pub samples: Vec<Sample>,
    /// What broke during the run.
    pub broke: Vec<String>,
    /// Hold at this instead of pulling (for tuning).
    pub hold: Option<Hold>,
}

impl Rollers {
    /// Straps `car` onto the rollers: in neutral, idling, with the gears worked by the
    /// dyno (not the automatic).
    pub fn new(car: &mut Car) -> Self {
        let setup = &car.setup;
        let engine = &setup.engine;
        let shares = engine.drive.shares();
        let radius_of = |i: usize| setup.axle(i < 2).tire.radius;
        let radius: f32 = (0..4).map(|i| shares[i] * radius_of(i)).sum();
        let mass = car.body.mass;
        let wheels: f32 = [true, false]
            .map(|front| 2.0 * setup.axle(front).wheel_inertia)
            .iter()
            .sum();
        // The tires' rolling resistance on the rollers, as a share of the load.
        let rolling = (setup.front.tire.rolling + setup.rear.tire.rolling) / 2.0;
        let gear = PULL_GEAR.min(engine.top_gear());
        let limit = engine.build.ecu.rev_limit;
        let start = START_RPM.max(engine.idle_rpm() + START_ABOVE_IDLE);
        let gear_loss = if matches!(engine.drive, Drive::All { .. }) {
            AWD_GEAR_LOSS
        } else {
            GEAR_LOSS
        };
        car.drivetrain.automatic = false;
        car.drivetrain.shift_to(engine, 0);
        car.controls = Default::default();
        Self {
            stage: Stage::Idle,
            time: 0.0,
            omega: 0.0,
            turned: 0.0,
            gear,
            start,
            end: limit - END_MARGIN,
            shift: limit - SHIFT_MARGIN,
            inertia: mass * radius * radius,
            wheels,
            rolling: rolling * mass * GRAVITY * radius,
            gear_loss,
            radius,
            power: 0.0,
            last: (0.0, 0.0),
            clock: 0.0,
            samples: Vec::new(),
            broke: Vec::new(),
            hold: None,
        }
    }

    /// The speed the rollers show (m/s).
    pub fn speed(&self) -> f32 {
        self.omega * self.radius
    }

    /// Engine rpm in the pull gear at the rollers' speed.
    fn rpm_in_gear(&self, car: &Car) -> f32 {
        car.setup.engine.rpm_in(self.gear, self.omega)
    }

    fn next(&mut self, stage: Stage) {
        self.stage = stage;
        self.time = 0.0;
    }

    /// Advances the run by `dt`.
    pub fn step(&mut self, car: &mut Car, dt: f32) {
        self.time += dt;
        self.clock += dt;
        let rpm = self.rpm_in_gear(car);
        let engine = &car.setup.engine;
        let broken = car.drivetrain.engine.blown(&engine.build.hardware);
        let shifting = car.drivetrain.selected_gear() != car.drivetrain.gear;
        match self.stage {
            Stage::Idle if self.time > IDLE => {
                car.drivetrain.shift_to(engine, 1);
                self.next(Stage::Gears);
            }
            Stage::Gears if broken => self.next(Stage::Coast),
            Stage::Gears if car.drivetrain.selected_gear() == self.gear => self.next(Stage::Settle),
            Stage::Gears if !shifting && car.drivetrain.gear > 0 => {
                // Up as soon as the next gear lands at the pull's run-in (the last shift
                // goes straight into the pull).
                let gear = car.drivetrain.gear + 1;
                let next = engine.rpm_in(gear, self.omega);
                if next >= self.start - RUN_IN || car.drivetrain.rpm > self.shift {
                    car.drivetrain.shift_to(engine, gear);
                }
            }
            Stage::Settle if car.drivetrain.gear == self.gear && self.hold.is_some() => {
                self.next(Stage::Hold)
            }
            Stage::Settle if car.drivetrain.gear == self.gear => self.next(Stage::Pull),
            Stage::Hold if broken => self.stop(car),
            Stage::Pull if rpm < self.start => self.last = (self.omega, self.clock),
            Stage::Pull => {
                let next = self.start + SAMPLE_RPM * self.samples.len() as f32;
                if rpm >= next {
                    self.sample(car, rpm);
                }
                if rpm >= self.end || broken || self.time > LONGEST_PULL {
                    car.drivetrain.shift_to(&car.setup.engine, 0);
                    self.next(Stage::Coast);
                }
            }
            Stage::Coast if self.omega <= 0.0 => self.next(Stage::Done),
            _ => {}
        }
        if self.stage == Stage::Coast || self.stage == Stage::Done {
            let cylinders = car.setup.engine.build.hardware.cylinders as usize;
            self.broke = car.drivetrain.engine.damage.failures(cylinders);
        }

        // The car on the rollers: part throttle up through the gears and full throttle
        // pulling, the brake holding the rollers back after the pull.
        let throttle = match self.stage {
            Stage::Gears => WARM_UP_THROTTLE,
            Stage::Pull => 1.0,
            Stage::Hold => self.hold.map_or(0.0, |hold| hold.throttle),
            _ => 0.0,
        };
        let engine = &car.setup.engine;
        // Held: the rollers' speed for the rpm in the pull gear.
        let held = (self.stage == Stage::Hold)
            .then_some(self.hold)
            .flatten()
            .map(|hold| hold.rpm / engine.rpm_in(self.gear, 1.0));
        let drivetrain = &mut car.drivetrain;
        drivetrain.engine.airspeed = FAN_AIRSPEED;
        drivetrain.engine.fuel_supply = car.fuel.supply;
        let drive = drivetrain.step(engine, throttle, None, self.omega, true, dt);
        let at_wheels = drive.torque - self.gear_loss * drive.torque.abs();
        let rolling = if self.omega > 0.0 { self.rolling } else { 0.0 };
        let inertia = self.inertia + self.wheels + drive.inertia;
        let brake = match self.stage {
            Stage::Coast | Stage::Done => self.inertia * ROLLER_BRAKE / self.radius,
            _ => 0.0,
        };
        let before = self.omega;
        if let Some(target) = held {
            // Held: the dyno's motor and brake bring the rollers to the held speed (as
            // quickly as `HOLD_RATE` at the tires) and keep them there, whatever the
            // wheels push with.
            let most = HOLD_RATE / self.radius * dt;
            self.omega += (target - self.omega).clamp(-most, most);
        } else {
            let spun = self.omega + (at_wheels - rolling) / inertia * dt;
            // The brake (and rolling resistance) only slow the rollers down to a stop.
            self.omega = (spun - brake / inertia * dt).max(0.0);
            if before == 0.0 && at_wheels <= rolling {
                self.omega = 0.0;
            }
        }
        self.turned = (self.turned + self.omega * dt) % TAU;
        // The power at the wheels: what they push the held rollers with, else what spins
        // up the rollers.
        let power = match (self.stage, held) {
            (Stage::Coast | Stage::Done, _) => 0.0,
            (_, Some(_)) => (at_wheels - rolling) * self.omega,
            _ => self.inertia * (self.omega - before) / dt * self.omega,
        };
        self.power += (power - self.power) * (dt / READING_LAG).min(1.0);
        let still = bevy::math::Vec3::new(0.0, GRAVITY, 0.0);
        let flow = car.drivetrain.engine.fuel_flow;
        car.fuel.step(&car.setup.tank, flow, still, dt);
    }

    /// A sample at engine speed `rpm`: the crank's torque now, and the power that spun up
    /// the rollers since the last sample.
    fn sample(&mut self, car: &Car, rpm: f32) {
        let (omega, time) = self.last;
        let elapsed = (self.clock - time).max(STEP);
        let accelerating = (self.omega - omega) / elapsed;
        let wheel = self.inertia * accelerating * (self.omega + omega) / 2.0;
        let drivetrain = &car.drivetrain;
        let crank = drivetrain
            .engine
            .torque(&car.setup.engine.build, drivetrain.rpm);
        self.samples.push(Sample { rpm, crank, wheel });
        self.last = (self.omega, self.clock);
    }

    /// Ends a hold (or a run on its way to one): off the throttle in neutral, coasting
    /// down.
    pub fn stop(&mut self, car: &mut Car) {
        if self.stage != Stage::Coast && self.stage != Stage::Done {
            car.drivetrain.shift_to(&car.setup.engine, 0);
            self.next(Stage::Coast);
        }
    }

    /// Runs to the end (a hold only ends when stopped).
    pub fn finish(&mut self, car: &mut Car) {
        if self.hold.is_some() && self.stage != Stage::Coast {
            return;
        }
        // An hour of dyno time is more than any run takes.
        for _ in 0..(3600.0 / STEP) as usize {
            if self.stage == Stage::Done {
                return;
            }
            self.step(car, STEP);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::CarSetup;
    use crate::terrain::Terrain;

    fn on_rollers(chassis: usize) -> (Car, Rollers) {
        let terrain = Terrain::flat();
        let mut car = Car::spawn_at(0, CarSetup::factory(chassis, 0), &terrain, 0.0, 0.0, 0.0);
        let rollers = Rollers::new(&mut car);
        (car, rollers)
    }

    #[test]
    fn a_run_goes_through_the_gears_and_pulls_in_fourth() {
        let (mut car, mut rollers) = on_rollers(3);
        let fuel = car.fuel.litres();
        let mut gears = Vec::new();
        let (mut time, mut top_warm_up, mut shifting, mut pull_from) = (0.0, 0.0, 0.0, None);
        while rollers.stage != Stage::Done && time < 300.0 {
            rollers.step(&mut car, STEP);
            time += STEP;
            let gear = car.drivetrain.gear;
            match rollers.stage {
                Stage::Gears => {
                    if gear > 0 && gears.last() != Some(&gear) {
                        gears.push(gear);
                    }
                    top_warm_up = f32::max(top_warm_up, car.drivetrain.rpm);
                }
                Stage::Settle => shifting += STEP,
                Stage::Pull if pull_from.is_none() => pull_from = Some(car.drivetrain.rpm),
                _ => {}
            }
        }
        assert_eq!(rollers.stage, Stage::Done);
        assert!(time < 45.0, "took {time} s");
        assert_eq!(gears, [1, 2, 3]);
        // Each gear only revved as far as the next one needs, and the pull starts as soon
        // as fourth is in, just below where it samples.
        assert!(top_warm_up < 3500.0, "{top_warm_up} rpm warming up");
        assert!(shifting < 0.5, "{shifting} s shifting into fourth");
        let pull_from = pull_from.unwrap();
        assert!(
            (1500.0..2000.0).contains(&pull_from),
            "pulls from {pull_from} rpm"
        );
        assert!(car.fuel.litres() < fuel);
        assert!(rollers.broke.is_empty());
        let samples = &rollers.samples;
        let limit = car.setup.engine.build.ecu.rev_limit;
        assert!((samples[0].rpm - 2000.0).abs() < 10.0, "{}", samples[0].rpm);
        assert!(samples.last().unwrap().rpm > limit - 250.0);
        // The wheels get less than the crank makes, but most of it.
        let peak = |f: fn(&Sample) -> f32| samples.iter().map(f).fold(0.0, f32::max);
        let (crank, wheel) = (peak(Sample::crank_hp), peak(Sample::wheel_hp));
        assert!(
            wheel < 0.9 * crank && wheel > 0.75 * crank,
            "{wheel} whp of {crank} hp"
        );
        assert!((130.0..200.0).contains(&crank), "{crank} hp");
        // The wheels' torque follows the crank's.
        for sample in samples.iter().skip(2) {
            let share = sample.wheel_torque() / sample.crank;
            assert!((0.7..0.9).contains(&share), "{share} at {} rpm", sample.rpm);
        }
    }
    #[test]
    fn a_hold_holds_the_rpm_at_the_throttle_set() {
        let (mut car, mut rollers) = on_rollers(3);
        rollers.hold = Some(Hold {
            rpm: 4000.0,
            throttle: 0.3,
        });
        let mut time = 0.0;
        while time < 30.0 {
            rollers.step(&mut car, STEP);
            time += STEP;
        }
        assert_eq!(rollers.stage, Stage::Hold);
        assert_eq!(car.drivetrain.gear, rollers.gear);
        let rpm = car.drivetrain.rpm;
        assert!((rpm - 4000.0).abs() < 50.0, "{rpm} rpm");
        let part = car.drivetrain.engine.intake_pressure;
        let power = rollers.power;
        assert!(power > 5e3, "{power} W at the wheels");
        // More throttle: more manifold pressure and power at the same rpm.
        rollers.hold = Some(Hold {
            rpm: 4000.0,
            throttle: 1.0,
        });
        for _ in 0..(5.0 / STEP) as usize {
            rollers.step(&mut car, STEP);
        }
        let rpm = car.drivetrain.rpm;
        assert!((rpm - 4000.0).abs() < 50.0, "{rpm} rpm floored");
        assert!(car.drivetrain.engine.intake_pressure > part + 20e3);
        assert!(rollers.power > 2.0 * power, "{} W floored", rollers.power);
        // Stopping coasts down to the end.
        rollers.stop(&mut car);
        rollers.finish(&mut car);
        assert_eq!(rollers.stage, Stage::Done);
    }

    #[test]
    fn all_wheel_drive_loses_more_and_skipping_ends_the_run() {
        // The rally car drives all four wheels.
        let (mut car, mut rollers) = on_rollers(5);
        rollers.finish(&mut car);
        assert_eq!(rollers.stage, Stage::Done);
        let crank = rollers
            .samples
            .iter()
            .map(Sample::crank_hp)
            .fold(0.0, f32::max);
        let wheel = rollers
            .samples
            .iter()
            .map(Sample::wheel_hp)
            .fold(0.0, f32::max);
        assert!(wheel < 0.88 * crank, "{wheel} whp of {crank} hp");
    }

    #[test]
    fn every_car_gets_through_a_run() {
        for chassis in 0..crate::chassis::CHASSIS.len() {
            let (mut car, mut rollers) = on_rollers(chassis);
            let name = car.setup.name;
            let mut time = 0.0;
            let mut warm_up = 0.0;
            while rollers.stage != Stage::Done && time < 120.0 {
                rollers.step(&mut car, STEP);
                time += STEP;
                if rollers.stage == Stage::Gears {
                    warm_up += STEP;
                }
            }
            assert_eq!(rollers.stage, Stage::Done, "{name}");
            assert!(warm_up < 15.0, "{name}: {warm_up} s through the gears");
            assert!(time < 60.0, "{name}: {time} s");
            let limit = car.setup.engine.build.ecu.rev_limit;
            let last = rollers.samples.last().expect(name).rpm;
            assert!(last > limit - 250.0, "{name}: up to {last} rpm");
            assert!(rollers.broke.is_empty(), "{name}: {:?}", rollers.broke);
        }
    }
}
