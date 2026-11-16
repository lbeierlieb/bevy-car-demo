//! Engine, clutch and gearbox driving the rear wheels, the front wheels or all four.
//!
//! While the clutch is locked, the engine turns with the driven wheels (through the gear
//! and final drive ratios): its torque goes to the wheels and its inertia is added to
//! theirs, so the wheel-spin update in the corners stays implicit and stable. The clutch
//! slips when pulling away, while shifting, and below idle speed. Then the engine spins
//! freely and the clutch passes a torque that depends only on engine speed (like a
//! centrifugal clutch), so it never couples the engine stiffly to the wheels at the 120 Hz
//! chassis rate.
//!
//! With a clutch pedal, the driver works the clutch instead: it passes up to its capacity
//! (less the further the pedal is pressed) in the direction it slips, and locks once the
//! engine and wheels meet. Then the engine can be dragged below idle and stall.

use std::f32::consts::PI;

use super::engine::parts::Parts;
use super::engine::{Engine, EngineBuild};

/// Engine speed to rpm.
const RPM: f32 = 60.0 / (2.0 * PI);
/// Below this, a running engine stalls (only with a clutch pedal).
const STALL_RPM: f32 = 400.0;
/// Clutch pedal travel beyond which a gear change needs no help from the automatic clutch.
const CLUTCH_IN: f32 = 0.6;
/// Time the automatic stays in a gear before it shifts again (s): just after a shift, the
/// clutch closing can spin the wheels up for a moment, and that isn't speed to shift on.
const AUTO_DWELL: f32 = 1.0;
/// The throttle linkage, pedal travel to plate opening (linear in between): a progressive
/// cam. A butterfly passes most of the air the engine can take long before it's fully
/// open, so a straight linkage would put nearly all the torque in the first third of the
/// pedal. This one spreads it over the travel, about in proportion around 3500 rpm.
const LINKAGE: [(f32, f32); 10] = [
    (0.0, 0.0),
    (0.1, 0.1),
    (0.2, 0.14),
    (0.3, 0.17),
    (0.4, 0.2),
    (0.5, 0.235),
    (0.6, 0.27),
    (0.7, 0.32),
    (0.8, 0.38),
    (0.9, 0.5),
];

/// Throttle plate opening (0..1) for the pedal pressed to `pedal` (0..1).
pub fn throttle_plate(pedal: f32) -> f32 {
    let pedal = pedal.clamp(0.0, 1.0);
    let next = LINKAGE.iter().position(|&(p, _)| p > pedal);
    let ((p0, t0), (p1, t1)) = match next {
        Some(i) => (LINKAGE[i - 1], LINKAGE[i]),
        None => (LINKAGE[LINKAGE.len() - 1], (1.0, 1.0)),
    };
    t0 + (t1 - t0) * (pedal - p0) / (p1 - p0)
}

/// The wheels the engine drives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drive {
    Rear,
    Front,
    /// All four, through a centre differential that sends this share of the torque to the
    /// front axle.
    All {
        front: f32,
    },
}

impl Drive {
    /// Each wheel's share of the torque (front left and right, rear left and right). The
    /// differentials turn at the wheels' speeds weighted the same way.
    pub fn shares(self) -> [f32; 4] {
        let front = match self {
            Drive::Rear => 0.0,
            Drive::Front => 1.0,
            Drive::All { front } => front,
        };
        [
            front / 2.0,
            front / 2.0,
            (1.0 - front) / 2.0,
            (1.0 - front) / 2.0,
        ]
    }

    pub fn name(self) -> &'static str {
        match self {
            Drive::Rear => "rear-wheel drive",
            Drive::Front => "front-wheel drive",
            Drive::All { .. } => "all-wheel drive",
        }
    }
}

/// A differential's resistance to its two sides turning at different speeds: none for an
/// open one, which sends the same torque to both sides however fast either spins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Differential {
    /// Torque moved from the faster side to the slower one per rad/s between them
    /// (N·m·s), and the most it moves (N·m).
    pub coupling: f32,
    pub limit: f32,
}

impl Differential {
    pub const OPEN: Self = Self {
        coupling: 0.0,
        limit: 0.0,
    };
    /// A limited-slip differential for the road.
    pub const LIMITED_SLIP: Self = Self {
        coupling: 40.0,
        limit: 400.0,
    };

