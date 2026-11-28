//! Engine, clutch and gearbox driving the rear wheels, the front wheels or all four.
//!
//! While the clutch is locked, the engine turns with the driven wheels (through the gear
//! and final drive ratios): its torque goes to the wheels and its inertia is added to
//! theirs, so the wheel-spin update in the corners stays implicit and stable. The clutch
//! slips when pulling away, while shifting, and below idle speed. Then the engine spins
//! freely and the clutch passes a torque that depends on engine speed (like a centrifugal
//! clutch) and on how long the driver has been on the gas (like a foot coming up off the
//! pedal), so it never couples the engine stiffly to the wheels at the 120 Hz chassis
//! rate. Pulling away in too high a gear, it drags the engine down and stalls it.
//!
//! With a clutch pedal, the driver works the clutch instead: it passes up to its capacity
//! (a share of it by the pedal's travel, `ENGAGEMENT`: free play at both ends, a biting
//! point) in the direction it slips, and locks once the engine and wheels meet. Then the
//! engine can be dragged below idle and stall.

use std::f32::consts::PI;

use super::engine::parts::Parts;
use super::engine::{Engine, EngineBuild};

/// Engine speed to rpm.
const RPM: f32 = 60.0 / (2.0 * PI);
/// Below this, a running engine stalls.
const STALL_RPM: f32 = 400.0;
/// The starter motor: its torque at the crank standing (N·m) and the speed it can't turn
/// the engine past (rpm), falling straight in between.
const STARTER_TORQUE: f32 = 60.0;
const STARTER_SPEED: f32 = 350.0;
/// Cranked past this by its own firing, the engine has caught and runs.
const CATCH_RPM: f32 = 550.0;
/// Revolutions the starter turns the engine before the ECU has seen where the crank is
/// and fires (crank and cam signals in sync).
const SYNC_TURNS: f32 = 2.0;
/// An H-shifter's gear goes in without the clutch when the engine turns within this of
/// the gear's speed (the synchros manage it): engine off and standing, or rev-matched.
const SYNC_RPM: f32 = 150.0;
/// The automatic clutch blips the throttle on a downshift: fully open this far below the
/// lower gear's speed, closing towards it.
const BLIP_RPM: f32 = 400.0;
/// After an upshift with the automatic clutch (no clutch control bound), the throttle
/// stays shut until the engine is down within this of the new gear's speed (rpm), then
/// comes back over `CUT_FADE` (s); at the latest after `CUT_MOST` (s) in the gear.
const CUT_RPM: f32 = 50.0;
const CUT_FADE: f32 = 0.1;
const CUT_MOST: f32 = 0.6;
/// The automatic clutch pulling away: it starts to bite this far above idle (rpm) and
/// passes its full torque `BITE_SPAN` higher.
const BITE_ABOVE_IDLE: f32 = 150.0;
const BITE_SPAN: f32 = 750.0;
/// On the gas, the automatic clutch closes fully within this (s) whatever the engine
/// speed, as a driver lets the pedal up: in too high a gear the engine can't keep up.
const FEED_TIME: f32 = 0.8;
/// Throttle (as the engine gets it, after traction control) under which the automatic
/// clutch opens again by time, over `FEED_BACK` (s): like a driver's foot going back down
/// lifting off, and so traction control cutting the gas doesn't stall the engine.
const FEED_GAS: f32 = 0.15;
const FEED_BACK: f32 = 0.3;
/// The clutch's engagement, pedal travel (0 released, 1 floored) to the share of its
/// torque capacity it passes (linear in between). Like a real one, the pedal has free
/// play at the top (fully engaged) and the bottom (fully apart), and the plates meet at
/// a biting point a little over halfway down: from there they pass only a little torque,
/// enough to creep at idle, rising steeply towards the top.
const ENGAGEMENT: [(f32, f32); 9] = [
    (0.0, 1.0),
    (0.12, 1.0),
    (0.22, 0.6),
    (0.32, 0.3),
    (0.4, 0.15),
    (0.47, 0.075),
    (BITE, 0.03),
    (0.64, 0.008),
    (CLUTCH_APART, 0.0),
];
/// Pedal travel at the biting point, and from where the plates are apart.
const BITE: f32 = 0.55;
const CLUTCH_APART: f32 = 0.72;
/// Clutch pedal travel beyond which a gear change needs no help from the automatic clutch:
/// the plates are apart.
const CLUTCH_IN: f32 = CLUTCH_APART;

/// Share of the clutch's capacity it passes with the pedal pressed to `travel` (0..1).
pub fn clutch_engagement(travel: f32) -> f32 {
    let travel = travel.clamp(0.0, 1.0);
    match ENGAGEMENT.iter().position(|&(p, _)| p > travel) {
        Some(i) => {
            let ((p0, s0), (p1, s1)) = (ENGAGEMENT[i - 1], ENGAGEMENT[i]);
            s0 + (s1 - s0) * (travel - p0) / (p1 - p0)
        }
        None => 0.0,
    }
}
/// The most forward gears a gearbox's wear is kept for.
pub const MAX_GEARS: usize = 8;
/// Energy the clutch takes slipping until it's worn out, per N·m it passes new (J/N·m).
const CLUTCH_LIFE: f32 = 500e3;
/// Worn past this the clutch passes less, down to `CLUTCH_WORN` of its torque just before
/// it's worn out; worn out, `CLUTCH_GONE`.
const CLUTCH_WEARING: f32 = 0.6;
const CLUTCH_WORN: f32 = 0.5;
const CLUTCH_GONE: f32 = 0.15;
/// A synchro brings the clutch disc and the input shaft (this inertia, kg·m²) to the
/// gear's speed, wearing by the energy that takes: this much wears one out (J).
const SYNCHRO_INERTIA: f32 = 0.012;
const SYNCHRO_LIFE: f32 = 3e6;
/// Worn past this, a synchro can't bring a gear in from as far off its speed: from
/// `SYNCHRO_REACH` (rpm) down to `SYNC_RPM` worn out (only rev-matched, then).
const SYNCHRO_WEARING: f32 = 0.5;
const SYNCHRO_REACH: f32 = 3000.0;
/// A gear passing more than the gearbox takes wears this much a second per share over.
const OVERLOAD_WEAR: f32 = 1.0 / 600.0;
/// A gear refused grinds this long (s).
const GRIND_TIME: f32 = 0.3;

