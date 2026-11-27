//! A cycle simulation of one cylinder, crank degree by crank degree, for baking the maps.
//!
//! The cylinder is one zone of ideal gas (its heat capacity rising with temperature) with
//! fresh mixture and burnt gas mixed. The valves are orifices with the cams' lift curves,
//! burning follows a Wiebe curve, heat goes into the walls by Woschni's correlation, and
//! the end gas knocks when its ignition delay (Douaud and Eyzat) runs out before the flame
//! reaches it. Intake runner and header primary are acoustic pipes: waves leave the valve,
//! reflect inverted off the plenum or the collector, and come back after twice the pipe's
//! length at the speed of sound. That is what tunes them: a wave arriving at the right
//! moment packs more charge in before the intake valve closes, or pulls exhaust out
//! during the overlap. Gas pushed back into the intake mixes into the runner, which
//! fills from the plenum with fresh air as the cylinder draws on it. Each cylinder sees a plenum and a collector at constant pressure; the other
//! cylinders' pulses are left out.

use std::f64::consts::PI;

use super::hardware::{CamProfile, EngineHardware};

const R: f64 = 287.0;
const STOICHIOMETRIC: f64 = 14.7;
const FUEL_ENERGY: f64 = 44e6;
/// Crank degrees per step.
const STEP: f64 = 1.0;
/// Steps per cycle.
const STEPS: usize = (720.0 / STEP) as usize;
/// Specific heat at constant volume (J/(kg·K)): at 300 K, and its rise per kelvin.
const CV: f64 = 718.0;
const CV_RISE: f64 = 0.13;
/// Share of the fuel that burns (the rest leaves unburnt).
const COMBUSTION_EFFICIENCY: f64 = 0.97;
/// Wiebe curve: its efficiency and form factors, and the burn duration (crank degrees, to
/// 99.9 % burnt) at 3000 rpm, ambient density and 5 % residual gas in an 86 mm bore.
const WIEBE_A: f64 = 6.9;
const WIEBE_M: f64 = 2.0;
const BURN_DURATION: f64 = 66.0;
/// Burn fraction where the flame has consumed the end gas: knock after it does no harm.
const END_GAS: f64 = 0.95;
/// Douaud and Eyzat's ignition delay: τ = A·(ON/100)^3.402·(p/atm)^-1.7·exp(B/T), A in s.
/// Their A is 17.68 ms; with the end gas as warm as this model makes it, a larger one puts
/// the knock limit of a 10.5:1 engine on 95 octane a few degrees short of MBT at low rpm
/// and full load, and past it at high rpm.
const KNOCK_A: f64 = 50e-3;
const KNOCK_B: f64 = 3800.0;
/// Ratio of specific heats of the unburnt mixture, for the end gas's compression.
const GAMMA_UNBURNT: f64 = 1.35;
/// Share of the waves' swing (around their mean) that comes back from the pipes' open ends.
const INTAKE_REFLECTION: f64 = 0.85;
const HEADER_REFLECTION: f64 = 0.8;
/// Share of the exhaust's heat above the wall temperature lost in the exhaust port.
const PORT_LOSS: f64 = 0.1;
/// Share of the way from the intake air's temperature to the wall's that the fresh charge
/// warms up in the intake port at 2000 rpm (it warms less at higher rpm, having less time).
const PORT_HEATING: f64 = 0.12;
/// Wall temperature above the coolant (K).
const WALL_ABOVE_COOLANT: f64 = 60.0;
/// The gas exchange counts as converged when the trapped charge changes less than this.
const CONVERGED: f64 = 1e-3;
const MAX_CYCLES: usize = 10;
/// The lowest speed simulated (rpm); slower points copy it.
const MIN_RPM: f64 = 400.0;

/// The engine as the simulation sees one cylinder of it, with one cam profile and phasing.
#[derive(Clone, Debug)]
pub struct Cylinder {
    bore: f64,
    crank: f64,
    rod: f64,
    piston_area: f64,
    clearance: f64,
    swept: f64,
    intake: Valve,
    exhaust: Valve,
    runner: Pipe,
    header: Pipe,
}

#[derive(Clone, Copy, Debug)]
struct Valve {
    /// Crank angle of full lift (degrees; 0 is top dead centre of combustion).
    centre: f64,
    half_duration: f64,
    lift: f64,
    /// Diameter and number of valves.
    diameter: f64,
    count: f64,
}

#[derive(Clone, Copy, Debug)]
struct Pipe {
    length: f64,
    area: f64,
}

