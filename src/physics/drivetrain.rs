//! Engine, clutch and gearbox driving the rear wheels.
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

/// Engine speed to rpm.
const RPM: f32 = 60.0 / (2.0 * PI);
/// Below this, a running engine stalls (only with a clutch pedal).
const STALL_RPM: f32 = 400.0;
/// Clutch pedal travel beyond which a gear change needs no help from the automatic clutch.
const CLUTCH_IN: f32 = 0.6;

#[derive(Clone, Debug)]
pub struct EngineSetup {
    pub idle_rpm: f32,
    pub redline_rpm: f32,
    /// Peak torque (N·m), shaped by `TORQUE_CURVE`.
    pub peak_torque: f32,
    /// Flywheel and crank inertia (kg·m²).
    pub inertia: f32,
    /// Friction torque at zero rpm, and its rise per rpm (engine braking).
    pub friction: f32,
    pub friction_per_rpm: f32,
    /// Most torque the clutch can pass while slipping (N·m).
    pub clutch_torque: f32,
    /// Forward gear ratios from first, the reverse ratio, and the final drive.
    pub gears: [f32; 6],
    pub reverse: f32,
    pub final_drive: f32,
    /// Time the clutch stays open for a gear change, and then takes to close again (s).
    pub shift_time: f32,
    pub engage_time: f32,
    /// Rev limiter: at the redline the fuel is cut for `limiter_cut`, then back on for at
    /// least `limiter_fuel` (s), so the engine audibly bounces off the limiter.
    pub limiter_cut: f32,
    pub limiter_fuel: f32,
}

impl Default for EngineSetup {
    /// A 2-litre petrol four: 230 N·m at 4200 rpm, about 130 kW at 6000 rpm.
    fn default() -> Self {
        Self {
            idle_rpm: 850.0,
            redline_rpm: 6800.0,
            peak_torque: 230.0,
            inertia: 0.18,
            friction: 12.0,
            friction_per_rpm: 0.005,
            clutch_torque: 350.0,
            gears: [3.3, 2.1, 1.5, 1.17, 0.95, 0.8],
            reverse: 3.3,
            final_drive: 3.7,
            shift_time: 0.25,
            engage_time: 0.15,
            limiter_cut: 0.08,
            limiter_fuel: 0.04,
        }
    }
}

/// Full-throttle torque as a fraction of the peak, by rpm (linear in between).
const TORQUE_CURVE: [(f32, f32); 5] = [
    (800.0, 0.55),
    (2000.0, 0.8),
    (4200.0, 1.0),
    (6000.0, 0.92),
    (6800.0, 0.8),
];

impl EngineSetup {
    /// Engine torque at a throttle (0..1) and rpm: the curve, minus friction. The rev
    /// limiter is in `Drivetrain`, as it needs state.
    pub fn torque(&self, throttle: f32, rpm: f32) -> f32 {
        let shape = TORQUE_CURVE
            .windows(2)
            .find(|w| rpm < w[1].0)
            .map_or(TORQUE_CURVE[4].1, |w| {
                let t = ((rpm - w[0].0) / (w[1].0 - w[0].0)).clamp(0.0, 1.0);
                w[0].1 + (w[1].1 - w[0].1) * t
            });
        // Below idle, an idle controller opens the throttle to keep the engine running.
        let idle = ((self.idle_rpm - rpm) / 300.0 + 0.08).clamp(0.0, 1.0);
        let throttle = throttle.max(idle);
        throttle * shape * self.peak_torque - self.friction - self.friction_per_rpm * rpm.max(0.0)
    }

    /// Overall ratio from engine to wheels for a gear (-1 reverse, 0 neutral, 1..=6).
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

    /// End of a rev counter's scale: a whole thousand above the redline.
    pub fn rev_scale(&self) -> f32 {
        (self.redline_rpm / 1000.0).ceil() * 1000.0 + 1000.0
    }
}