/// The share of its torque a clutch worn `wear` (0 new, 1 worn out) passes.
pub fn clutch_grip(wear: f32) -> f32 {
    if wear >= 1.0 {
        CLUTCH_GONE
    } else if wear > CLUTCH_WEARING {
        1.0 - (1.0 - CLUTCH_WORN) * (wear - CLUTCH_WEARING) / (1.0 - CLUTCH_WEARING)
    } else {
        1.0
    }
}

/// How far off a gear's speed (rpm) a synchro worn `wear` still brings it in from.
pub fn synchro_reach(wear: f32) -> f32 {
    if wear < SYNCHRO_WEARING {
        f32::INFINITY
    } else {
        let left = (1.0 - (wear - SYNCHRO_WEARING) / (1.0 - SYNCHRO_WEARING)).max(0.0);
        SYNC_RPM + (SYNCHRO_REACH - SYNC_RPM) * left
    }
}

/// Reverse only goes in below this speed (m/s): at a standstill, near enough.
pub const REVERSE_SPEED: f32 = 2.0;
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

    /// Whether the engine drives `axle` (0 front, 1 rear), if only through the centre
    /// coupling (all-wheel drive with the whole share at the front).
    pub fn drives(self, axle: usize) -> bool {
        match self {
            Drive::Front => axle == 0,
            Drive::Rear => axle == 1,
            Drive::All { .. } => true,
        }
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
    /// Most torque the clutch can pass while slipping (N·m), new.
    pub clutch_torque: f32,
    /// Most torque the gearbox takes for good (N·m at its input shaft).
    pub gearbox_torque: f32,
    /// Forward gear ratios from first, the reverse ratio, and the final drive.
    pub gears: Vec<f32>,
    pub reverse: f32,
    pub final_drive: f32,
    /// Time the clutch stays open for a gear change, and then takes to close again (s).
    pub shift_time: f32,
    pub engage_time: f32,
    pub drive: Drive,
    /// The differentials in the front and the rear axle (each counts only on a driven
    /// axle), and the centre coupling between the axles with all-wheel drive.
    pub differentials: [Differential; 2],
    pub centre: Differential,
}

#[cfg(any(test, not(target_arch = "wasm32")))]
impl Default for EngineSetup {
    /// A 2-litre petrol four with a six-speed gearbox.
    fn default() -> Self {
        Self {
            parts: Parts::stock(),
            build: EngineBuild::default(),
            clutch_torque: 350.0,
            gearbox_torque: 400.0,
            gears: vec![3.3, 2.1, 1.5, 1.17, 0.95, 0.8],
            reverse: 3.3,
            final_drive: 3.7,
            shift_time: 0.25,
            engage_time: 0.15,
            drive: Drive::Rear,
            differentials: [Differential::OPEN; 2],
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

/// How hard an engine fires with its cylinders `fill`ed (see `Drivetrain::fill`): 0 at
/// idle, 1 at full load without boost, more on boost.
pub fn firing_load(fill: f32) -> f32 {
    ((fill - 0.2) / 0.7).max(0.0)
}

/// How full the cylinders are when an engine fires at `load` (the other way round).
pub fn fill_for(load: f32) -> f32 {
    0.2 + 0.7 * load
}

#[derive(Clone, Debug)]
pub struct Drivetrain {
    pub rpm: f32,
    /// False once stalled, until started again.
    pub running: bool,
    /// -1 reverse, 0 neutral, 1 and up forward.
    pub gear: i8,
    /// Shifts by itself, based on rpm and throttle: only for the test drivers, the player
    /// shifts by hand (real automatic gearboxes are to come).
    pub automatic: bool,
    /// Gear being shifted into, and the time left until the clutch closes again.
    shift: Option<(i8, f32)>,
    /// Time since the last gear change went in.
    in_gear: f32,
    /// How far the clutch has closed again after a shift (0..1).
    engaged: f32,
    /// Share of the driver's throttle let through after an upshift with the automatic
    /// clutch (0 cut until the engine has come down to the gear's speed, 1 all).
    cut: f32,
    /// How far the automatic clutch has closed pulling away by time on the gas (0..1; 1
    /// once locked), as a driver's foot coming up.
    feed: f32,
    locked: bool,
    pub engine: Engine,
    /// How hard the engine fired in the last step (0 at idle or cut, 1 at full load).
    pub load: f32,
    /// How full the cylinders were in the last step: air per cycle relative to filling
    /// the displacement at ambient (above 1 on boost; 0 when not firing).
    pub fill: f32,
    /// Clutch pedal travel (0 released, 1 pressed) in the last step, if there's a pedal.
    clutch_pedal: Option<f32>,
    /// Engine minus wheel rpm through the pedal clutch in the last step, while slipping.
    slip: f32,
    /// The H-shifter's gate the lever is held in while its gear can't go in (the clutch
    /// pedal not pressed, or reverse while rolling forward): the gears grind.
    pub grinding: Option<i8>,
    /// The starter button held (the driver's, set before each step): cranks the engine
    /// with the ignition on while it isn't running.
    pub starter: bool,
    /// Switched off by the driver (not stalled): the automatic clutch doesn't start it
    /// again by itself.
    switched_off: bool,
    /// Cranking in the last step, and the revolutions turned since the starter engaged.
    cranking: bool,
    turned: f32,
    /// The H-shifter's gate seen in the last step (`None` before any).
    lever: Option<i8>,
    /// Put in neutral by the game (`neutral`): the gate the lever is still in counts as
    /// neutral until the lever moves, so a reset or a round's end doesn't put the car
    /// straight back in the gear the lever was left in.
    parked: bool,
    /// How worn the clutch is (0 new, 1 worn out): it slips under less torque.
    pub clutch_wear: f32,
    /// How worn each forward gear and its synchro is (0 new, 1 worn out): worn, it grinds
    /// unless shifted gently, worn out it only goes in rev-matched.
    pub gear_wear: [f32; MAX_GEARS],
    /// The gear in the last step, and the input shaft's speed then (rpm): turning with
    /// the gear in gear, with the engine in neutral with the clutch up.
    last_gear: i8,
    shaft: f32,
    /// Time left grinding after a shift a worn synchro refused (s).
    grind: f32,
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
            cut: 1.0,
            feed: 0.0,
            locked: false,
            engine: Engine::default(),
            load: 0.0,
            fill: 0.0,
            clutch_pedal: None,
            slip: 0.0,
            grinding: None,
            starter: false,
            switched_off: false,
            cranking: false,
            turned: 0.0,
            lever: None,
            parked: false,
            clutch_wear: 0.0,
            gear_wear: [0.0; MAX_GEARS],
            last_gear: 1,
            shaft: engine.idle_rpm(),
            grind: 0.0,
        }
    }

    /// Most torque the clutch passes as worn as it is (N·m).
    pub fn clutch_capacity(&self, engine: &EngineSetup) -> f32 {
        engine.clutch_torque * clutch_grip(self.clutch_wear)
    }

    /// Whether the synchro of forward `gear` brings it in at the driven wheels'
    /// `wheel_omega`: the input shaft is that far off the gear's speed. It wears by the
    /// work it does.
    fn synchro(&mut self, engine: &EngineSetup, gear: i8, wheel_omega: f32) -> bool {
        let Some(wear) = self.gear_wear.get_mut(gear as usize - 1) else {
            return true;
        };
        let off = (self.shaft - engine.rpm_in(gear, wheel_omega)).abs();
        if off > synchro_reach(*wear) {
            return false;
        }
        let spun = off / RPM;
        *wear = (*wear + 0.5 * SYNCHRO_INERTIA * spun * spun / SYNCHRO_LIFE).min(1.0);
        true
    }

    /// The ignition off: the engine stops firing and runs down. It stays off until
    /// cranked again.
    pub fn switch_off(&mut self) {
        self.running = false;
        self.switched_off = true;
    }

    /// Switched off by the driver (rather than stalled).
    pub fn switched_off(&self) -> bool {
        self.switched_off
    }

    /// The starter turning the engine over.
    pub fn cranking(&self) -> bool {
        self.cranking
    }

    /// Starts a stalled engine at once, unless it's blown (for tests). The driver cranks
    /// it with `starter` instead.
    pub fn start(&mut self, engine: &EngineSetup) {
        if !self.running && !self.engine.blown(&engine.build.hardware) {
            self.running = true;
            self.rpm = self.rpm.max(engine.idle_rpm());
            self.engine.start();
        }
    }

    /// The gear shown to the driver: the one being shifted into, if shifting.
    /// Whether its gears are ones `engine`'s gearbox has (after loading a snapshot).
    pub fn fits(&self, engine: &EngineSetup) -> bool {
        let gears = -1..=engine.top_gear();
        gears.contains(&self.gear)
            && gears.contains(&self.selected_gear())
            && self.grinding.is_none_or(|gear| gears.contains(&gear))
    }

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
        if target != gear && !over_rev && (target >= 0 || speed.abs() < REVERSE_SPEED) {
            self.shift_to(engine, target);
        }
    }