impl Valve {
    /// Effective flow area (m²) at crank angle `angle`: the curtain area while the lift is
    /// small, up to what the port passes.
    fn area(&self, angle: f64) -> f64 {
        let x = wrap(angle - self.centre) / self.half_duration;
        if x.abs() >= 1.0 {
            return 0.0;
        }
        let lift = self.lift * (1.0 - x * x).powf(1.5);
        let curtain = PI * self.diameter * lift;
        let saturation = lift / (0.225 * self.diameter);
        self.count * 0.7 * curtain / (1.0 + saturation * saturation).sqrt()
    }

    fn opens(&self) -> f64 {
        self.centre - self.half_duration
    }

    fn closes(&self) -> f64 {
        self.centre + self.half_duration
    }
}

/// An angle wrapped into -360..360 degrees.
fn wrap(angle: f64) -> f64 {
    (angle + 360.0).rem_euclid(720.0) - 360.0
}

impl Cylinder {
    /// One cylinder of `hardware` on cam profile `cam`, the phasers advancing the intake cam
    /// and retarding the exhaust cam by the given phases (crank degrees), on top of the
    /// hardware's cam timing. Valves (as many and as big as the head's), runner and header
    /// diameters scale with the bore, and their flow areas with the head's port flow.
    pub fn new(hardware: &EngineHardware, cam: &CamProfile, intake: f32, exhaust: f32) -> Self {
        let bore = f64::from(hardware.bore);
        let stroke = f64::from(hardware.stroke);
        let piston_area = PI / 4.0 * bore * bore;
        let swept = piston_area * stroke;
        let cr = f64::from(hardware.compression_ratio);
        let separation = f64::from(cam.lobe_separation);
        let lift = f64::from(cam.lift);
        let flow = f64::from(hardware.port_flow);
        let layout = hardware.valvetrain;
        // Valve sizes to a ten-thousandth of the bore (so 0.36 is 0.36 here, not the f32's
        // nearest).
        let diameter = |share: f32| (f64::from(share) * 1e4).round() / 1e4 * bore;
        let pipe = |length: f32, diameter: f64| Pipe {
            length: f64::from(length),
            area: flow * PI / 4.0 * diameter * diameter,
        };
        Self {
            bore,
            crank: stroke / 2.0,
            rod: f64::from(hardware.rod_length),
            piston_area,
            clearance: swept / (cr - 1.0),
            swept,
            intake: Valve {
                centre: -360.0 + separation - f64::from(intake + hardware.intake_timing),
                half_duration: f64::from(cam.intake_duration) / 2.0,
                lift,
                diameter: diameter(layout.intake_diameter),
                count: f64::from(layout.intakes()) * flow,
            },
            exhaust: Valve {
                centre: 360.0 - separation + f64::from(exhaust + hardware.exhaust_timing),
                half_duration: f64::from(cam.exhaust_duration) / 2.0,
                lift,
                diameter: diameter(layout.exhaust_diameter),
                count: f64::from(layout.exhausts()) * flow,
            },
            runner: pipe(hardware.intake.runner_length, 0.45 * bore),
            header: pipe(hardware.exhaust.header_length, 0.42 * bore),
        }
    }

    /// Cylinder volume (m³) at crank angle `angle` (degrees).
    fn volume(&self, angle: f64) -> f64 {
        let (sin, cos) = angle.to_radians().sin_cos();
        let (a, l) = (self.crank, self.rod);
        let travel = a + l - a * cos - (l * l - a * a * sin * sin).sqrt();
        self.clearance + self.piston_area * travel
    }

    /// Area of the walls (m²) with the cylinder at `volume`: head and piston crown a little
    /// bigger than the bore, and the liner above the piston.
    fn wall_area(&self, volume: f64) -> f64 {
        2.2 * self.piston_area + PI * self.bore * volume / self.piston_area
    }

    /// Crank angle where the intake valve closes and where the exhaust valve opens, both
    /// within the cycle that starts at the former.
    fn closed_phase(&self) -> (f64, f64) {
        let start = wrap(self.intake.closes());
        let end = start + (self.exhaust.opens() - start).rem_euclid(720.0);
        (start, end)
    }
}

/// The operating conditions around the cylinder.
#[derive(Clone, Copy, Debug)]
pub struct Conditions {
    pub rpm: f64,
    /// Pressures in the intake plenum and the exhaust collector (Pa).
    pub intake_pressure: f64,
    pub exhaust_pressure: f64,
    /// Temperatures of the air in the intake and of the coolant (K).
    pub intake_temperature: f64,
    pub coolant_temperature: f64,
    /// Research octane number of the fuel.
    pub octane: f64,
}

impl Conditions {
    fn rpm(&self) -> f64 {
        self.rpm.max(MIN_RPM)
    }

    fn step_time(&self) -> f64 {
        STEP / (6.0 * self.rpm())
    }

