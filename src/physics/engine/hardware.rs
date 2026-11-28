//! The engine's parts, as a garage would change them.

use std::f32::consts::PI;

/// Compressor wheel tip speed it's made for (m/s).
const TIP_SPEED_LIMIT: f32 = 520.0;
/// Share of the rods' strength their pull at the redline stays below.
pub const ROD_MARGIN: f32 = 0.7;
/// A rod buckles pushing at this many times the pull that breaks it.
pub const BUCKLING: f32 = 2.5;
/// Crank angle of the highest cylinder pressure, about (rad after top dead centre).
const PEAK_ANGLE: f32 = 12.0 * PI / 180.0;
/// Share of the valve float speed the redline stays below.
pub const FLOAT_MARGIN: f32 = 0.95;

/// Everything about the engine that is hardware. The cylinder side (bottom end, head,
/// cams, runners, headers) shapes the baked maps, so changing it means baking them again;
/// the parts between the manifolds and the outside (filter, throttle, idle valve, exhaust
/// restrictions) act live.
#[derive(Clone, Debug, PartialEq)]
pub struct EngineHardware {
    pub name: String,
    pub cylinders: u32,
    /// Bore and stroke (m).
    pub bore: f32,
    pub stroke: f32,
    /// Connecting rod length, centre to centre (m).
    pub rod_length: f32,
    pub compression_ratio: f32,
    pub bottom_end: BottomEnd,
    /// Highest cylinder pressure the head gasket seals for good (Pa).
    pub head_gasket: f32,
    /// Cam profiles the valvetrain can switch between (like VTEC); one for fixed cams.
    pub cams: Vec<CamProfile>,
    /// How far the cam phasers can advance the intake cam and retard the exhaust cam
    /// (crank degrees; zero without phasers).
    pub intake_phasing: f32,
    pub exhaust_phasing: f32,
    /// Fixed cam timing, as set with adjustable cam gears (crank degrees; intake advanced,
    /// exhaust retarded), on top of the phasers.
    pub intake_timing: f32,
    pub exhaust_timing: f32,
    pub valves: ValveTrain,
    /// Where the cams sit and how many valves each cylinder has, how big.
    pub valvetrain: ValveLayout,
    /// How much the head's ports and valves flow, relative to the stock 2-litre four's
    /// (for its bore): bigger valves and ported runners flow more.
    pub port_flow: f32,
    pub intake: Intake,
    /// Turbochargers between the air filter and the throttle, and in the exhaust: `turbos`
    /// alike (one on each of the manifold's branches; 0 without), each as `turbo` says,
    /// sharing the intercooler.
    pub turbo: Option<Turbo>,
    pub turbos: u32,
    /// A supercharger between the air filter and the throttle, driven by the crank (see
    /// `supercharger`).
    pub supercharger: Option<Supercharger>,
    pub exhaust: Exhaust,
    /// The lambda sensor in the exhaust reads λ (wideband) rather than only rich or lean
    /// (narrowband).
    pub wideband: bool,
    pub cooling: Cooling,
    /// Flywheel and crank inertia (kg·m²).
    pub inertia: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamProfile {
    /// Valve open durations (crank degrees).
    pub intake_duration: f32,
    pub exhaust_duration: f32,
    /// Angle between the intake and exhaust lobe centres (cam degrees).
    pub lobe_separation: f32,
    /// Valve lift (m).
    pub lift: f32,
}

/// Pistons and rods: their masses pull on the rods at the top of every exhaust stroke,
/// with the square of the rpm, and the gas pushes on them after every firing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BottomEnd {
    /// Piston with pin and rings, and connecting rod (kg).
    pub piston_mass: f32,
    pub rod_mass: f32,
    /// Pull that breaks a rod (N); pushing, it buckles at `BUCKLING` times that.
    pub rod_strength: f32,
    /// How much more knock the pistons take than stock ones.
    pub piston_toughness: f32,
}

/// Where the cams sit: in the block, working the valves through lifters, pushrods and
/// rockers (one cam for both rows of valves), or over the valves in the head (one cam or
/// one for the intakes and one for the exhausts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CamDrive {
    Ohv,
    Sohc,
    Dohc,
}

impl CamDrive {
    pub const ALL: [CamDrive; 3] = [CamDrive::Ohv, CamDrive::Sohc, CamDrive::Dohc];

    /// One cam turns both the intake and the exhaust lobes: its timing moves them together.
    pub fn one_cam(self) -> bool {
        self != CamDrive::Dohc
    }
}