    /// An H-shifter's lever in `gate` (0 between gates): the gear follows the lever. With a
    /// clutch pedal, a gear only goes in while the pedal is pressed; otherwise the box stays
    /// in neutral until it is, unless the engine already turns at the gear's speed (the
    /// driven wheels at `wheel_omega`, rad/s averaged). Reverse only goes in when (nearly)
    /// stopped (`speed` in m/s).
    pub fn select_gate(&mut self, engine: &EngineSetup, gate: i8, speed: f32, wheel_omega: f32) {
        // A shifter's gates past the gearbox's top gear give the top gear.
        let gate = gate.min(engine.top_gear());
        // After `neutral`, the lever has to move into a gate before it counts.
        if self.parked && self.lever.is_none_or(|before| before == gate) {
            self.lever = Some(gate);
            self.grinding = None;
            return;
        }
        self.parked = false;
        self.lever = Some(gate);
        let lever = gate;
        // Reverse is refused while rolling forward (backing up quickly must keep it).
        let gate = if gate < 0 && speed > REVERSE_SPEED {
            0
        } else {
            gate
        };
        if gate != self.selected_gear() {
            let clutch_in = self.clutch_pedal.is_some_and(|travel| travel > CLUTCH_IN);
            let synced = (self.rpm - engine.rpm_in(gate, wheel_omega)).abs() < SYNC_RPM;
            if gate == 0 || self.clutch_pedal.is_none() || clutch_in {
                self.shift_to(engine, gate);
            } else if synced {
                // Slides in with the clutch up: no break in the drive.
                self.gear = gate;
                self.shift = None;
                self.in_gear = 0.0;
                self.engaged = 1.0;
                self.locked = false;
            } else {
                self.shift_to(engine, 0);
            }
        }
        self.grinding = (lever != 0 && self.selected_gear() != lever).then_some(lever);
    }

    /// Out of gear at once, whatever the lever or a shift under way says. An H-shifter's
    /// lever counts again once it moves.
    pub fn neutral(&mut self) {
        self.parked = true;
        self.gear = 0;
        self.last_gear = 0;
        self.shift = None;
        self.in_gear = 0.0;
        self.engaged = 1.0;
        self.cut = 1.0;
        self.locked = false;
        self.grinding = None;
    }