    fn wall_temperature(&self) -> f64 {
        self.coolant_temperature + WALL_ABOVE_COOLANT
    }

    /// Temperature of the fresh charge after the intake port has warmed it.
    fn fresh_temperature(&self) -> f64 {
        let share = (PORT_HEATING * (2000.0 / self.rpm()).sqrt()).min(0.3);
        self.intake_temperature + share * (self.wall_temperature() - self.intake_temperature)
    }

    fn piston_speed(&self, cylinder: &Cylinder) -> f64 {
        4.0 * cylinder.crank * self.rpm() / 60.0
    }
}

/// The cylinder's contents.
#[derive(Clone, Copy, Debug)]
pub struct Charge {
    /// Total mass and the burnt gas in it (kg), and the internal energy (J).
    mass: f64,
    burnt: f64,
    energy: f64,
}

impl Charge {
    fn temperature(&self) -> f64 {
        temperature(self.energy / self.mass)
    }

    fn pressure(&self, volume: f64) -> f64 {
        self.mass * R * self.temperature() / volume
    }

    /// Fresh mixture (air and fuel) and the air and fuel in it (kg).
    fn fresh(&self) -> f64 {
        self.mass - self.burnt
    }

    fn air(&self) -> f64 {
        self.fresh() * STOICHIOMETRIC / (1.0 + STOICHIOMETRIC)
    }

    fn fuel(&self) -> f64 {
        self.fresh() / (1.0 + STOICHIOMETRIC)
    }
}

/// Internal energy per kg (J/kg, zero at 300 K) at `temperature`.
fn energy(temperature: f64) -> f64 {
    let t = temperature - 300.0;
    CV * t + CV_RISE / 2.0 * t * t
}

fn temperature(energy: f64) -> f64 {
    300.0 + (2.0 * energy / (CV + (CV * CV + 2.0 * CV_RISE * energy).sqrt()))
}

fn enthalpy(temperature: f64) -> f64 {
    energy(temperature) + R * temperature
}

fn gamma(temperature: f64) -> f64 {
    1.0 + R / (CV + CV_RISE * (temperature - 300.0))
}

/// Woschni's heat transfer coefficient (W/(m²·K)) for gas at `pressure` (Pa) and
/// `temperature`, with the gas moving at `speed` (m/s).
fn woschni(bore: f64, pressure: f64, temperature: f64, speed: f64) -> f64 {
    3.26 * bore.powf(-0.2) * (pressure / 1e3).powf(0.8) * temperature.powf(-0.55) * speed.powf(0.8)
}

/// Compressible flow (kg/s) through an orifice of effective `area` from `upstream` to
/// `downstream`, with gas at `temperature` upstream; linear close to equal pressures.
fn orifice(area: f64, upstream: f64, downstream: f64, temperature: f64) -> f64 {
    const LINEAR: f64 = 0.99;
    let psi = |ratio: f64| {
        let ratio = ratio.max(0.528);
        (7.0 * (ratio.powf(2.0 / 1.4) - ratio.powf(2.4 / 1.4))).sqrt()
    };
    let ratio = downstream / upstream;
    let psi = if ratio > LINEAR {
        psi(LINEAR) * (1.0 - ratio) / (1.0 - LINEAR)
    } else {
        psi(ratio)
    };
    area * upstream / (R * temperature).sqrt() * psi
}

/// Flow (kg/s) from side `a` to side `b` of an orifice, negative when it goes back.
fn flow(area: f64, a: f64, b: f64, ta: f64, tb: f64) -> f64 {
    if a >= b {
        orifice(area, a, b, ta)
    } else {
        -orifice(area, b, a, tb)
    }
}

/// A pipe's waves at the valve end: the pressure waves sent into it, in a ring buffer one
/// step apart, so the reflections can be read back after the pipe's echo delay.
#[derive(Clone, Debug)]
struct Waves {
    sent: Vec<f64>,
    head: usize,
    /// Running mean of the waves sent.
    mean: f64,
}

impl Waves {
    fn new() -> Self {
        Self {
            sent: vec![0.0; 4096],
            head: 0,
            mean: 0.0,
        }
    }

    /// The wave arriving now: what was sent `delay` steps ago (at least one), reflected
    /// inverted with its swing around the mean damped by `reflection`.
    fn arriving(&self, delay: f64, reflection: f64) -> f64 {
        let delay = delay.clamp(1.0, (self.sent.len() - 2) as f64);
        let back = delay.floor() as usize;
        let t = delay - back as f64;
        let at = |back: usize| self.sent[(self.head + self.sent.len() - back) % self.sent.len()];
        let sent = at(back) * (1.0 - t) + at(back + 1) * t;
        -(self.mean + reflection * (sent - self.mean))
    }