/// The head's valves: where their cams are, how many per cylinder (2, 3 or 4: the odd one
/// an intake), and their diameters relative to the bore.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValveLayout {
    pub drive: CamDrive,
    pub valves: u8,
    pub intake_diameter: f32,
    pub exhaust_diameter: f32,
}

impl ValveLayout {
    /// The 2-litre four's: two cams over four valves.
    pub const FOUR_VALVE: Self = Self {
        drive: CamDrive::Dohc,
        valves: 4,
        intake_diameter: 0.36,
        exhaust_diameter: 0.3,
    };
    /// Pushrods to two big valves: about two thirds of the four valves' curtain area.
    pub const PUSHROD: Self = Self {
        drive: CamDrive::Ohv,
        valves: 2,
        intake_diameter: 0.48,
        exhaust_diameter: 0.39,
    };

    /// Intake and exhaust valves per cylinder.
    pub fn intakes(&self) -> u8 {
        self.valves.div_ceil(2)
    }

    pub fn exhausts(&self) -> u8 {
        self.valves / 2
    }

    /// It makes sense: 2 to 4 valves, of sizes that fit in a bore.
    pub fn fine(&self) -> bool {
        (2..=4).contains(&self.valves)
            && [self.intake_diameter, self.exhaust_diameter]
                .iter()
                .all(|d| (0.15..=0.6).contains(d))
    }
}

/// The valve springs and what they move. Above the speed where the spring can no longer
/// pull the valve after the cam's closing flank, the valve floats: it lofts off the cam
/// and bounces on its seat, and in an interference engine it can meet the piston.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValveTrain {
    /// Spring force with the valve closed (N), and its rise with lift (N/m).
    pub seat_force: f32,
    pub rate: f32,
    /// Moving mass per valve on the valve's side: valve, retainer, follower (or the
    /// rocker's share) and a third of the spring (kg).
    pub mass: f32,
    /// Valve lift over the lifter's (a pushrod engine's rockers; 1 with the cam over the
    /// valve), and what moves on the cam's side of the rocker: lifter and pushrod (kg;
    /// none with overhead cams).
    pub rocker_ratio: f32,
    pub cam_side_mass: f32,
    /// Roller followers or lifters, which drag about half what flat tappets do.
    pub rollers: bool,
}

impl ValveTrain {
    /// Spring force at full `lift` (N).
    pub fn force(&self, lift: f32) -> f32 {
        self.seat_force + self.rate * lift
    }

    /// Mass the spring moves as seen at the valve (kg): the cam side moves slower by the
    /// rocker ratio, so it counts with its square less.
    pub fn moving_mass(&self) -> f32 {
        self.mass + self.cam_side_mass / (self.rocker_ratio * self.rocker_ratio)
    }

    /// Engine speed (rpm) where the valves start to float on `cam`. The lift curve
    /// L·(1 - x²)^1.5 over the duration D decelerates the valve at most by 12·L·ω²/D² over
    /// the nose, which the spring's force at full lift has to provide.
    pub fn float_rpm(&self, cam: &CamProfile) -> f32 {
        let duration = cam.intake_duration.min(cam.exhaust_duration).to_radians();
        let omega = (self.force(cam.lift) * duration * duration
            / (12.0 * self.moving_mass() * cam.lift))
            .sqrt();
        omega * 60.0 / (2.0 * PI)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Intake {
    /// Effective flow area of the air filter (m²).
    pub filter_area: f32,
    pub throttle_diameter: f32,
    /// Flow area of the idle air valve, fully open (m²).
    pub idle_valve_area: f32,
    /// Intake runner length from plenum to valve (m).
    pub runner_length: f32,
    /// Plenum and runner volume behind the throttle (m³).
    pub plenum_volume: f32,
    /// Volume of the ducts ahead of the throttle (m³): after the air filter, or after the
    /// compressor (with the intercooler) on a turbo engine.
    pub tract_volume: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turbo {
    pub compressor: Compressor,
    pub turbine: Turbine,
    /// Inertia of the shaft with both wheels (kg·m²).
    pub inertia: f32,
    pub wastegate: Wastegate,
    pub intercooler: Intercooler,
    /// A valve venting the boost when the throttle shuts on it, so the compressor doesn't
    /// surge against it.
    pub diverter: Option<Diverter>,
}

impl Turbo {
    /// Fastest the shaft is made to spin (rad/s): the compressor wheel's tip speed limit.
    pub fn max_speed(&self) -> f32 {
        2.0 * TIP_SPEED_LIMIT / self.compressor.diameter
    }
}

/// A centrifugal compressor. Along each speed line its isentropic head falls with the
/// square of the flow: from `head` × tip speed² with no flow to nothing at the choke flow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Compressor {
    /// Wheel diameter at the tip (m).
    pub diameter: f32,
    /// Head coefficient at zero flow.
    pub head: f32,
    /// Flow coefficient (flow / (density · tip speed · diameter²)) where the head is gone.
    pub choke: f32,
    /// Best efficiency, reached at 60 % of the choke flow coefficient.
    pub efficiency: f32,
}

/// A radial turbine: a nozzle of effective `area`, its efficiency best at a blade speed
/// ratio of 0.65.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Turbine {
    /// Wheel diameter (m), effective flow area (m²) and best efficiency.
    pub diameter: f32,
    pub area: f32,
    pub efficiency: f32,
}

/// A valve letting exhaust past the turbine, pushed open by the boost against a spring.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wastegate {
    /// Effective flow area fully open (m²).
    pub area: f32,
    /// Boost (above ambient, Pa) where it starts to open, and how much more opens it fully.
    pub spring: f32,
    pub span: f32,
}