    /// Starts a gear change; the clutch opens for `shift_time`. With the clutch pedal
    /// pressed, the gear goes in at once.
    pub fn shift_to(&mut self, engine: &EngineSetup, gear: i8) {
        let gear = gear.clamp(-1, engine.top_gear());
        if gear == self.selected_gear() {
            return;
        }
        self.cut = 1.0;
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
        let blown = self.engine.blown(&engine.build.hardware);
        if blown {
            self.running = false;
        }
        // The starter turns it over with the ignition on (a blown engine only turns).
        let cranking = self.starter && !self.running;
        if cranking {
            if !self.cranking {
                self.engine.start();
                self.turned = 0.0;
            }
            self.turned += self.rpm / 60.0 * dt;
        }
        self.cranking = cranking;
        let synced = self.turned >= SYNC_TURNS;
        self.in_gear += dt;
        if self.automatic && grounded {
            self.auto_shift(engine, throttle, wheel_omega);
        }
        if let Some((gear, left)) = &mut self.shift {
            *left -= dt;
            if *left <= 0.0 {
                if clutch.is_none() && self.gear >= 1 && *gear > self.gear {
                    self.cut = 0.0;
                }
                self.gear = *gear;
                self.shift = None;
                self.in_gear = 0.0;
            }
        }
        // A gear going in: its synchro brings the input shaft to its speed, if it can; else
        // it grinds and stays out.
        if self.grind > 0.0 {
            self.grind -= dt;
            if self.grind <= 0.0 {
                self.grinding = None;
            }
        }
        if self.gear != self.last_gear {
            if self.gear >= 1 && !self.synchro(engine, self.gear, wheel_omega) {
                self.grinding = Some(self.gear);
                self.grind = GRIND_TIME;
                self.gear = 0;
                self.locked = false;
            }
            self.last_gear = self.gear;
        }
        // While shifting, the clutch is open and the driver's throttle is cut (like a
        // sequential gearbox), so the engine doesn't flare up and slam the wheels. Going
        // down a gear, it blips the throttle up to the lower gear's speed instead.
        let blip = match self.shift {
            Some((gear, _)) if gear >= 1 && gear < self.gear => {
                let target = engine.rpm_in(gear, wheel_omega);
                ((target - self.rpm) / BLIP_RPM).clamp(0.0, 1.0)
            }
            _ => 0.0,
        };
        let (ratio, throttle) = if self.shift.is_some() {
            (0.0, blip)
        } else {
            self.engaged = (self.engaged + dt / engine.engage_time).min(1.0);
            // Up a gear, the engine comes down to the gear's speed before the throttle
            // opens again: on the gas while the clutch still slips it would flare.
            if self.cut < 1.0 {
                let matched = self.rpm <= engine.rpm_in(self.gear, wheel_omega) + CUT_RPM;
                if self.cut > 0.0 || matched || self.locked || self.in_gear > CUT_MOST {
                    self.cut = (self.cut + dt / CUT_FADE).min(1.0);
                }
            }
            (engine.ratio(self.gear), throttle * self.cut)
        };
        let build = &engine.build;
        // While shifting it doesn't fire (with the automatic clutch the ECU's overrun cut
        // does that above idle, so the engine idles on while a gear goes in from a stop).
        let firing = (self.running || (cranking && synced && !blown))
            && (self.shift.is_none() || blip > 0.0 || clutch.is_none());
        self.engine
            .step(build, throttle_plate(throttle), self.rpm, firing, dt);
        let cycle = self.engine.cycle(build, self.rpm);
        self.fill = if firing && cycle.indicated > 0.0 {
            cycle.load
        } else {
            0.0
        };
        self.load = firing_load(self.fill).min(1.0);
        let wheel_rpm = wheel_omega * ratio * RPM;
        let state = self.engine;
        let starter = move |rpm: f32| {
            if cranking {
                STARTER_TORQUE * (1.0 - rpm / STARTER_SPEED).max(0.0)
            } else {
                0.0
            }
        };
        let engine_torque = move |rpm: f32| state.torque(build, rpm) + starter(rpm);
        let most = self.clutch_capacity(engine);
        if let Some(travel) = clutch {
            let capacity = most * clutch_engagement(travel) * self.engaged;
            let drive = self.step_pedal(engine, engine_torque, capacity, ratio, wheel_rpm, dt);
            self.catch(cranking && !blown);
            self.wear(engine, &drive, ratio, wheel_rpm, wheel_omega, dt);
            return drive;
        }

        // Off the gas at a crawl, or in neutral, the driver's foot is back on the pedal:
        // closing starts over. On the gas it comes up, off it goes back down; locked,
        // it's up.
        let crawling = wheel_rpm < engine.idle_rpm() * 0.9;
        if (self.gear == 0 && self.shift.is_none()) || (crawling && throttle < FEED_GAS) {
            self.feed = 0.0;
        } else if self.locked {
            self.feed = 1.0;
        } else if throttle >= FEED_GAS {
            self.feed = (self.feed + dt / FEED_TIME).min(1.0);
        } else {
            self.feed = (self.feed - dt / FEED_BACK).max(0.0);
        }
        // Lock once the wheels have caught up with the engine; slip again below idle
        // (unless the clutch is fully up: then it drags the engine down with the wheels),
        // or when the engine makes more torque than the clutch can pass.
        let holds = engine_torque(wheel_rpm).abs() <= most;
        if ratio == 0.0 || (crawling && self.feed < 1.0) || !holds {
            self.locked = false;
        } else if wheel_rpm >= self.rpm - 50.0 {
            self.locked = true;
        }

        let drive = if self.locked {
            self.rpm = wheel_rpm;
            WheelDrive {
                torque: engine_torque(self.rpm) * ratio,
                inertia: engine.build.hardware.inertia * ratio * ratio,
            }
        } else {
            // Slipping clutch: it bites from `BITE_ABOVE_IDLE` and passes its full torque
            // `BITE_SPAN` higher, so the engine pulls away at about 1500 rpm on full
            // throttle; and it closes with time on the gas (`feed`) whatever the speed.
            // While shifting or in neutral it's open, and after a shift it closes
            // gradually. It passes up to that, as much as brings the engine to the
            // wheels' speed within the step (so either way, and never past it).
            let bite =
                ((self.rpm - engine.idle_rpm() - BITE_ABOVE_IDLE) / BITE_SPAN).clamp(0.0, 1.0);
            let share = (bite * bite * (3.0 - 2.0 * bite)).max(self.feed);
            let capacity = if ratio == 0.0 {
                0.0
            } else {
                most * share * self.engaged
            };
            let inertia = engine.build.hardware.inertia;
            let torque = engine_torque(self.rpm);
            let matching = torque - (wheel_rpm - self.rpm) / RPM * inertia / dt;
            let clutch = matching.clamp(-capacity, capacity);
            self.rpm = (self.rpm + (torque - clutch) / inertia * dt * RPM).max(0.0);
            WheelDrive {
                torque: clutch * ratio,
                inertia: 0.0,
            }
        };
        // Dragged down by the clutch, it stalls (not hunting at idle in neutral).
        if self.running && self.rpm < STALL_RPM && ratio != 0.0 {
            self.running = false;
        }
        self.catch(cranking && !blown);
        self.wear(engine, &drive, ratio, wheel_rpm, wheel_omega, dt);
        drive
    }

    /// What the step did to the clutch and the gears: the clutch wears by the work it did
    /// slipping, the gear in by the torque it passed over what the gearbox takes; and where
    /// the input shaft turns now.
    fn wear(
        &mut self,
        engine: &EngineSetup,
        drive: &WheelDrive,
        ratio: f32,
        wheel_rpm: f32,
        wheel_omega: f32,
        dt: f32,
    ) {
        if ratio != 0.0 {
            let input = drive.torque / ratio;
            let slip = (self.rpm - wheel_rpm).abs() / RPM;
            if !self.locked {
                let life = CLUTCH_LIFE * engine.clutch_torque;
                self.clutch_wear = (self.clutch_wear + input.abs() * slip * dt / life).min(1.0);
            }
            if let Some(wear) = (self.gear >= 1)
                .then(|| self.gear_wear.get_mut(self.gear as usize - 1))
                .flatten()
            {
                let over = input.abs() / engine.gearbox_torque - 1.0;
                if over > 0.0 {
                    *wear = (*wear + over * OVERLOAD_WEAR * dt).min(1.0);
                }
            }
        }
        let pressed = self.clutch_pedal.is_some_and(|travel| travel > CLUTCH_IN);
        if self.shift.is_none() && self.gear != 0 {
            self.shaft = engine.rpm_in(self.gear, wheel_omega);
        } else if self.shift.is_none() && !pressed {
            self.shaft = self.rpm;
        }
    }