    fn send(&mut self, wave: f64) {
        self.head = (self.head + 1) % self.sent.len();
        self.sent[self.head] = wave;
        self.mean += (wave - self.mean) * (STEP / 720.0);
    }
}

/// One valve and its pipe, as the flow into the cylinder through them sees it.
struct Port {
    area: f64,
    /// Pressure at the valve if no gas flowed (the pipe's mean plus twice the arriving
    /// wave), and how much flow into the cylinder lowers it (Pa per kg/s).
    pressure: f64,
    impedance: f64,
    /// Temperature of the gas on the pipe's side and in the cylinder.
    outside: f64,
    inside: f64,
}

impl Port {
    /// Flow into the cylinder (kg/s) with the cylinder at `cylinder` (Pa).
    fn solve(&self, cylinder: f64) -> f64 {
        if self.area == 0.0 {
            return 0.0;
        }
        // The flow m satisfies m = valve(pressure − impedance·m): between no flow and the
        // flow that would drop the valve's pipe side to the cylinder's pressure.
        let through = |m: f64| {
            m - flow(
                self.area,
                self.pressure - self.impedance * m,
                cylinder,
                self.outside,
                self.inside,
            )
        };
        let limit = (self.pressure - cylinder) / self.impedance;
        let (mut a, mut b) = (0.0, limit);
        let (mut fa, mut fb) = (through(a), limit);
        if fa == 0.0 {
            return 0.0;
        }
        // Illinois method: regula falsi that halves the stale end's weight.
        let mut side = 0;
        for _ in 0..40 {
            let m = (a * fb - b * fa) / (fb - fa);
            let fm = through(m);
            if fm.abs() < 1e-9 || (b - a).abs() < 1e-9 * limit.abs().max(1e-6) {
                return m;
            }
            if fm.signum() == fb.signum() {
                (b, fb) = (m, fm);
                if side == -1 {
                    fa /= 2.0;
                }
                side = -1;
            } else {
                (a, fa) = (m, fm);
                if side == 1 {
                    fb /= 2.0;
                }
                side = 1;
            }
        }
        (a + b) / 2.0
    }

    /// How the flow into the cylinder changes with the cylinder's pressure (kg/(s·Pa)), at
    /// the flow `m`.
    fn slope(&self, m: f64, cylinder: f64) -> f64 {
        if self.area == 0.0 {
            return 0.0;
        }
        let at = self.pressure - self.impedance * m;
        let valve = |a, b| flow(self.area, a, b, self.outside, self.inside);
        let step = 1e-4 * cylinder;
        let base = valve(at, cylinder);
        let by_cylinder = (valve(at, cylinder + step) - base) / step;
        let by_pipe = (valve(at + step, cylinder) - base) / step;
        by_cylinder / (1.0 + self.impedance * by_pipe)
    }
}

/// What the closed part of the cycle (from intake closing to exhaust opening) does.
#[derive(Clone, Copy, Debug)]
pub struct Burn {
    /// Work on the piston and heat into the walls (J).
    pub work: f64,
    pub heat: f64,
    /// The end gas's knock integral when the flame reaches it (knock at 1).
    pub knock: f64,
    /// Highest cylinder pressure (Pa).
    pub peak: f64,
    end: Charge,
}

/// One whole cycle, the gas exchange converged.
#[derive(Clone, Debug)]
pub struct CycleResult {
    /// The charge trapped when the intake valve closes.
    pub trapped: Charge,
    /// Air trapped (kg).
    pub air: f64,
    /// Work over the whole cycle and over its closed part (J).
    pub work: f64,
    pub closed_work: f64,
    /// Heat into the walls, the cylinder's and the exhaust port's (J).
    pub heat: f64,
    /// Mass-averaged temperature of the exhaust after the port (K).
    pub exhaust_temperature: f64,
    /// Fuel energy of the trapped charge (J).
    pub fuel_energy: f64,
}

impl CycleResult {
    /// Air trapped relative to filling the cylinder at the intake's density.
    pub fn volumetric_efficiency(&self, cylinder: &Cylinder, conditions: &Conditions) -> f64 {
        let density = conditions.intake_pressure / (R * conditions.intake_temperature);
        self.air / (density * cylinder.swept)
    }
}

impl Cylinder {
    /// Wiebe burn duration (crank degrees) for the charge trapped in `charge`: longer at
    /// high rpm (in degrees), with more residual gas, thinner charge and bigger bores.
    fn burn_duration(&self, charge: &Charge, conditions: &Conditions) -> f64 {
        let (start, _) = self.closed_phase();
        let density = charge.mass / self.volume(start);
        let residual = charge.burnt / charge.mass;
        let duration = BURN_DURATION
            * (conditions.rpm() / 3000.0).powf(0.2)
            * (density / 1.18).powf(-0.2)
            * (1.0 + 2.5 * (residual - 0.05).max(-0.03))
            * (self.bore / 0.086).sqrt();
        duration.min(150.0)
    }