/// A supercharger: a Roots blower (positive displacement) or a centrifugal compressor,
/// driven by a belt from the crank.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Supercharger {
    /// A Roots blower; else centrifugal.
    pub roots: bool,
    /// A Roots blower's volume carried per revolution of its rotors (m³), and the
    /// effective area air leaks back through between them and the case (m²).
    pub displacement: f32,
    pub leak: f32,
    /// A centrifugal one's compressor.
    pub compressor: Compressor,
    /// The rotors' (or the impeller's) speed over the crank's.
    pub ratio: f32,
    /// What the belt and bearings drag at the crank (N·m), and more per 1000 rpm.
    pub drag: [f32; 2],
    /// The valve letting the boost back round when the throttle shuts on it.
    pub bypass: Diverter,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Diverter {
    /// Effective flow area fully open (m²).
    pub area: f32,
    /// Boost over manifold pressure (Pa) where it starts to open, and how much more opens
    /// it fully.
    pub opens: f32,
    pub span: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Intercooler {
    /// Share of the charge's heat above ambient it takes out.
    pub effectiveness: f32,
    /// Effective flow area (m²), for its pressure drop.
    pub area: f32,
}

/// What an exhaust manifold's primaries end in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collector {
    /// A collector into the pipe (tuned headers, a cast manifold).
    Tuned,
    /// A turbine's inlet.
    Turbine,
}

impl Collector {
    pub const ALL: [Collector; 2] = [Collector::Tuned, Collector::Turbine];
}

impl Manifold {
    /// Cylinders on each branch, of an engine of `cylinders`.
    pub fn per_branch(&self, cylinders: u32) -> f32 {
        cylinders as f32 / self.branches.max(1) as f32
    }
}

/// How an exhaust manifold gathers the cylinders' exhaust: primaries of a length (m) into
/// `branches` collectors, each taking as many cylinders (a V engine's pair of manifolds:
/// two, one per bank; a 10-into-1: one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Manifold {
    pub primary_length: f32,
    pub collector: Collector,
    pub branches: u32,
    /// How it's made, which decides how its pressure waves behave (see `cycle.rs`).
    pub made: Made,
}

/// How an exhaust manifold is made: what the waves leaving a cylinder meet at the end of
/// its primary, and what reaches it from the other cylinders' (`cycle.rs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Made {
    /// Tubular headers: smooth bends of even bore, each primary on its own until it opens
    /// into a big collector cone. Most of a wave comes back inverted, and the other
    /// cylinders' pulses pass on down the pipe.
    Tubes,
    /// Cast iron, the primaries short, rough stubs merging early into a log of small
    /// volume: less of a wave comes back, and every other cylinder's blowdown on the branch
    /// reaches this one's valve.
    Cast,
    /// Cast iron in pairs (a cast 4-2-1): the cylinders firing 360° apart share a runner
    /// before the pairs meet, so only the partner's pulse reaches the valve, when it's shut.
    CastPaired,
}