    /// Cranked (with the ignition on) past what the starter alone reaches: it has caught.
    fn catch(&mut self, cranking: bool) {
        if cranking && self.rpm > CATCH_RPM {
            self.running = true;
            self.switched_off = false;
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
        if ratio == 0.0 || capacity <= 0.0 || engine_torque(self.rpm).abs() > capacity {
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
            // The wheels can overshoot the engine within a step, too. With the plates apart
            // (or in neutral) the two sides turn freely: nothing to catch up with.
            let crossed = slip * self.slip < 0.0;
            let clutch = if ratio == 0.0 || capacity <= 0.0 {
                0.0
            } else {
                capacity * slip.signum()
            };
            self.slip = if clutch == 0.0 { 0.0 } else { slip };
            let net = engine_torque(self.rpm) - clutch;
            self.rpm = (self.rpm + net / engine.build.hardware.inertia * dt * RPM).max(0.0);
            // Caught up (or overshot) within the step: lock, if the clutch can hold.
            if clutch != 0.0 && (crossed || (self.rpm - wheel_rpm) * slip <= 0.0) {
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

crate::snapshot::snapshot!(Drivetrain {
    rpm,
    running,
    gear,
    automatic,
    shift,
    in_gear,
    engaged,
    cut,
    feed,
    locked,
    engine,
    load,
    fill,
    clutch_pedal,
    slip,
    grinding,
    starter,
    switched_off,
    cranking,
    turned,
    lever,
    parked,
    clutch_wear,
    gear_wear,
    last_gear,
    shaft,
    grind,
});

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

    /// Pulls away from a standstill in `gear` at `throttle` without a clutch control for
    /// `seconds`: the speed reached (m/s) and the lowest and highest rpm on the way.
    fn pull_away(gear: i8, throttle: f32, seconds: f32) -> (Drivetrain, f32, f32, f32) {
        let engine = EngineSetup::default();
        let (radius, mass, dt) = (0.33, 1260.0, 1.0 / 120.0);
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.neutral();
        drivetrain.shift_to(&engine, gear);
        let mut omega = 0.0;
        let (mut low, mut high) = (f32::INFINITY, 0.0f32);
        for i in 0..(seconds / dt) as usize {
            // Idling into gear first, then the gas.
            let gas = if i < 60 { 0.0 } else { throttle };
            let out = drivetrain.step(&engine, gas, None, omega, true, dt);
            omega = (omega + out.torque / (mass * radius * radius + out.inertia) * dt).max(0.0);
            if i >= 60 {
                low = low.min(drivetrain.rpm);
                high = high.max(drivetrain.rpm);
            }
        }
        (drivetrain, omega * radius, low, high)
    }

    #[test]
    fn the_automatic_clutch_pulls_away_low_in_first() {
        for throttle in [0.3, 0.6, 1.0] {
            let (drivetrain, speed, low, _) = pull_away(1, throttle, 4.0);
            assert!(drivetrain.running && drivetrain.locked, "at {throttle}");
            assert!(speed > 3.0, "{speed} m/s at {throttle}");
            assert!(low > 700.0, "down to {low} rpm at {throttle}");
        }
        // Floored, it slips at well under 2000 rpm until the wheels catch up.
        let (_, _, _, high) = pull_away(1, 1.0, 1.0);
        assert!(high > 1000.0 && high < 1900.0, "{high} rpm");
    }

    #[test]
    fn the_gas_cut_pulling_away_opens_the_automatic_clutch_again() {
        // Traction control cutting the gas against wheelspin: the clutch lets go again
        // instead of dragging the engine down.
        let engine = EngineSetup::default();
        let dt = 1.0 / 120.0;
        let mut drivetrain = Drivetrain::new(&engine);
        for i in 0..240 {
            let gas = if i < 30 { 1.0 } else { 0.08 };
            drivetrain.step(&engine, gas, None, 0.0, true, dt);
        }
        assert!(drivetrain.running);
        assert!(drivetrain.rpm > 700.0, "rpm {}", drivetrain.rpm);
    }

    #[test]
    fn the_automatic_clutch_stalls_pulling_away_in_third() {
        for throttle in [0.3, 1.0] {
            let (drivetrain, speed, ..) = pull_away(3, throttle, 4.0);
            assert!(!drivetrain.running, "at {throttle}");
            assert!(speed < 2.0, "{speed} m/s at {throttle}");
        }
        // Stalled, it stays off until started: no gas brings it back.
        let engine = EngineSetup::default();
        let (mut drivetrain, ..) = pull_away(3, 1.0, 4.0);
        drivetrain.neutral();
        for _ in 0..240 {
            drivetrain.step(&engine, 1.0, None, 0.0, true, 1.0 / 120.0);
        }
        assert!(!drivetrain.running && drivetrain.rpm < 1.0);
        drivetrain.starter = true;
        for _ in 0..240 {
            drivetrain.step(&engine, 0.0, None, 0.0, true, 1.0 / 120.0);
        }
        assert!(drivetrain.running);
    }

    #[test]
    fn the_automatic_clutch_opens_braking_to_a_stop() {
        let engine = EngineSetup::default();
        let dt = 1.0 / 120.0;
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.neutral();
        drivetrain.shift_to(&engine, 4);
        let mut omega = 2000.0 / engine.rpm_in(4, 1.0);
        for _ in 0..120 {
            drivetrain.step(&engine, 0.3, None, omega, true, dt);
        }
        assert!(drivetrain.locked);
        while omega > 0.0 {
            drivetrain.step(&engine, 0.0, None, omega, true, dt);
            omega = (omega - 8.0 * dt).max(0.0);
        }
        for _ in 0..120 {
            drivetrain.step(&engine, 0.0, None, 0.0, true, dt);
        }
        assert!(drivetrain.running);
        assert!(
            (drivetrain.rpm - engine.idle_rpm()).abs() < 100.0,
            "rpm {}",
            drivetrain.rpm
        );
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
    fn a_worn_clutch_slips_and_wears_on() {
        let engine = EngineSetup {
            clutch_torque: 230.0,
            ..EngineSetup::default()
        };
        let mut new = Drivetrain::new(&engine);
        let mut worn = Drivetrain::new(&engine);
        worn.clutch_wear = 0.95;
        let fast = drive(&mut new, &engine, 1.0, 4.0);
        let slow = drive(&mut worn, &engine, 1.0, 4.0);
        assert!(new.locked && !worn.locked);
        assert!(slow < 0.9 * fast, "{slow} against {fast} m/s");
        assert!(worn.clutch_wear > 0.951, "{}", worn.clutch_wear);
        // Pulling away wears a new one a little.
        assert!(
            new.clutch_wear > 0.0 && new.clutch_wear < 0.01,
            "{}",
            new.clutch_wear
        );
    }

    #[test]
    fn worn_synchros_grind_then_refuse_the_gear() {
        let engine = EngineSetup::default();
        let dt = 1.0 / 120.0;
        // From second into third with the clutch pressed, the wheels at `omega`, third's
        // synchro worn `wear`; double-clutched (the revs matched in neutral with the clutch
        // up) or not.
        let shift = |wear: f32, omega: f32, double: bool| {
            let mut drivetrain = Drivetrain::new(&engine);
            drivetrain.gear_wear[2] = wear;
            drivetrain.gear = 2;
            drivetrain.last_gear = 2;
            drivetrain.step(&engine, 0.2, Some(0.0), omega, true, dt);
            if double {
                drivetrain.step(&engine, 0.0, Some(1.0), omega, true, dt);
                drivetrain.select_gate(&engine, 0, 10.0, omega);
                drivetrain.rpm = engine.rpm_in(3, omega);
                drivetrain.step(&engine, 0.0, Some(0.0), omega, true, dt);
            }
            drivetrain.step(&engine, 0.0, Some(1.0), omega, true, dt);
            drivetrain.select_gate(&engine, 3, 10.0, omega);
            drivetrain.step(&engine, 0.0, Some(1.0), omega, true, dt);
            drivetrain
        };
        assert_eq!(shift(0.0, 30.0, false).gear, 3);
        let new = shift(0.0, 30.0, false).gear_wear[2];
        assert!(new > 0.0 && new < 1e-3, "{new}");
        // Worn, a gentle shift still goes in, a quick one grinds.
        assert_eq!(shift(0.8, 30.0, false).gear, 3);
        let quick = shift(0.8, 75.0, false);
        assert_eq!((quick.gear, quick.grinding), (0, Some(3)));
        // Worn out, it only goes in rev-matched.
        assert_eq!(shift(1.0, 30.0, false).gear, 0);
        assert_eq!(shift(1.0, 30.0, true).gear, 3);
        // With the automatic clutch, a refused gear leaves the box in neutral, grinding a
        // moment.
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.gear_wear[1] = 1.0;
        drivetrain.step(&engine, 0.5, None, 40.0, true, dt);
        drivetrain.shift_to(&engine, 2);
        for _ in 0..40 {
            drivetrain.step(&engine, 0.5, None, 40.0, true, dt);
        }
        assert_eq!((drivetrain.gear, drivetrain.grinding), (0, Some(2)));
        for _ in 0..60 {
            drivetrain.step(&engine, 0.0, None, 40.0, true, dt);
        }
        assert_eq!(drivetrain.grinding, None);
    }

    #[test]
    fn torque_over_what_the_gearbox_takes_wears_the_gear() {
        let strong = EngineSetup::default();
        let weak = EngineSetup {
            gearbox_torque: 120.0,
            ..strong.clone()
        };
        let mut fine = Drivetrain::new(&strong);
        let mut overloaded = Drivetrain::new(&weak);
        drive(&mut fine, &strong, 1.0, 4.0);
        drive(&mut overloaded, &weak, 1.0, 4.0);
        assert_eq!(fine.gear_wear[0], 0.0);
        assert!(overloaded.gear_wear[0] > 1e-3, "{:?}", overloaded.gear_wear);
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
    fn the_clutch_has_free_play_and_a_biting_point() {
        assert_eq!(clutch_engagement(0.0), 1.0);
        assert_eq!(clutch_engagement(0.1), 1.0);
        assert_eq!(clutch_engagement(CLUTCH_APART), 0.0);
        assert_eq!(clutch_engagement(1.0), 0.0);
        let mut last = 1.0;
        for i in 1..=100 {
            let share = clutch_engagement(i as f32 / 100.0);
            assert!(share <= last);
            last = share;
        }
        let engine = EngineSetup::default();
        // Stalled in first: started with the pedal not quite floored, it keeps running.
        let mut drivetrain = Drivetrain::new(&engine);
        drive_with_clutch(&mut drivetrain, &engine, 0.0, |_| 0.0, 1.0);
        assert!(!drivetrain.running);
        drivetrain.start(&engine);
        drive_with_clutch(&mut drivetrain, &engine, 0.0, |_| 0.8, 1.0);
        assert!(drivetrain.running);
        // Letting the pedal up a little doesn't stall it or move the car.
        let speed = drive_with_clutch(&mut drivetrain, &engine, 0.0, |_| 0.75, 2.0);
        assert!(drivetrain.running && speed < 0.01, "speed {speed}");
        // Held just past the biting point, the car pulls away on the idle alone.
        let speed = drive_with_clutch(&mut drivetrain, &engine, 0.0, |_| 0.47, 5.0);
        assert!(drivetrain.running && speed > 2.0, "speed {speed}");
        // With a little throttle, letting it out in half a second pulls away too.
        let mut drivetrain = Drivetrain::new(&engine);
        drive_with_clutch(&mut drivetrain, &engine, 0.1, |_| 1.0, 1.0);
        let release = |t: f32| (1.0 - t / 0.5).max(0.0);
        let speed = drive_with_clutch(&mut drivetrain, &engine, 0.1, release, 4.0);
        assert!(drivetrain.running && speed > 5.0, "speed {speed}");
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
        drivetrain.select_gate(&engine, 3, 10.0, 30.0);
        for _ in 0..60 {
            drivetrain.step(&engine, 0.0, Some(0.0), 30.0, true, dt);
        }
        assert_eq!(drivetrain.gear, 0);
        assert_eq!(drivetrain.grinding, Some(3));
        // Pressing the clutch with the lever still in third puts it in, at once.
        drivetrain.step(&engine, 0.0, Some(1.0), 30.0, true, dt);
        drivetrain.select_gate(&engine, 3, 10.0, 30.0);
        assert_eq!(drivetrain.gear, 3);
        assert_eq!(drivetrain.grinding, None);
        // Out of gear needs no clutch; reverse is refused while rolling.
        drivetrain.step(&engine, 0.0, Some(0.0), 30.0, true, dt);
        drivetrain.select_gate(&engine, 0, 10.0, 30.0);
        drivetrain.step(&engine, 0.0, Some(1.0), 30.0, true, dt);
        drivetrain.select_gate(&engine, -1, 10.0, 30.0);
        assert_eq!(drivetrain.selected_gear(), 0);
        assert_eq!(drivetrain.grinding, Some(-1));
        // Stopped, it goes in, and stays in while backing up quickly.
        drivetrain.select_gate(&engine, -1, 0.0, 30.0);
        assert_eq!(drivetrain.gear, -1);
        drivetrain.step(&engine, 0.0, Some(0.0), -30.0, true, dt);
        drivetrain.select_gate(&engine, -1, -5.0, -30.0);
        assert_eq!(drivetrain.selected_gear(), -1);
        // Without a pedal, the automatic clutch puts the gear in.
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.select_gate(&engine, 2, 10.0, 30.0);
        for _ in 0..60 {
            drivetrain.step(&engine, 0.0, None, 30.0, true, dt);
        }
        assert_eq!(drivetrain.gear, 2);
        // A gate past the top gear (a 7-gate shifter on a smaller box) gives the top gear.
        let top = engine.top_gear();
        drivetrain.select_gate(&engine, top + 2, 10.0, 30.0);
        for _ in 0..60 {
            drivetrain.step(&engine, 0.0, None, 30.0, true, dt);
        }
        assert_eq!(drivetrain.gear, top);
        assert_eq!(drivetrain.grinding, None);
    }

    #[test]
    fn a_gear_goes_in_without_the_clutch_at_matched_speeds() {
        let engine = EngineSetup::default();
        let dt = 1.0 / 120.0;
        // Engine off and standing: both sides of the box are still, first slides in.
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.neutral();
        drivetrain.running = false;
        drivetrain.rpm = 0.0;
        drivetrain.step(&engine, 0.0, Some(0.0), 0.0, true, dt);
        drivetrain.select_gate(&engine, 0, 0.0, 0.0);
        drivetrain.select_gate(&engine, 1, 0.0, 0.0);
        assert_eq!(drivetrain.gear, 1);
        assert_eq!(drivetrain.grinding, None);
        // Idling while standing, it grinds.
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.neutral();
        drivetrain.step(&engine, 0.0, Some(0.0), 0.0, true, dt);
        drivetrain.select_gate(&engine, 0, 0.0, 0.0);
        drivetrain.select_gate(&engine, 1, 0.0, 0.0);
        assert_eq!(drivetrain.gear, 0);
        assert_eq!(drivetrain.grinding, Some(1));
        // Rolling with the engine revved to third's speed, third goes in.
        let omega = drivetrain.rpm / engine.rpm_in(3, 1.0);
        drivetrain.select_gate(&engine, 3, 10.0, omega);
        assert_eq!(drivetrain.gear, 3);
    }

    #[test]
    fn a_floored_clutch_lets_the_engine_turn_freely() {
        let engine = EngineSetup::default();
        let dt = 1.0 / 120.0;
        let omega = 3000.0 / engine.rpm_in(3, 1.0);
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.neutral();
        drivetrain.shift_to(&engine, 3);
        drivetrain.step(&engine, 0.0, Some(1.0), omega, true, dt);
        drivetrain.shift_to(&engine, 3);
        for _ in 0..120 {
            drivetrain.step(&engine, 0.0, Some(0.0), omega, true, dt);
        }
        assert!((drivetrain.rpm - 3000.0).abs() < 10.0);
        // Clutch floored, out of gear, then into second: the engine falls towards idle
        // and isn't held at second's speed (4200 rpm here).
        for i in 0..240 {
            if i == 30 {
                drivetrain.select_gate(&engine, 0, 15.0, omega);
            }
            if i == 60 {
                drivetrain.select_gate(&engine, 2, 15.0, omega);
                assert_eq!(drivetrain.gear, 2);
            }
            drivetrain.step(&engine, 0.0, Some(1.0), omega, true, dt);
        }
        assert!(drivetrain.rpm < 1500.0, "rpm {}", drivetrain.rpm);
        // A blip gets it there; the clutch let out at matched speeds locks.
        for _ in 0..240 {
            let throttle = if drivetrain.rpm < 4100.0 { 0.6 } else { 0.0 };
            drivetrain.step(&engine, throttle, Some(1.0), omega, true, dt);
            if drivetrain.rpm > 4100.0 {
                break;
            }
        }
        for _ in 0..60 {
            drivetrain.step(&engine, 0.0, Some(0.0), omega, true, dt);
        }
        assert!(drivetrain.locked);
    }

    #[test]
    fn put_in_neutral_the_lever_has_to_move_first() {
        let engine = EngineSetup::default();
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.step(&engine, 0.0, Some(1.0), 0.0, true, 1.0 / 120.0);
        drivetrain.select_gate(&engine, 2, 0.0, 0.0);
        assert_eq!(drivetrain.gear, 2);
        // A reset: the lever left in second doesn't count, nor grind with the clutch up.
        drivetrain.neutral();
        drivetrain.step(&engine, 0.0, Some(0.0), 0.0, true, 1.0 / 120.0);
        drivetrain.select_gate(&engine, 2, 0.0, 0.0);
        assert_eq!((drivetrain.gear, drivetrain.grinding), (0, None));
        // Moved into another gate (or out and back in), it counts again.
        drivetrain.select_gate(&engine, 1, 0.0, 0.0);
        assert_eq!(drivetrain.grinding, Some(1));
        // A car put down afresh doesn't know where the lever was: the first gate seen waits.
        let mut fresh = Drivetrain::new(&engine);
        fresh.neutral();
        fresh.select_gate(&engine, 3, 0.0, 0.0);
        assert_eq!(fresh.gear, 0);
        fresh.select_gate(&engine, 0, 0.0, 0.0);
        fresh.select_gate(&engine, 3, 0.0, 0.0);
        assert_eq!(fresh.selected_gear(), 3);
    }

    #[test]
    fn the_automatic_clutch_blips_on_a_downshift() {
        let engine = EngineSetup::default();
        let dt = 1.0 / 120.0;
        let omega = 2500.0 / engine.rpm_in(3, 1.0);
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.neutral();
        drivetrain.shift_to(&engine, 3);
        for _ in 0..240 {
            drivetrain.step(&engine, 0.0, None, omega, true, dt);
        }
        assert!(drivetrain.locked && drivetrain.gear == 3);
        drivetrain.shift_by(&engine, -1, omega, 15.0);
        let target = engine.rpm_in(2, omega);
        let mut top = 0.0f32;
        while drivetrain.shift.is_some() {
            drivetrain.step(&engine, 0.0, None, omega, true, dt);
            top = top.max(drivetrain.rpm);
        }
        // Up to second's speed by the end of the shift, without flaring far past it.
        assert!(
            (drivetrain.rpm - target).abs() < 300.0,
            "rpm {} for {target}",
            drivetrain.rpm
        );
        assert!(top < target + 400.0, "flared to {top} for {target}");
    }

    #[test]
    fn the_automatic_clutch_cuts_the_throttle_up_a_gear_until_the_speeds_match() {
        let engine = EngineSetup::default();
        let (radius, mass, dt) = (0.33, 1260.0, 1.0 / 120.0);
        let mut drivetrain = Drivetrain::new(&engine);
        let mut omega = 0.0;
        let step = |drivetrain: &mut Drivetrain, omega: &mut f32| {
            let out = drivetrain.step(&engine, 1.0, None, *omega, true, dt);
            *omega += out.torque / (mass * radius * radius + out.inertia) * dt;
        };
        while drivetrain.rpm < 6000.0 || !drivetrain.locked {
            step(&mut drivetrain, &mut omega);
        }
        drivetrain.shift_by(&engine, 1, omega, omega * radius);
        while drivetrain.shift.is_some() {
            step(&mut drivetrain, &mut omega);
        }
        // Into second, the engine is still far above its speed: no gas until it's down.
        assert!(drivetrain.rpm > engine.rpm_in(2, omega) + 500.0);
        let mut time = 0.0;
        while !drivetrain.locked {
            step(&mut drivetrain, &mut omega);
            assert!(drivetrain.load < 0.05, "load {} slipping", drivetrain.load);
            time += dt;
            assert!(time < 0.5, "still slipping");
        }
        // Then the gas comes back.
        for _ in 0..30 {
            step(&mut drivetrain, &mut omega);
        }
        assert!(drivetrain.load > 0.5, "load {}", drivetrain.load);
        // With a clutch control bound, the driver does it: the gas is there at once.
        let mut pedal = Drivetrain::new(&engine);
        let omega = 6000.0 / engine.rpm_in(1, 1.0);
        for _ in 0..120 {
            pedal.step(&engine, 1.0, Some(0.0), omega, true, dt);
        }
        pedal.shift_by(&engine, 1, omega, omega * radius);
        while pedal.shift.is_some() {
            pedal.step(&engine, 1.0, Some(0.0), omega, true, dt);
        }
        pedal.step(&engine, 1.0, Some(0.0), omega, true, dt);
        assert!(pedal.cut == 1.0 && pedal.load > 0.5, "load {}", pedal.load);
    }

    #[test]
    fn every_factory_engine_starts_on_the_starter() {
        let catalogue = crate::catalogue::builtin();
        let dt = 1.0 / 120.0;
        for (f, family) in catalogue.families.iter().enumerate() {
            for (v, version) in family.versions.iter().enumerate() {
                let parts = crate::physics::engine::parts::Parts::of_version(&catalogue, f, v);
                let engine = EngineSetup {
                    build: EngineBuild::of_parts(&parts),
                    parts,
                    ..EngineSetup::default()
                };
                let mut drivetrain = Drivetrain::new(&engine);
                drivetrain.neutral();
                // Switched off, it runs down and stays off, even with the automatic clutch.
                drivetrain.switch_off();
                for _ in 0..(3.0 / dt) as usize {
                    drivetrain.step(&engine, 0.0, None, 0.0, true, dt);
                }
                assert!(
                    !drivetrain.running && drivetrain.rpm < 1.0,
                    "{}",
                    version.code
                );
                // Held on the starter, it fires up by itself.
                drivetrain.starter = true;
                let mut t = 0.0;
                while !drivetrain.running {
                    drivetrain.step(&engine, 0.0, None, 0.0, true, dt);
                    t += dt;
                    assert!(
                        t < 3.0,
                        "{} doesn't catch: {} rpm",
                        version.code,
                        drivetrain.rpm
                    );
                }
                drivetrain.starter = false;
                for _ in 0..(4.0 / dt) as usize {
                    drivetrain.step(&engine, 0.0, None, 0.0, true, dt);
                }
                let idle = engine.idle_rpm();
                println!(
                    "{}: caught after {t:.2} s, idles at {:.0} rpm ({idle:.0})",
                    version.code, drivetrain.rpm
                );
                assert!(drivetrain.running);
                assert!(
                    (drivetrain.rpm - idle).abs() < 100.0,
                    "{}: {} rpm",
                    version.code,
                    drivetrain.rpm
                );
            }
        }
    }

    #[test]
    fn the_starter_alone_doesnt_run_the_engine() {
        let engine = EngineSetup::default();
        let dt = 1.0 / 120.0;
        let mut drivetrain = Drivetrain::new(&engine);
        drivetrain.neutral();
        drivetrain.switch_off();
        // Without fuel it only turns over at the starter's speed, and stops when let go.
        drivetrain.engine.fuel_supply = 0.0;
        drivetrain.starter = true;
        for _ in 0..(2.0 / dt) as usize {
            drivetrain.step(&engine, 0.0, Some(1.0), 0.0, true, dt);
        }
        assert!(drivetrain.cranking() && !drivetrain.running);
        assert!(
            (150.0..STARTER_SPEED).contains(&drivetrain.rpm),
            "{} rpm",
            drivetrain.rpm
        );
        drivetrain.starter = false;
        for _ in 0..(2.0 / dt) as usize {
            drivetrain.step(&engine, 0.0, Some(1.0), 0.0, true, dt);
        }
        assert!(!drivetrain.running && drivetrain.rpm < 1.0);
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