    /// Spark advance that puts the middle of the burn 8° after top dead centre, about
    /// where it gives the most work.
    fn spark_estimate(&self, charge: &Charge, conditions: &Conditions) -> f64 {
        let half = (2f64.ln() / WIEBE_A).powf(1.0 / (WIEBE_M + 1.0));
        half * self.burn_duration(charge, conditions) - 8.0
    }

    /// The closed part of the cycle from the charge trapped at intake closing, with the
    /// spark `spark` degrees before top dead centre.
    pub fn burn(&self, trapped: &Charge, conditions: &Conditions, spark: f64) -> Burn {
        let (start, end) = self.closed_phase();
        let dt = conditions.step_time();
        let duration = self.burn_duration(trapped, conditions);
        let released = trapped.fuel() * FUEL_ENERGY * COMBUSTION_EFFICIENCY;
        let fresh = trapped.fresh();
        let wall = conditions.wall_temperature();
        let speed = conditions.piston_speed(self);
        let (v0, p0, t0) = {
            let v = self.volume(start);
            (v, trapped.pressure(v), trapped.temperature())
        };
        // Ignition delay of the end gas at 1 atm and 1 K, without the temperature term.
        let delay = KNOCK_A * (conditions.octane / 100.0).powf(3.402);
        let wiebe = |angle: f64| {
            let y = ((angle + spark) / duration).max(0.0);
            1.0 - (-WIEBE_A * y.powf(WIEBE_M + 1.0)).exp()
        };

        let mut charge = *trapped;
        let (mut work, mut heat, mut knock, mut peak) = (0.0, 0.0, 0.0, 0.0f64);
        let mut angle = start;
        let mut burnt = 0.0;
        while angle < end {
            let next = angle + STEP;
            let (v, v1) = (self.volume(angle), self.volume(next));
            let t = charge.temperature();
            let p = charge.mass * R * t / v;
            peak = peak.max(p);
            let burnt1 = wiebe(next);
            // Woschni: the gas moves with the piston, and faster once combustion pushes
            // the pressure above motoring.
            let motored = p0 * (v0 / v).powf(1.33);
            let gas_speed =
                2.28 * speed + 3.24e-3 * self.swept * t0 / (p0 * v0) * (p - motored).max(0.0);
            let loss = woschni(self.bore, p, t, gas_speed) * self.wall_area(v) * (t - wall) * dt;
            let release = released * (burnt1 - burnt);
            // Heun: the work with the pressure at the start, then with the mean.
            let guess = Charge {
                energy: charge.energy + release - loss - p * (v1 - v),
                ..charge
            };
            let p1 = guess.pressure(v1);
            let dw = (p + p1) / 2.0 * (v1 - v);
            charge.energy += release - loss - dw;
            charge.burnt += fresh * (burnt1 - burnt);
            work += dw;
            heat += loss;
            if burnt < END_GAS {
                let unburnt = t0 * (p / p0).powf((GAMMA_UNBURNT - 1.0) / GAMMA_UNBURNT);
                let tau = delay * (p / 101_325.0).powf(-1.7) * (KNOCK_B / unburnt).exp();
                knock += dt / tau;
            }
            burnt = burnt1;
            angle = next;
        }
        Burn {
            work,
            heat,
            knock,
            peak,
            end: charge,
        }
    }

    /// Spark advance (degrees before top dead centre) for the most work from `trapped`.
    pub fn mbt(&self, trapped: &Charge, conditions: &Conditions) -> f64 {
        // Golden section search over the work.
        let work = |spark| self.burn(trapped, conditions, spark).work;
        let ratio = (5f64.sqrt() - 1.0) / 2.0;
        let (mut a, mut b) = (-10.0, 60.0);
        let (mut c, mut d) = (b - ratio * (b - a), a + ratio * (b - a));
        let (mut wc, mut wd) = (work(c), work(d));
        for _ in 0..16 {
            if wc > wd {
                (b, d, wd) = (d, c, wc);
                c = b - ratio * (b - a);
                wc = work(c);
            } else {
                (a, c, wc) = (c, d, wd);
                d = a + ratio * (b - a);
                wd = work(d);
            }
        }
        (a + b) / 2.0
    }