    /// Torque it moves from a side turning at `a` to the one turning at `b` (rad/s).
    pub fn transfer(&self, a: f32, b: f32) -> f32 {
        (self.coupling * (a - b)).clamp(-self.limit, self.limit)
    }
}

#[derive(Clone, Debug)]
pub struct EngineSetup {
    /// The parts the engine is built from, and the engine: its hardware, maps and ECU.
    pub parts: Parts,
    pub build: EngineBuild,
    /// Most torque the clutch can pass while slipping (N·m).
    pub clutch_torque: f32,
    /// Forward gear ratios from first, the reverse ratio, and the final drive.
    pub gears: &'static [f32],
    pub reverse: f32,
    pub final_drive: f32,
    /// Time the clutch stays open for a gear change, and then takes to close again (s).
    pub shift_time: f32,
    pub engage_time: f32,
    pub drive: Drive,
    /// The differentials in the driven axles, and between them with all-wheel drive.
    pub differential: Differential,
    pub centre: Differential,
}

impl Default for EngineSetup {
    /// A 2-litre petrol four with a six-speed gearbox.
    fn default() -> Self {
        Self {
            parts: Parts::stock(),
            build: EngineBuild::default(),
            clutch_torque: 350.0,
            gears: &[3.3, 2.1, 1.5, 1.17, 0.95, 0.8],
            reverse: 3.3,
            final_drive: 3.7,
            shift_time: 0.25,
            engage_time: 0.15,
            drive: Drive::Rear,
            differential: Differential::OPEN,
            centre: Differential::OPEN,
        }
    }
}

impl EngineSetup {
    /// Overall ratio from engine to wheels for a gear (-1 reverse, 0 neutral, 1 and up).
    pub fn ratio(&self, gear: i8) -> f32 {
        match gear {
            -1 => -self.reverse * self.final_drive,
            0 => 0.0,
            g => self.gears[g as usize - 1] * self.final_drive,
        }
    }

    /// Engine rpm in `gear` with the driven wheels at `wheel_omega` and the clutch locked.
    pub fn rpm_in(&self, gear: i8, wheel_omega: f32) -> f32 {
        wheel_omega * self.ratio(gear) * RPM
    }

    pub fn top_gear(&self) -> i8 {
        self.gears.len() as i8
    }

    /// The idle speed the ECU holds, above which the automatic clutch starts to bite.
    pub fn idle_rpm(&self) -> f32 {
        self.build.ecu.idle_rpm
    }

    /// The engine's redline, for the rev counter and the gearbox.
    pub fn redline_rpm(&self) -> f32 {
        self.build.hardware.redline()
    }

    /// End of a rev counter's scale: a whole thousand above the redline.
    pub fn rev_scale(&self) -> f32 {
        (self.redline_rpm() / 1000.0).ceil() * 1000.0 + 1000.0
    }
}

#[derive(Clone, Debug)]
pub struct Drivetrain {
    pub rpm: f32,
    /// False once stalled, until started again.
    pub running: bool,
    /// -1 reverse, 0 neutral, 1 and up forward.
    pub gear: i8,
    /// Shifts by itself, based on rpm and throttle.
    pub automatic: bool,
    /// Gear being shifted into, and the time left until the clutch closes again.
    shift: Option<(i8, f32)>,
    /// Time since the last gear change went in.
    in_gear: f32,
    /// How far the clutch has closed again after a shift (0..1).
    engaged: f32,
    locked: bool,
    pub engine: Engine,
    /// How hard the engine fired in the last step (0 at idle or cut, 1 at full load).
    pub load: f32,
    /// Clutch pedal travel (0 released, 1 pressed) in the last step, if there's a pedal.
    clutch_pedal: Option<f32>,
    /// Engine minus wheel rpm through the pedal clutch in the last step, while slipping.
    slip: f32,
}

/// What the drivetrain does to the driven wheels this step (shared by both).
pub struct WheelDrive {
    pub torque: f32,
    /// Engine inertia as felt at the wheels while the clutch is locked.
    pub inertia: f32,
}

impl Drivetrain {
    pub fn new(engine: &EngineSetup) -> Self {
        Self {
            rpm: engine.idle_rpm(),
            running: true,
            gear: 1,
            automatic: false,
            shift: None,
            in_gear: AUTO_DWELL,
            engaged: 1.0,
            locked: false,
            engine: Engine::default(),
            load: 0.0,
            clutch_pedal: None,
            slip: 0.0,
        }
    }