impl Made {
    pub const ALL: [Made; 3] = [Made::Tubes, Made::Cast, Made::CastPaired];
    /// The effective flow area of a cast manifold's merge, relative to the exhaust pipe's
    /// (a live restriction, in `Exhaust::resistance`): the abrupt junction of rough, narrow
    /// runners loses about a dynamic head of the gas's speed there, as much as the pipe
    /// loses over its length and bends; a 4-2-1's two Ys about half that. Tubular headers'
    /// smooth collector cone loses next to nothing.
    pub const CAST_FLOW: f32 = 1.0;
    pub const PAIRED_FLOW: f32 = 1.5;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Exhaust {
    /// The manifold: its primaries from valve to collector, what they end in, and how the
    /// cylinders are gathered.
    pub manifold: Manifold,
    /// Effective flow areas of the pipe and, if fitted, the catalyst and silencer (m²).
    pub pipe_area: f32,
    pub cat_area: Option<f32>,
    pub silencer_area: Option<f32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Cooling {
    /// Radiator frontal area (m²), and the share of the coolant-to-air temperature
    /// difference it can use.
    pub radiator_area: f32,
    pub radiator_effectiveness: f32,
    /// Coolant temperatures where the thermostat starts to open and is fully open (K).
    pub thermostat_opens: f32,
    pub thermostat_open: f32,
    /// Coolant temperatures where the fan switches on and off again (K), and the air speed
    /// it draws through the radiator (m/s).
    pub fan_on: f32,
    pub fan_off: f32,
    pub fan_speed: f32,
    /// Heat capacities of the coolant with the block, and of the oil with the sump (J/K).
    pub coolant_capacity: f32,
    pub oil_capacity: f32,
}

impl Default for Cooling {
    fn default() -> Self {
        Self {
            radiator_area: 0.3,
            radiator_effectiveness: 0.6,
            thermostat_opens: 361.0,
            thermostat_open: 371.0,
            fan_on: 373.0,
            fan_off: 368.0,
            fan_speed: 2.5,
            coolant_capacity: 55e3,
            oil_capacity: 15e3,
        }
    }
}

impl Exhaust {
    /// Flow resistance of the parts in series: the pressure drop is this times ṁ²/(2ρ).
    /// A cast manifold's merge is one of them, unless it feeds a turbine (`turbocharged`;
    /// the turbine's map takes it in).
    pub fn resistance(&self, turbocharged: bool) -> f32 {
        let merge = match self.manifold.made {
            _ if turbocharged && self.manifold.collector == Collector::Turbine => None,
            Made::Cast => Some(Made::CAST_FLOW),
            Made::CastPaired => Some(Made::PAIRED_FLOW),
            Made::Tubes => None,
        };
        let merge = merge.map(|share| share * self.pipe_area);
        [
            Some(self.pipe_area),
            merge,
            self.cat_area,
            self.silencer_area,
        ]
        .into_iter()
        .flatten()
        .map(|area| 1.0 / (area * area))
        .sum()
    }
}

impl EngineHardware {
    /// How many turbos it has, alike (at least one with a turbo, none without), as a
    /// factor for their flows.
    pub fn turbo_count(&self) -> f32 {
        if self.turbo.is_some() {
            self.turbos.max(1) as f32
        } else {
            0.0
        }
    }

    /// It has parts only an ECU with cam control can use: a second cam profile, or
    /// phasers.
    pub fn has_cam_control(&self) -> bool {
        self.cams.len() > 1 || self.intake_phasing > 0.0 || self.exhaust_phasing > 0.0
    }

    /// The valve spring force the cams work against per cylinder, as on four valves with
    /// flat tappets (N) at full `lift`: what the valvetrain's friction follows.
    pub fn spring_drag(&self, lift: f32) -> f32 {
        let followers = if self.valves.rollers { 0.5 } else { 1.0 };
        self.valves.force(lift) * (f32::from(self.valvetrain.valves) / 4.0) * followers
    }

    /// Highest speed the engine is made for on cam profile `profile` (rpm): clear of
    /// valve float, and with the rods' pull well within their strength; rounded down to
    /// 100 rpm.
    pub fn safe_rpm(&self, profile: usize) -> f32 {
        let cam = &self.cams[profile.min(self.cams.len() - 1)];
        let valves = FLOAT_MARGIN * self.valves.float_rpm(cam);
        let pull = self.rod_pull(1.0);
        let omega = (ROD_MARGIN * self.bottom_end.rod_strength / pull).sqrt();
        let rods = omega * 60.0 / (2.0 * PI);
        (valves.min(rods) / 100.0).floor() * 100.0
    }

    /// Mass moving up and down with each piston: the piston and about a third of the rod
    /// (kg).
    fn reciprocating_mass(&self) -> f32 {
        self.bottom_end.piston_mass + self.bottom_end.rod_mass / 3.0
    }

    /// Inertia force on a rod at top dead centre at crank speed `omega` (rad/s; N),
    /// m·r·ω²·(1 + r/l).
    pub fn rod_pull(&self, omega: f32) -> f32 {
        let r = self.stroke / 2.0;
        self.reciprocating_mass() * r * omega * omega * (1.0 + r / self.rod_length)
    }

    /// Load on the rods relative to what breaks them (1 breaks them at once): the larger
    /// of the pull at top dead centre of the gas exchange, and the gas's push at the
    /// `peak` cylinder pressure (Pa) less the inertia there, against buckling.
    pub fn rod_load(&self, rpm: f32, peak: f32) -> f32 {
        let omega = rpm * 2.0 * PI / 60.0;
        let pull = self.rod_pull(omega);
        let r = self.stroke / 2.0;
        let lambda = r / self.rod_length;
        let at_peak = self.reciprocating_mass()
            * r
            * omega
            * omega
            * (PEAK_ANGLE.cos() + lambda * (2.0 * PEAK_ANGLE).cos());
        let area = PI / 4.0 * self.bore * self.bore;
        let push = peak * area - at_peak;
        let strength = self.bottom_end.rod_strength;
        (pull / strength).max(push / (BUCKLING * strength))
    }

    /// The redline: the safe speed on the last cam profile (the one for high rpm), which the
    /// rev counter shows and the factory tune limits the revs to.
    pub fn redline(&self) -> f32 {
        self.safe_rpm(self.cams.len() - 1)
    }

    /// Swept volume of all cylinders (m³).
    pub fn displacement(&self) -> f32 {
        PI / 4.0 * self.bore * self.bore * self.stroke * self.cylinders as f32
    }

    /// The 2-litre four with lower compression and a mid-sized turbo, wastegated at about
    /// 0.7 bar of boost, with an intercooler and a diverter valve.
    #[cfg(test)]
    pub fn turbo() -> Self {
        super::parts::Parts::turbo().hardware()
    }
}

impl Default for EngineHardware {
    /// A 2-litre petrol four with fixed cams and a stock exhaust: the W20, which the other
    /// engines are scaled from.
    fn default() -> Self {
        Self {
            name: "W20 2.0 four".to_string(),
            cylinders: 4,
            bore: 0.086,
            stroke: 0.086,
            rod_length: 0.143,
            compression_ratio: 10.5,
            bottom_end: BottomEnd {
                piston_mass: 0.42,
                rod_mass: 0.6,
                rod_strength: 30e3,
                piston_toughness: 1.0,
            },
            head_gasket: 130e5,
            cams: vec![CamProfile {
                intake_duration: 240.0,
                exhaust_duration: 240.0,
                lobe_separation: 110.0,
                lift: 0.0095,
            }],
            intake_phasing: 0.0,
            exhaust_phasing: 0.0,
            intake_timing: 0.0,
            exhaust_timing: 0.0,
            valves: ValveTrain {
                seat_force: 180.0,
                rate: 20e3,
                mass: 0.1,
                rocker_ratio: 1.0,
                cam_side_mass: 0.0,
                rollers: false,
            },
            valvetrain: ValveLayout::FOUR_VALVE,
            port_flow: 1.0,
            intake: Intake {
                filter_area: 2.5e-3,
                throttle_diameter: 0.055,
                idle_valve_area: 1.2e-4,
                runner_length: 0.35,
                plenum_volume: 3.0e-3,
                tract_volume: 2.0e-3,
            },
            turbo: None,
            turbos: 0,
            supercharger: None,
            exhaust: Exhaust {
                manifold: Manifold {
                    primary_length: 0.8,
                    collector: Collector::Tuned,
                    branches: 1,
                    made: Made::Tubes,
                },
                pipe_area: 1.9e-3,
                cat_area: Some(2.0e-3),
                silencer_area: Some(1.6e-3),
            },
            wideband: false,
            cooling: Cooling::default(),
            inertia: 0.18,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pushrods_float_the_valves_earlier() {
        let ohc = EngineHardware::default();
        let cam = &ohc.cams[0];
        // The same valve and spring, worked through a lifter, a pushrod and a rocker.
        let pushrod = ValveTrain {
            rocker_ratio: 1.6,
            cam_side_mass: 0.15,
            ..ohc.valves
        };
        let (overhead, block) = (ohc.valves.float_rpm(cam), pushrod.float_rpm(cam));
        assert!(block < 0.85 * overhead, "{block} vs {overhead} rpm");
        // Without a cam side the mass is the valve side's.
        assert_eq!(ohc.valves.moving_mass(), ohc.valves.mass);
    }

    #[test]
    fn two_valves_and_rollers_drag_less() {
        let dohc = EngineHardware::default();
        let lift = dohc.cams[0].lift;
        assert_eq!(dohc.spring_drag(lift), dohc.valves.force(lift));
        let mut ohv = dohc.clone();
        ohv.valvetrain = ValveLayout::PUSHROD;
        assert!(ohv.spring_drag(lift) < dohc.spring_drag(lift));
        ohv.valves.rollers = true;
        assert_eq!(ohv.spring_drag(lift), 0.25 * dohc.spring_drag(lift));
    }
}