    /// Spark advance (degrees before top dead centre) where `trapped` starts to knock. It
    /// searches up from retarded: far advanced, the burn can finish before the end gas
    /// gets hot, which isn't a way out of knock.
    pub fn knock_limit(&self, trapped: &Charge, conditions: &Conditions) -> f64 {
        let knocks = |spark| self.burn(trapped, conditions, spark).knock >= 1.0;
        let (lowest, highest, step) = (-20.0, 60.0, 4.0);
        if knocks(lowest) {
            return lowest;
        }
        let mut a = lowest;
        while a < highest {
            let mut b = a + step;
            if knocks(b) {
                for _ in 0..10 {
                    let mid = (a + b) / 2.0;
                    if knocks(mid) {
                        b = mid;
                    } else {
                        a = mid;
                    }
                }
                return (a + b) / 2.0;
            }
            a = b;
        }
        highest
    }

    /// Runs whole cycles until the gas exchange settles, with the spark `spark` degrees
    /// before top dead centre, or where it gives about the most work.
    pub fn run(&self, conditions: &Conditions, spark: Option<f64>) -> CycleResult {
        let dt = conditions.step_time();
        let (start, end) = self.closed_phase();
        let t_in = conditions.fresh_temperature();
        let wall = conditions.wall_temperature();
        let speed = conditions.piston_speed(self);
        let (p_in, p_ex) = (conditions.intake_pressure, conditions.exhaust_pressure);
        let sound = |gamma: f64, t: f64| (gamma * R * t).sqrt();

        // A guess at the trapped charge: the cylinder filled at intake pressure.
        let v_start = self.volume(start);
        let mass = p_in * v_start / (R * 330.0);
        let mut trapped = Charge {
            mass,
            burnt: 0.05 * mass,
            energy: mass * energy(330.0),
        };
        let mut intake = Waves::new();
        let mut header = Waves::new();
        // The gas in the intake runner, mixed: its mass, burnt share and temperature.
        let runner_mass =
            p_in * self.runner.area * self.runner.length / (R * conditions.intake_temperature);
        let (mut runner_burnt, mut runner_t) = (0.0, t_in);
        let mut header_t = 1000.0;
        let mut last = None;
        for _ in 0..MAX_CYCLES {
            let spark = spark.unwrap_or_else(|| self.spark_estimate(&trapped, conditions));
            let burn = self.burn(&trapped, conditions, spark);
            let closed_steps = ((end - start) / STEP).ceil() as usize;
            let a_in = sound(1.4, conditions.intake_temperature);
            let a_ex = sound(1.33, 0.9 * header_t);
            let delay_in = 2.0 * self.runner.length / a_in / dt;
            let delay_ex = 2.0 * self.header.length / a_ex / dt;
            // With both valves closed, the waves reflect off them unchanged.
            for _ in 0..closed_steps {
                intake.send(intake.arriving(delay_in, INTAKE_REFLECTION));
                header.send(header.arriving(delay_ex, HEADER_REFLECTION));
            }

            let mut charge = burn.end;
            let (mut work, mut heat) = (burn.work, burn.heat);
            let (mut exhaust, mut exhaust_heat, mut port_heat) = (0.0, 0.0, 0.0);
            let mut angle = start + closed_steps as f64 * STEP;
            for _ in closed_steps..STEPS {
                let next = angle + STEP;
                let (v, v1) = (self.volume(angle), self.volume(next));
                let mid = angle + STEP / 2.0;
                let t = charge.temperature();
                let p = charge.mass * R * t / v;
                let wave_in = intake.arriving(delay_in, INTAKE_REFLECTION);
                let wave_ex = header.arriving(delay_ex, HEADER_REFLECTION);
                // Waves can't take the pipe below a vacuum.
                let intake_port = Port {
                    area: self.intake.area(mid),
                    pressure: (p_in + 2.0 * wave_in).max(0.1 * p_in),
                    impedance: a_in / self.runner.area,
                    outside: runner_t,
                    inside: t,
                };
                let exhaust_port = Port {
                    area: self.exhaust.area(mid),
                    pressure: (p_ex + 2.0 * wave_ex).max(0.1 * p_ex),
                    impedance: a_ex / self.header.area,
                    outside: header_t,
                    inside: t,
                };
                let mut m_in = intake_port.solve(p);
                let mut m_ex = exhaust_port.solve(p);
                let s_in = intake_port.slope(m_in, p);
                let s_ex = exhaust_port.slope(m_ex, p);
                let gas_speed = if intake_port.area > 0.0 { 6.18 } else { 2.28 } * speed;
                let loss = woschni(self.bore, p, t, gas_speed) * self.wall_area(v) * (t - wall);
                // The cylinder's pressure change, with the flows' dependence on it taken
                // implicitly (the valves would otherwise make the step unstable).
                let g = gamma(t);
                let c = g * R * t * dt / v;
                let others = -g * p * (v1 - v) / v - (g - 1.0) * loss * dt / v;
                let dp = (others + c * (m_in + m_ex)) / (1.0 - c * (s_in + s_ex));
                m_in += s_in * dp;
                m_ex += s_ex * dp;
                intake.send(wave_in - intake_port.impedance * m_in);
                header.send(wave_ex - exhaust_port.impedance * m_ex);

                let dw = (p + dp / 2.0) * (v1 - v);
                let mut d_energy = -dw - loss * dt;
                let burnt_share = charge.burnt / charge.mass;
                let h = enthalpy(t);
                // Intake: in from the runner, which fresh air from the plenum replaces; or
                // back out into it, pushing as much into the plenum (big enough to count as
                // fresh air still).
                let dm = m_in * dt;
                if dm > 0.0 {
                    d_energy += dm * enthalpy(runner_t);
                    charge.burnt += dm * runner_burnt;
                    let share = (dm / runner_mass).min(1.0);
                    runner_burnt *= 1.0 - share;
                    runner_t += (t_in - runner_t) * share;
                } else if dm < 0.0 {
                    let out = -dm;
                    let share = out / (runner_mass + out);
                    runner_burnt += (burnt_share - runner_burnt) * share;
                    runner_t += (t - runner_t) * share;
                    d_energy -= out * h;
                    charge.burnt -= out * burnt_share;
                }
                charge.mass += dm;
                // Exhaust: out, or burnt gas back in from the header.
                let dm = m_ex * dt;
                if dm < 0.0 {
                    let out = -dm;
                    d_energy -= out * h;
                    charge.burnt -= out * burnt_share;
                    let lost = PORT_LOSS * (t - wall);
                    exhaust += out;
                    exhaust_heat += out * (t - lost);
                    port_heat += out * (enthalpy(t) - enthalpy(t - lost));
                    let header_mass = p_ex * self.header.area * self.header.length / (R * header_t);
                    header_t += (t - lost - header_t) * (out / header_mass).min(1.0);
                } else {
                    d_energy += dm * enthalpy(header_t);
                    charge.burnt += dm;
                }
                charge.mass += dm;
                charge.energy += d_energy;
                charge.mass = charge.mass.max(1e-9);
                charge.burnt = charge.burnt.clamp(0.0, charge.mass);
                charge.energy = charge.energy.max(charge.mass * energy(250.0));
                work += dw;
                heat += loss * dt;
                angle = next;
            }

            let change = (charge.fresh() - trapped.fresh()).abs() / charge.mass;
            let exhaust_temperature = if exhaust > 0.0 {
                exhaust_heat / exhaust
            } else {
                t_in
            };
            last = Some(CycleResult {
                trapped,
                air: trapped.air(),
                work,
                closed_work: burn.work,
                heat: heat + port_heat,
                exhaust_temperature,
                fuel_energy: trapped.fuel() * FUEL_ENERGY,
            });
            trapped = charge;
            if change < CONVERGED {
                break;
            }
        }
        last.expect("at least one cycle")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::engine::hardware::Intake;

    fn conditions(rpm: f64, intake: f64, exhaust: f64) -> Conditions {
        Conditions {
            rpm,
            intake_pressure: intake,
            exhaust_pressure: exhaust,
            intake_temperature: 300.0,
            coolant_temperature: 363.0,
            octane: 95.0,
        }
    }

    /// Volumetric efficiency of `hardware` on cam profile `profile`.
    fn ve(hardware: &EngineHardware, profile: usize, rpm: f64, intake: f64, exhaust: f64) -> f64 {
        let cylinder = Cylinder::new(hardware, &hardware.cams[profile], 0.0, 0.0);
        let conditions = conditions(rpm, intake, exhaust);
        let result = cylinder.run(&conditions, None);
        result.volumetric_efficiency(&cylinder, &conditions)
    }

    fn wide_open_ve(hardware: &EngineHardware, rpm: f64) -> f64 {
        ve(hardware, 0, rpm, 100e3, 105e3)
    }

    #[test]
    fn valves_and_volume() {
        let hardware = EngineHardware::default();
        let cylinder = Cylinder::new(&hardware, &hardware.cams[0], 0.0, 0.0);
        let cr = cylinder.volume(180.0) / cylinder.volume(0.0);
        assert!((cr - f64::from(hardware.compression_ratio)).abs() < 1e-6);
        assert_eq!(cylinder.intake.area(0.0), 0.0);
        assert!(cylinder.intake.area(-250.0) > 5e-4);
        assert!(cylinder.exhaust.area(250.0) > 4e-4);
        // Overlap around the top dead centre of the gas exchange.
        assert!(cylinder.intake.area(360.0) > 0.0 && cylinder.exhaust.area(-360.0) > 0.0);
    }

    #[test]
    fn two_big_valves_breathe_well_low_and_run_out_high() {
        let four = EngineHardware::default();
        let two = EngineHardware {
            valvetrain: crate::physics::engine::hardware::ValveLayout::PUSHROD,
            ..four.clone()
        };
        let share = |rpm| wide_open_ve(&two, rpm) / wide_open_ve(&four, rpm);
        let (low, high) = (share(2500.0), share(7000.0));
        assert!(low > 0.93, "{low} at 2500 rpm");
        assert!(high < low - 0.05, "{high} at 7000 rpm, {low} at 2500");
    }

    #[test]
    fn energy_and_temperature_agree() {
        for t in [250.0, 300.0, 900.0, 2600.0] {
            assert!((temperature(energy(t)) - t).abs() < 1e-6);
        }
    }

    #[test]
    fn runners_tune_the_band() {
        let long = EngineHardware {
            intake: Intake {
                runner_length: 0.5,
                ..EngineHardware::default().intake
            },
            ..EngineHardware::default()
        };
        let mut short = long.clone();
        short.intake.runner_length = 0.2;
        assert!(wide_open_ve(&long, 4000.0) > wide_open_ve(&short, 4000.0) + 0.05);
        assert!(wide_open_ve(&short, 7500.0) > wide_open_ve(&long, 7500.0) + 0.03);
    }

    #[test]
    fn a_wild_cam_breathes_high_and_idles_poorly() {
        let mut hardware = EngineHardware::default();
        hardware.cams.push(CamProfile {
            intake_duration: 280.0,
            exhaust_duration: 280.0,
            lobe_separation: 106.0,
            lift: 0.012,
        });
        let wide_open = |profile, rpm| ve(&hardware, profile, rpm, 100e3, 105e3);
        assert!(wide_open(1, 7500.0) > wide_open(0, 7500.0));
        assert!(wide_open(1, 2000.0) < wide_open(0, 2000.0));
        // At idle (low manifold pressure), the overlap lets exhaust back in.
        let idle = |profile| ve(&hardware, profile, 850.0, 30e3, 102e3);
        assert!(idle(1) < idle(0) * 0.9, "{} vs {}", idle(1), idle(0));
    }

    #[test]
    fn more_compression_is_more_efficient_and_knocks_earlier() {
        let stock = EngineHardware::default();
        let high = EngineHardware {
            compression_ratio: 12.0,
            ..stock.clone()
        };
        let conditions = conditions(2000.0, 100e3, 105e3);
        let run = |hardware: &EngineHardware| {
            let cylinder = Cylinder::new(hardware, &hardware.cams[0], 0.0, 0.0);
            let result = cylinder.run(&conditions, None);
            let knock = cylinder.knock_limit(&result.trapped, &conditions);
            (result.closed_work / result.fuel_energy, knock)
        };
        let ((stock_efficiency, stock_knock), (high_efficiency, high_knock)) =
            (run(&stock), run(&high));
        assert!(
            high_efficiency > stock_efficiency + 0.005,
            "{high_efficiency} vs {stock_efficiency}"
        );
        assert!(
            high_knock < stock_knock - 3.0,
            "{high_knock} vs {stock_knock}"
        );
        // Better fuel knocks later.
        let cylinder = Cylinder::new(&stock, &stock.cams[0], 0.0, 0.0);
        let result = cylinder.run(&conditions, None);
        let better = Conditions {
            octane: 98.0,
            ..conditions
        };
        assert!(cylinder.knock_limit(&result.trapped, &better) > stock_knock + 0.5);
    }

    #[test]
    fn the_spark_shapes_the_work() {
        let hardware = EngineHardware::default();
        let cylinder = Cylinder::new(&hardware, &hardware.cams[0], 0.0, 0.0);
        let conditions = conditions(4000.0, 100e3, 105e3);
        let result = cylinder.run(&conditions, None);
        let mbt = cylinder.mbt(&result.trapped, &conditions);
        let work = |spark| cylinder.burn(&result.trapped, &conditions, spark).work;
        assert!((15.0..40.0).contains(&mbt), "MBT {mbt}");
        // The estimate the gas exchange runs on is close to MBT.
        let estimate = cylinder.spark_estimate(&result.trapped, &conditions);
        assert!((estimate - mbt).abs() < 3.0, "{estimate} vs {mbt}");
        assert!(work(mbt - 15.0) < 0.95 * work(mbt) && work(mbt + 15.0) < 0.97 * work(mbt));
    }
}