    /// Starts a stalled engine (the starter spins it up to idle at once), unless it's blown.
    pub fn start(&mut self, engine: &EngineSetup) {
        if !self.running && !self.engine.blown(&engine.build.hardware) {
            self.running = true;
            self.rpm = self.rpm.max(engine.idle_rpm());
            self.engine.start();
        }
    }

    /// The gear shown to the driver: the one being shifted into, if shifting.
    pub fn selected_gear(&self) -> i8 {
        self.shift.map_or(self.gear, |(gear, _)| gear)
    }

    /// A manual shift `steps` gears up (or down if negative), within R..=top gear.
    /// Reverse only engages when the car is (nearly) stopped (`speed` in m/s), and
    /// downshifts that would over-rev the engine are refused.
    pub fn shift_by(&mut self, engine: &EngineSetup, steps: i8, wheel_omega: f32, speed: f32) {
        let gear = self.selected_gear();
        let target = (gear + steps).clamp(-1, engine.top_gear());
        let over_rev = target > 0 && engine.rpm_in(target, wheel_omega) > engine.redline_rpm();
        if target != gear && !over_rev && (target >= 0 || speed.abs() < 2.0) {
            self.shift_to(engine, target);
        }
    }

    /// An H-shifter's lever in `gate` (0 between gates): the gear follows the lever. With a
    /// clutch pedal, a gear only goes in while the pedal is pressed; otherwise the box stays
    /// in neutral until it is. Reverse only goes in when (nearly) stopped (`speed` in m/s).
    pub fn select_gate(&mut self, engine: &EngineSetup, gate: i8, speed: f32) {
        let gate = if gate < 0 && speed.abs() > 2.0 {
            0
        } else {
            gate
        };
        if gate == self.selected_gear() {
            return;
        }
        let clutch_in = self.clutch_pedal.is_some_and(|travel| travel > CLUTCH_IN);
        if gate == 0 || self.clutch_pedal.is_none() || clutch_in {
            self.shift_to(engine, gate);
        } else {
            self.shift_to(engine, 0);
        }
    }

    /// Starts a gear change; the clutch opens for `shift_time`. With the clutch pedal
    /// pressed, the gear goes in at once.
    pub fn shift_to(&mut self, engine: &EngineSetup, gear: i8) {
        let gear = gear.clamp(-1, engine.top_gear());
        if gear == self.selected_gear() {
            return;
        }
        if self.clutch_pedal.is_some_and(|travel| travel > CLUTCH_IN) {
            self.gear = gear;
            self.shift = None;
            self.in_gear = 0.0;
            self.engaged = 1.0;
        } else {
            self.shift = Some((gear, engine.shift_time));
            self.engaged = 0.0;
        }
        self.locked = false;
    }

