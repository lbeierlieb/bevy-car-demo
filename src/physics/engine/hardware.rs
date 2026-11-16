//! The engine's parts, as a garage would change them.

use std::f32::consts::PI;

/// Everything about the engine that is hardware. The cylinder side (bottom end, head,
/// cams, runners, headers) shapes the baked maps, so changing it means baking them again;
/// the parts between the manifolds and the outside (filter, throttle, idle valve, exhaust
/// restrictions) act live.
#[derive(Clone, Debug)]
pub struct EngineHardware {
    pub name: &'static str,
    pub cylinders: u32,
    /// Bore and stroke (m).
    pub bore: f32,
    pub stroke: f32,
    /// Connecting rod length, centre to centre (m).
    pub rod_length: f32,
    pub compression_ratio: f32,
    /// Cam profiles the valvetrain can switch between (like VTEC); one for fixed cams.
    pub cams: Vec<CamProfile>,
    /// How far the cam phasers can advance the intake cam and retard the exhaust cam
    /// (crank degrees; zero without phasers).
    pub intake_phasing: f32,
    pub exhaust_phasing: f32,
    pub intake: Intake,
    /// A turbocharger between the air filter and the throttle, and in the exhaust.
    pub turbo: Option<Turbo>,
    pub exhaust: Exhaust,
    pub cooling: Cooling,
    /// Flywheel and crank inertia (kg·m²).
    pub inertia: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct CamProfile {
    /// Valve open durations (crank degrees).
    pub intake_duration: f32,
    pub exhaust_duration: f32,
    /// Angle between the intake and exhaust lobe centres (cam degrees).
    pub lobe_separation: f32,
    /// Valve lift (m).
    pub lift: f32,
}

impl CamProfile {
    /// Crank degrees both valves are open around top dead centre, with the cams phased
    /// (intake advanced, exhaust retarded, crank degrees).
    pub fn overlap(&self, intake_phase: f32, exhaust_phase: f32) -> f32 {
        let overlap = (self.intake_duration + self.exhaust_duration) / 2.0
            - 2.0 * self.lobe_separation
            + intake_phase
            + exhaust_phase;
        overlap.max(0.0)
    }
}

#[derive(Clone, Copy, Debug)]
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

#[derive(Clone, Copy, Debug)]
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

/// A centrifugal compressor. Along each speed line its isentropic head falls with the
/// square of the flow: from `head` × tip speed² with no flow to nothing at the choke flow.
#[derive(Clone, Copy, Debug)]
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
#[derive(Clone, Copy, Debug)]
pub struct Turbine {
    /// Wheel diameter (m), effective flow area (m²) and best efficiency.
    pub diameter: f32,
    pub area: f32,
    pub efficiency: f32,
}

/// A valve letting exhaust past the turbine, pushed open by the boost against a spring.
#[derive(Clone, Copy, Debug)]
pub struct Wastegate {
    /// Effective flow area fully open (m²).
    pub area: f32,
    /// Boost (above ambient, Pa) where it starts to open, and how much more opens it fully.
    pub spring: f32,
    pub span: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Diverter {
    /// Effective flow area fully open (m²).
    pub area: f32,
    /// Boost over manifold pressure (Pa) where it starts to open, and how much more opens
    /// it fully.
    pub opens: f32,
    pub span: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Intercooler {
    /// Share of the charge's heat above ambient it takes out.
    pub effectiveness: f32,
    /// Effective flow area (m²), for its pressure drop.
    pub area: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Exhaust {
    /// Header primary length from valve to collector (m).
    pub header_length: f32,
    /// Effective flow areas of the pipe and, if fitted, the catalyst and silencer (m²).
    pub pipe_area: f32,
    pub cat_area: Option<f32>,
    pub silencer_area: Option<f32>,
}

#[derive(Clone, Copy, Debug)]
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
    pub fn resistance(&self) -> f32 {
        [Some(self.pipe_area), self.cat_area, self.silencer_area]
            .into_iter()
            .flatten()
            .map(|area| 1.0 / (area * area))
            .sum()
    }
}

impl EngineHardware {
    /// Swept volume of all cylinders (m³).
    pub fn displacement(&self) -> f32 {
        PI / 4.0 * self.bore * self.bore * self.stroke * self.cylinders as f32
    }

    /// The 2-litre four with lower compression and a mid-sized turbo, wastegated at about
    /// 0.7 bar of boost, with an intercooler and a diverter valve.
    pub fn turbo() -> Self {
        let stock = Self::default();
        Self {
            name: "2.0 turbo",
            compression_ratio: 8.8,
            intake: Intake {
                tract_volume: 6.0e-3,
                ..stock.intake
            },
            turbo: Some(Turbo {
                compressor: Compressor {
                    diameter: 0.05,
                    head: 0.7,
                    choke: 0.29,
                    efficiency: 0.75,
                },
                turbine: Turbine {
                    diameter: 0.054,
                    area: 6e-4,
                    efficiency: 0.7,
                },
                inertia: 2.5e-5,
                wastegate: Wastegate {
                    area: 3e-4,
                    spring: 0.6e5,
                    span: 0.12e5,
                },
                intercooler: Intercooler {
                    effectiveness: 0.7,
                    area: 3e-3,
                },
                diverter: Some(Diverter {
                    area: 5e-4,
                    opens: 0.3e5,
                    span: 0.2e5,
                }),
            }),
            ..stock
        }
    }
}

impl Default for EngineHardware {
    /// A 2-litre petrol four with fixed cams and a stock exhaust.
    fn default() -> Self {
        Self {
            name: "2.0 naturally aspirated",
            cylinders: 4,
            bore: 0.086,
            stroke: 0.086,
            rod_length: 0.143,
            compression_ratio: 10.5,
            cams: vec![CamProfile {
                intake_duration: 240.0,
                exhaust_duration: 240.0,
                lobe_separation: 110.0,
                lift: 0.0095,
            }],
            intake_phasing: 0.0,
            exhaust_phasing: 0.0,
            intake: Intake {
                filter_area: 2.5e-3,
                throttle_diameter: 0.055,
                idle_valve_area: 1.2e-4,
                runner_length: 0.35,
                plenum_volume: 3.0e-3,
                tract_volume: 2.0e-3,
            },
            turbo: None,
            exhaust: Exhaust {
                header_length: 0.8,
                pipe_area: 1.9e-3,
                cat_area: Some(2.0e-3),
                silencer_area: Some(1.6e-3),
            },
            cooling: Cooling::default(),
            inertia: 0.18,
        }
    }
}