#[derive(Clone, Debug)]
pub struct Drivetrain {
    pub rpm: f32,
    /// False once stalled, until started again.
    pub running: bool,
    /// -1 reverse, 0 neutral, 1..=6 forward.
    pub gear: i8,
    /// Shifts by itself, based on rpm and throttle.
    pub automatic: bool,
    /// Gear being shifted into, and the time left until the clutch closes again.
    shift: Option<(i8, f32)>,
    /// How far the clutch has closed again after a shift (0..1).
    engaged: f32,
    locked: bool,
    /// Time left in the rev limiter's cycle: fuel cut, then fuel back on.
    limiter: f32,
    /// Throttle the engine actually got in the last step (0..1), after the rev limiter
    /// and the cut during shifts.
    pub fuel: f32,
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
            rpm: engine.idle_rpm,
            running: true,
            gear: 1,
            automatic: false,
            shift: None,
            engaged: 1.0,
            locked: false,
            limiter: 0.0,
            fuel: 0.0,
            clutch_pedal: None,
            slip: 0.0,
        }
    }

    /// Starts a stalled engine (the starter spins it up to idle at once).
    pub fn start(&mut self, engine: &EngineSetup) {
        if !self.running {
            self.running = true;
            self.rpm = self.rpm.max(engine.idle_rpm);
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
        let over_rev = target > 0 && engine.rpm_in(target, wheel_omega) > engine.redline_rpm;
        if target != gear && !over_rev && (target >= 0 || speed.abs() < 2.0) {
            self.shift_to(engine, target);
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
            self.engaged = 1.0;
        } else {
            self.shift = Some((gear, engine.shift_time));
            self.engaged = 0.0;
        }
        self.locked = false;
    }

    /// Advances by `dt` with the driven wheels turning at `wheel_omega` (rad/s, averaged)
    /// and returns the torque for them. `clutch` is the pedal's travel, if there's a pedal.
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
        self.clutch_pedal = clutch;
        // Without a pedal, the automatic clutch never lets the engine stall.
        if clutch.is_none() {
            self.running = true;
        }
        if self.automatic && grounded {
            self.auto_shift(engine, throttle, wheel_omega);
        }
        if let Some((gear, left)) = &mut self.shift {
            *left -= dt;
            if *left <= 0.0 {
                self.gear = *gear;
                self.shift = None;
            }
        }
        // While shifting, the clutch is open and the throttle is cut (like the ignition cut
        // of a sequential gearbox), so the engine doesn't flare up and slam the wheels.
        let (ratio, throttle) = if self.shift.is_some() {
            (0.0, 0.0)
        } else {
            self.engaged = (self.engaged + dt / engine.engage_time).min(1.0);
            (engine.ratio(self.gear), throttle)
        };
        // Rev limiter: a cut starts at the redline; after it the fuel comes back for a
        // moment even if the revs haven't dropped (in gear they barely do), unless the
        // engine is well over the redline.
        self.limiter = (self.limiter - dt).max(0.0);
        if self.limiter == 0.0 && self.rpm >= engine.redline_rpm {
            self.limiter = engine.limiter_cut + engine.limiter_fuel;
        }
        let cut = self.limiter > engine.limiter_fuel || self.rpm > engine.redline_rpm + 200.0;
        let throttle = if cut { 0.0 } else { throttle };
        self.fuel = if self.running { throttle } else { 0.0 };
        let wheel_rpm = wheel_omega * ratio * RPM;
        // A stalled engine only drags.
        let running = self.running;
        let engine_torque = move |rpm: f32| {
            if running {
                engine.torque(throttle, rpm)
            } else {
                -engine.friction - engine.friction_per_rpm * rpm.max(0.0)
            }
        };
        if let Some(travel) = clutch {
            let capacity = engine.clutch_torque * (1.0 - travel) * self.engaged;
            return self.step_pedal(engine, engine_torque, capacity, ratio, wheel_rpm, dt);
        }

        // Lock once the wheels have caught up with the engine; slip again below idle.
        if ratio == 0.0 || wheel_rpm < engine.idle_rpm * 0.9 {
            self.locked = false;
        } else if wheel_rpm >= self.rpm - 50.0 {
            self.locked = true;
        }

        if self.locked {
            self.rpm = wheel_rpm;
            WheelDrive {
                torque: engine_torque(self.rpm) * ratio,
                inertia: engine.inertia * ratio * ratio,
            }
        } else {
            // Slipping clutch: it bites from 700 rpm above idle and passes its full torque
            // 1500 rpm higher, so the engine pulls away at about 2500 rpm on full throttle.
            // While shifting or in neutral it's open, and after a shift it closes gradually.
            let bite = ((self.rpm - engine.idle_rpm - 700.0) / 1500.0).clamp(0.0, 1.0);
            let clutch = if ratio == 0.0 {
                0.0
            } else {
                engine.clutch_torque * bite * bite * (3.0 - 2.0 * bite) * self.engaged
            };
            let net = engine_torque(self.rpm) - clutch;
            self.rpm = (self.rpm + net / engine.inertia * dt * RPM).max(0.0);
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
                inertia: engine.inertia * ratio * ratio,
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
            self.rpm = (self.rpm + net / engine.inertia * dt * RPM).max(0.0);
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
        if self.shift.is_some() || self.gear < 1 {
            return;
        }
        let rpm_in = |gear: i8| engine.rpm_in(gear, wheel_omega);
        let up = 3000.0 + (engine.redline_rpm - 300.0 - 3000.0) * throttle;
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
            (drivetrain.rpm - engine.idle_rpm).abs() < 100.0,
            "rpm {}",
            drivetrain.rpm
        );
        // Full throttle: the clutch bites, then locks.
        let speed = drive(&mut drivetrain, &engine, 1.0, 3.0);
        assert!(speed > 8.0, "speed {speed}");
        assert!(drivetrain.locked);
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
            gear >= 2 && engine.rpm_in(gear, omega) <= engine.redline_rpm,
            "gear {gear}"
        );
        // Stopped: down through first and neutral to reverse, and no further.
        for _ in 0..10 {
            drivetrain.shift_by(&engine, -1, 0.0, 0.0);
        }
        assert_eq!(drivetrain.selected_gear(), -1);
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
        assert!(!drivetrain.running && drivetrain.fuel == 0.0);
        drivetrain.start(&engine);
        drive_with_clutch(&mut drivetrain, &engine, 0.0, |_| 1.0, 1.0);
        assert!(drivetrain.running);
        assert!(
            (drivetrain.rpm - engine.idle_rpm).abs() < 100.0,
            "rpm {}",
            drivetrain.rpm
        );
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
            let fuelled = drivetrain.fuel > 0.0;
            cuts += (was_fuelled && !fuelled) as u32;
            was_fuelled = fuelled;
        }
        // Roughly ten cuts a second (free revving overshoots, which can cut early), each
        // dropping the revs noticeably.
        assert!((6..=14).contains(&cuts), "{cuts} cuts");
        assert!(high - low > 150.0, "rpm {low}..{high}");
        assert!(high < engine.redline_rpm + 300.0, "rpm {high}");
    }

    #[test]
    fn rev_limiter_holds_redline() {
        let engine = EngineSetup::default();
        let mut drivetrain = Drivetrain::new(&engine);
        drive(&mut drivetrain, &engine, 1.0, 20.0);
        assert!(
            drivetrain.rpm < engine.redline_rpm + 300.0,
            "rpm {}",
            drivetrain.rpm
        );
        assert!(
            drivetrain.rpm > engine.redline_rpm - 300.0,
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