    /// Advances by `dt` with the driven wheels turning at `wheel_omega` (rad/s, averaged)
    /// and returns the torque for them. `clutch` is the pedal's travel, if there's a pedal;
    /// the automatic gearbox works the clutch itself and ignores the pedal.
    /// The automatic only shifts while the wheels are `grounded`: in the air they spin
    /// freely and would make it hunt between gears.
    pub fn step(
        &mut self,
        engine: &EngineSetup,
        throttle: f32,
        clutch: Option<f32>,
        wheel_omega: f32,
        grounded: bool,
        dt: f32,
    ) -> WheelDrive {
        let clutch = clutch.filter(|_| !self.automatic);
        self.clutch_pedal = clutch;
        // Without a pedal, the automatic clutch never lets the engine stall: it starts
        // again by itself, as long as it gets fuel. A blown engine stays off.
        if clutch.is_none() && self.engine.fuel_supply > 0.5 {
            self.start(engine);
        }
        if self.engine.blown(&engine.build.hardware) {
            self.running = false;
        }
        self.in_gear += dt;
        if self.automatic && grounded {
            self.auto_shift(engine, throttle, wheel_omega);
        }
        if let Some((gear, left)) = &mut self.shift {
            *left -= dt;
            if *left <= 0.0 {
                self.gear = *gear;
                self.shift = None;
                self.in_gear = 0.0;
            }
        }
        // While shifting, the clutch is open and the throttle and ignition are cut (like a
        // sequential gearbox), so the engine doesn't flare up and slam the wheels.
        let (ratio, throttle) = if self.shift.is_some() {
            (0.0, 0.0)
        } else {
            self.engaged = (self.engaged + dt / engine.engage_time).min(1.0);
            (engine.ratio(self.gear), throttle)
        };
        let build = &engine.build;
        let firing = self.running && self.shift.is_none();
        self.engine
            .step(build, throttle_plate(throttle), self.rpm, firing, dt);
        let cycle = self.engine.cycle(build, self.rpm);
        self.load = if firing && cycle.indicated > 0.0 {
            ((cycle.load - 0.2) / 0.7).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let wheel_rpm = wheel_omega * ratio * RPM;
        let state = self.engine;
        let engine_torque = move |rpm: f32| state.torque(build, rpm);
        if let Some(travel) = clutch {
            let capacity = engine.clutch_torque * (1.0 - travel) * self.engaged;
            return self.step_pedal(engine, engine_torque, capacity, ratio, wheel_rpm, dt);
        }

        // Lock once the wheels have caught up with the engine; slip again below idle, or
        // when the engine makes more torque than the clutch can pass.
        let holds = engine_torque(wheel_rpm).abs() <= engine.clutch_torque;
        if ratio == 0.0 || wheel_rpm < engine.idle_rpm() * 0.9 || !holds {
            self.locked = false;
        } else if wheel_rpm >= self.rpm - 50.0 {
            self.locked = true;
        }

        if self.locked {
            self.rpm = wheel_rpm;
            WheelDrive {
                torque: engine_torque(self.rpm) * ratio,
                inertia: engine.build.hardware.inertia * ratio * ratio,
            }
        } else {
            // Slipping clutch: it bites from 700 rpm above idle and passes its full torque
            // 1500 rpm higher, so the engine pulls away at about 2500 rpm on full throttle.
            // While shifting or in neutral it's open, and after a shift it closes gradually.
            let bite = ((self.rpm - engine.idle_rpm() - 700.0) / 1500.0).clamp(0.0, 1.0);
            let clutch = if ratio == 0.0 {
                0.0
            } else {
                engine.clutch_torque * bite * bite * (3.0 - 2.0 * bite) * self.engaged
            };
            let net = engine_torque(self.rpm) - clutch;
            self.rpm = (self.rpm + net / engine.build.hardware.inertia * dt * RPM).max(0.0);
            WheelDrive {
                torque: clutch * ratio,
                inertia: 0.0,
            }
        }
    }

    /// The driver's clutch: it holds while it can pass the engine's torque, and slips with
    /// its full `capacity` otherwise, towards the slower side. Locking when the two sides
    /// meet keeps the stiff coupling implicit, as without a pedal.
    fn step_pedal(
        &mut self,
        engine: &EngineSetup,
        engine_torque: impl Fn(f32) -> f32,
        capacity: f32,
        ratio: f32,
        wheel_rpm: f32,
        dt: f32,
    ) -> WheelDrive {
        if ratio == 0.0 || engine_torque(self.rpm).abs() > capacity {
            self.locked = false;
        }
        let drive = if self.locked {
            self.slip = 0.0;
            self.rpm = wheel_rpm;
            WheelDrive {
                torque: engine_torque(self.rpm) * ratio,
                inertia: engine.build.hardware.inertia * ratio * ratio,
            }
        } else {
            let slip = self.rpm - wheel_rpm;
            // The wheels can overshoot the engine within a step, too.
            let crossed = slip * self.slip < 0.0;
            self.slip = slip;
            let clutch = if ratio == 0.0 {
                0.0
            } else {
                capacity * slip.signum()
            };
            let net = engine_torque(self.rpm) - clutch;
            self.rpm = (self.rpm + net / engine.build.hardware.inertia * dt * RPM).max(0.0);
            // Caught up (or overshot) within the step: lock, if the clutch can hold.
            if ratio != 0.0 && (crossed || (self.rpm - wheel_rpm) * slip <= 0.0) {
                self.rpm = wheel_rpm.max(0.0);
                self.locked = engine_torque(self.rpm).abs() <= capacity;
            }
            WheelDrive {
                torque: clutch * ratio,
                inertia: 0.0,
            }
        };
        if self.running && self.rpm < STALL_RPM {
            self.running = false;
        }
        drive
    }

    /// Up when the engine passes a throttle-dependent rpm, down when it drops low.
    fn auto_shift(&mut self, engine: &EngineSetup, throttle: f32, wheel_omega: f32) {
        if self.shift.is_some() || self.gear < 1 || self.in_gear < AUTO_DWELL {
            return;
        }
        let rpm_in = |gear: i8| engine.rpm_in(gear, wheel_omega);
        let up = 3000.0 + (engine.redline_rpm() - 300.0 - 3000.0) * throttle;
        let down = 1300.0 + 2200.0 * throttle;
        let rpm = rpm_in(self.gear);
        if rpm > up && self.gear < engine.top_gear() {
            self.shift_to(engine, self.gear + 1);
        } else if rpm < down && self.gear > 1 && rpm_in(self.gear - 1) < up {
            self.shift_to(engine, self.gear - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linkage_opens_the_plate_progressively() {
        assert_eq!(throttle_plate(0.0), 0.0);
        assert_eq!(throttle_plate(1.0), 1.0);
        assert_eq!(throttle_plate(2.0), 1.0);
        let mut last = 0.0;
        for i in 1..=100 {
            let plate = throttle_plate(i as f32 / 100.0);
            assert!(plate > last);
            last = plate;
        }
        // Half the pedal is well under half the plate.
        assert!(throttle_plate(0.5) < 0.3);
    }

    /// A car-like load on the wheels: the car's mass as inertia at the tire radius.
    fn drive(
        drivetrain: &mut Drivetrain,
        engine: &EngineSetup,
        throttle: f32,
        seconds: f32,
    ) -> f32 {
        let (radius, mass, dt) = (0.33, 1260.0, 1.0 / 120.0);
        let mut omega = 0.0;
        for _ in 0..(seconds / dt) as usize {
            let out = drivetrain.step(engine, throttle, None, omega, true, dt);
            omega += out.torque / (mass * radius * radius + out.inertia) * dt;
        }
        omega * radius
    }

    #[test]
    fn idles_and_pulls_away() {
        let engine = EngineSetup::default();
        let mut drivetrain = Drivetrain::new(&engine);
        // In gear without throttle the clutch stays open and the engine idles.
        assert!(drive(&mut drivetrain, &engine, 0.0, 2.0) < 0.01);
        assert!(
            (drivetrain.rpm - engine.idle_rpm()).abs() < 100.0,
            "rpm {}",
            drivetrain.rpm
        );
        // Full throttle: the clutch bites, then locks.
        let speed = drive(&mut drivetrain, &engine, 1.0, 3.0);
        assert!(speed > 8.0, "speed {speed}");
        assert!(drivetrain.locked);
    }

    #[test]
    fn a_weak_clutch_slips_under_a_strong_engine() {
        // A clutch that passes half of what the engine makes: the engine runs away from
        // the wheels, and the car gets only what the clutch passes.
        let strong = EngineSetup::default();
        let weak = EngineSetup {
            clutch_torque: 100.0,
            ..strong.clone()
        };
        let mut held = Drivetrain::new(&strong);
        let mut slipping = Drivetrain::new(&weak);
        let fast = drive(&mut held, &strong, 1.0, 4.0);
        let slow = drive(&mut slipping, &weak, 1.0, 4.0);
        assert!(held.locked && !slipping.locked);
        assert!(slow < 0.7 * fast, "{slow} against {fast} m/s");
        assert!(slipping.rpm > 5000.0, "{} rpm", slipping.rpm);
    }

    #[test]
    fn manual_shifts_stay_within_the_gears() {
        let engine = EngineSetup::default();
        let mut drivetrain = Drivetrain::new(&engine);
        // Up past top gear (at a speed where top gear is fine) stays in top gear.
        let omega = 80.0;
        for _ in 0..8 {
            drivetrain.shift_by(&engine, 1, omega, 26.0);
        }
        assert_eq!(drivetrain.selected_gear(), engine.top_gear());
        // Down into first at that speed would over-rev: refused at a gear that doesn't.
        for _ in 0..8 {
            drivetrain.shift_by(&engine, -1, omega, 26.0);
        }
        let gear = drivetrain.selected_gear();
        assert!(
            gear >= 2 && engine.rpm_in(gear, omega) <= engine.redline_rpm(),
            "gear {gear}"
        );
        // Stopped: down through first and neutral to reverse, and no further.
        for _ in 0..10 {
            drivetrain.shift_by(&engine, -1, 0.0, 0.0);
        }
        assert_eq!(drivetrain.selected_gear(), -1);
    }

    #[test]
    fn automatic_ignores_the_clutch_pedal() {
        let engine = EngineSetup::default();
        // Wherever the pedal is, the automatic pulls away and shifts up.
        for pedal in [0.0, 0.5, 1.0] {
            let mut drivetrain = Drivetrain::new(&engine);
            drivetrain.automatic = true;
            let speed = drive_with_clutch(&mut drivetrain, &engine, 1.0, |_| pedal, 8.0);
            assert!(drivetrain.running, "pedal {pedal}");
            assert!(
                speed > 20.0 && drivetrain.gear >= 2,
                "pedal {pedal}: {speed} m/s"
            );
        }
    }

    /// Like `drive`, with the clutch pedal at `clutch(t)` over the run.
    fn drive_with_clutch(
        drivetrain: &mut Drivetrain,
        engine: &EngineSetup,
        throttle: f32,
        clutch: impl Fn(f32) -> f32,
        seconds: f32,
    ) -> f32 {
        let (radius, mass, dt) = (0.33, 1260.0, 1.0 / 120.0);
        let mut omega = 0.0;
        for i in 0..(seconds / dt) as usize {
            let pedal = Some(clutch(i as f32 * dt));
            let out = drivetrain.step(engine, throttle, pedal, omega, true, dt);
            omega += out.torque / (mass * radius * radius + out.inertia) * dt;
        }
        omega * radius
    }

    #[test]
    fn dropping_the_clutch_stalls() {
        let engine = EngineSetup::default();
        let mut drivetrain = Drivetrain::new(&engine);
        // In first with the clutch in, the engine idles and the car stays put.
        assert!(drive_with_clutch(&mut drivetrain, &engine, 0.0, |_| 1.0, 1.0) < 0.01);
        assert!(drivetrain.running);
        // Clutch out at once without throttle: stalled, and barely moved.
        let speed = drive_with_clutch(&mut drivetrain, &engine, 0.0, |_| 0.0, 1.0);
        assert!(!drivetrain.running);
        assert!(speed < 1.0, "speed {speed}");
        // Stalled, it stays off (in gear, the engine only drags) until started with the
        // clutch in.
        drive_with_clutch(&mut drivetrain, &engine, 1.0, |_| 0.0, 1.0);
        assert!(!drivetrain.running && drivetrain.load == 0.0);
        drivetrain.start(&engine);
        drive_with_clutch(&mut drivetrain, &engine, 0.0, |_| 1.0, 1.0);
        assert!(drivetrain.running);
        assert!(
            (drivetrain.rpm - engine.idle_rpm()).abs() < 100.0,
            "rpm {}",
            drivetrain.rpm
        );
    }

    #[test]
    fn stalled_engine_drags_against_the_motion() {
        let engine = EngineSetup::default();
        let mut drivetrain = Drivetrain::new(&engine);
        drive_with_clutch(&mut drivetrain, &engine, 0.0, |_| 0.0, 1.0);
        assert!(!drivetrain.running);
        // Stalled in gear with the clutch out (and the throttle pressed, as when trying to
        // pull away): the car stays put rather than creeping backwards.
        let speed = drive_with_clutch(&mut drivetrain, &engine, 1.0, |_| 0.0, 5.0);
        assert!(speed.abs() < 0.05, "speed {speed}");
        // Rolling either way, the engine slows the wheels.
        for omega in [10.0, -10.0] {
            let out = drivetrain.step(&engine, 0.0, Some(0.0), omega, true, 1.0 / 120.0);
            assert!(out.torque * omega < 0.0, "torque {} at {omega}", out.torque);
        }
    }

    #[test]
    fn clutch_pedal_pulls_away() {
        let engine = EngineSetup::default();
        let mut drivetrain = Drivetrain::new(&engine);
        drive_with_clutch(&mut drivetrain, &engine, 0.0, |_| 1.0, 1.0);
        // Some throttle, letting the clutch out over a second and a half.
        let release = |t: f32| (1.0 - t / 1.5).max(0.0);
        let speed = drive_with_clutch(&mut drivetrain, &engine, 0.5, release, 4.0);
        assert!(drivetrain.running);
        assert!(drivetrain.locked);
        assert!(speed > 5.0, "speed {speed}");
        // Clutch in on the move: no more drive, and the gear changes at once.
        drivetrain.step(&engine, 0.0, Some(1.0), 20.0, true, 1.0 / 120.0);
        drivetrain.shift_to(&engine, 2);
        assert_eq!(drivetrain.gear, 2);
        let out = drivetrain.step(&engine, 1.0, Some(1.0), 20.0, true, 1.0 / 120.0);
        assert_eq!(out.torque, 0.0);
    }

    #[test]
    fn h_shifter_needs_the_clutch_pedal() {
        let engine = EngineSetup::default();
        let mut drivetrain = Drivetrain::new(&engine);
        let dt = 1.0 / 120.0;
        // Clutch pedal out, lever into third: the gear doesn't go in, the box is in neutral.
        drivetrain.step(&engine, 0.0, Some(0.0), 30.0, true, dt);
        drivetrain.select_gate(&engine, 3, 10.0);
        for _ in 0..60 {
            drivetrain.step(&engine, 0.0, Some(0.0), 30.0, true, dt);
        }
        assert_eq!(drivetrain.gear, 0);
        // Pressing the clutch with the lever still in third puts it in, at once.
        drivetrain.step(&engine, 0.0, Some(1.0), 30.0, true, dt);
        drivetrain.select_gate(&engine, 3, 10.0);
        assert_eq!(drivetrain.gear, 3);
        // Out of gear needs no clutch; reverse is refused while rolling.
        drivetrain.step(&engine, 0.0, Some(0.0), 30.0, true, dt);
        drivetrain.select_gate(&engine, 0, 10.0);
        drivetrain.step(&engine, 0.0, Some(1.0), 30.0, true, dt);
        drivetrain.select_gate(&engine, -1, 10.0);
        assert_eq!(drivetrain.selected_gear(), 0);
        // Without a pedal, the automatic clutch puts the gear in.
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.select_gate(&engine, 2, 10.0);
        for _ in 0..60 {
            drivetrain.step(&engine, 0.0, None, 30.0, true, dt);
        }
        assert_eq!(drivetrain.gear, 2);
    }

    #[test]
    fn rev_limiter_bounces() {
        let engine = EngineSetup::default();
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.shift_to(&engine, 0);
        // Free revving in neutral: settle, then watch the rpm and fuel for a second.
        for _ in 0..240 {
            drivetrain.step(&engine, 1.0, None, 0.0, true, 1.0 / 120.0);
        }
        let (mut low, mut high, mut cuts, mut was_fuelled) = (f32::MAX, 0.0f32, 0, true);
        for _ in 0..120 {
            drivetrain.step(&engine, 1.0, None, 0.0, true, 1.0 / 120.0);
            (low, high) = (low.min(drivetrain.rpm), high.max(drivetrain.rpm));
            let fuelled = drivetrain.load > 0.0;
            cuts += (was_fuelled && !fuelled) as u32;
            was_fuelled = fuelled;
        }
        // Roughly ten cuts a second (free revving overshoots, which can cut early), each
        // dropping the revs noticeably.
        assert!((6..=14).contains(&cuts), "{cuts} cuts");
        assert!(high - low > 150.0, "rpm {low}..{high}");
        assert!(high < engine.redline_rpm() + 300.0, "rpm {high}");
    }

    #[test]
    fn rev_limiter_holds_redline() {
        let engine = EngineSetup::default();
        let mut drivetrain = Drivetrain::new(&engine);
        drive(&mut drivetrain, &engine, 1.0, 20.0);
        assert!(
            drivetrain.rpm < engine.redline_rpm() + 300.0,
            "rpm {}",
            drivetrain.rpm
        );
        assert!(
            drivetrain.rpm > engine.redline_rpm() - 300.0,
            "rpm {}",
            drivetrain.rpm
        );
    }

    #[test]
    fn automatic_shifts_up_through_the_gears() {
        let engine = EngineSetup::default();
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.automatic = true;
        drive(&mut drivetrain, &engine, 1.0, 20.0);
        assert!(drivetrain.gear >= 4, "gear {}", drivetrain.gear);
        // Coasting down to a crawl, it comes back down to first.
        let mut omega: f32 = 30.0;
        for _ in 0..2000 {
            drivetrain.step(&engine, 0.0, None, omega, true, 1.0 / 120.0);
            omega = (omega - 0.1).max(1.0);
        }
        assert_eq!(drivetrain.gear, 1);
    }
}
