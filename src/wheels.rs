//! Rims and tires: their sizes, which tire goes on which rim, what fits under a car, and
//! what they weigh.
//!
//! A tire size is written the usual way, `205/55 R16`: 205 mm wide, its sidewall 55 % of
//! that high, on a 16" rim. A rim is a diameter and a width (in inches). Each axle of a car
//! has its own rims and tires, of a compound in `tire::COMPOUNDS`.

use crate::physics::tire::{COMPOUNDS, GRIP, RELAXATION, ROLLING, TireParams};

/// Metres in an inch.
const INCH: f32 = 0.0254;
/// A rim's mass (kg): this much, and this much per inch of diameter times inch of width.
const RIM_MASS: f32 = 1.0;
const RIM_MASS_PER_SIZE: f32 = 0.068;
/// A tire's mass per area (kg/m²) of its tread (width times circumference) and of its
/// sidewalls (both rings between the rim and the tread).
const TREAD_MASS: f32 = 19.0;
const SIDEWALL_MASS: f32 = 2.8;
/// A rim's moment of inertia over its mass times its bead seat radius squared: most of its
/// mass is in the barrel, the spokes are inside.
const RIM_INERTIA: f32 = 0.65;
/// How far in from the outside the tread's mass is, as a share of the radius.
const TREAD_DEPTH: f32 = 0.95;
/// The load a tire is made for (see `TireParams::nominal_load`) per metre of its width, on
/// a tire of `DESIGN_DIAMETER`; it grows with the root of the diameter (a longer contact
/// patch).
const DESIGN_LOAD: f32 = 14_500.0;
const DESIGN_DIAMETER: f32 = 0.64;
/// The tire the carcass figures are for: its width (m) and sidewall (%). A wider tire is
/// stiffer and rolls heavier (with the root of the width); a lower sidewall is stiffer
/// and peaks at a smaller slip angle.
const REFERENCE_WIDTH: f32 = 0.215;
const REFERENCE_ASPECT: f32 = 45.0;
const STIFFNESS: f32 = 200_000.0;
const DAMPING: f32 = 1_500.0;
/// How the stiffness and the peak slip angle follow the sidewall's height.
const SIDEWALL_STIFFNESS: f32 = 0.3;
const SIDEWALL_PEAK: f32 = 0.35;
/// How the relaxation length follows the sidewall's height (a taller one flexes more).
const SIDEWALL_RELAXATION: f32 = 0.4;
/// Moment of inertia of what spins with the wheel on the hub (brake disc, hub, half the
/// drive shaft) per kg of the corner's unsprung mass without its wheel (m²).
pub const HUB_INERTIA: f32 = 0.004;
/// The rim width that suits a tire best (inches per mm of tire width), and how far either
/// side of it a rim still takes the tire (inches).
const IDEAL_RIM: f32 = 0.8 / 25.4;
const RIM_SPREAD: f32 = 1.25;

/// The offered sizes: rim diameters and widths (in tenths of an inch), tire widths (mm)
/// and sidewalls (% of the width).
pub const RIM_DIAMETERS: std::ops::RangeInclusive<u8> = 13..=20;
pub const RIM_WIDTHS: std::ops::RangeInclusive<u8> = 50..=120;
pub const RIM_WIDTH_STEP: usize = 5;
pub const TIRE_WIDTHS: std::ops::RangeInclusive<u16> = 145..=345;
pub const TIRE_WIDTH_STEP: usize = 10;
pub const ASPECTS: std::ops::RangeInclusive<u8> = 25..=75;
pub const ASPECT_STEP: usize = 5;

/// A rim: its diameter (inches) and width (tenths of an inch).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Rim {
    pub diameter: u8,
    pub width: u8,
}

impl Rim {
    /// Width (inches).
    pub fn width_inches(&self) -> f32 {
        f32::from(self.width) / 10.0
    }

    /// Radius of the bead seat, where the tire sits (m).
    pub fn radius(&self) -> f32 {
        f32::from(self.diameter) * INCH / 2.0
    }

    pub fn mass(&self) -> f32 {
        RIM_MASS + RIM_MASS_PER_SIZE * f32::from(self.diameter) * self.width_inches()
    }

    /// Moment of inertia about the axle (kg·m²).
    pub fn inertia(&self) -> f32 {
        RIM_INERTIA * self.mass() * self.radius().powi(2)
    }

    /// E.g. `16×7.0"`.
    pub fn label(&self) -> String {
        format!("{}×{:.1}\"", self.diameter, self.width_inches())
    }

    /// Every rim on offer.
    pub fn all() -> impl Iterator<Item = Rim> {
        RIM_DIAMETERS.flat_map(|diameter| {
            (RIM_WIDTHS.step_by(RIM_WIDTH_STEP)).map(move |width| Rim { diameter, width })
        })
    }
}

/// A tire's size: its width (mm), its sidewall's height (% of the width), and the rim
/// diameter it goes on (inches).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TireSize {
    pub width: u16,
    pub aspect: u8,
    pub rim: u8,
}

impl TireSize {
    /// Width of the tread (m).
    pub fn width_m(&self) -> f32 {
        f32::from(self.width) / 1000.0
    }

    /// Height of the sidewall (m).
    pub fn sidewall(&self) -> f32 {
        self.width_m() * f32::from(self.aspect) / 100.0
    }

    /// Radius of the rim it sits on (m).
    pub fn rim_radius(&self) -> f32 {
        f32::from(self.rim) * INCH / 2.0
    }

    /// Outside radius, unloaded (m).
    pub fn radius(&self) -> f32 {
        self.rim_radius() + self.sidewall()
    }

    /// The load (N) it grips with its compound's friction at; more grips less per newton.
    pub fn design_load(&self) -> f32 {
        let diameter = 2.0 * self.radius();
        DESIGN_LOAD * self.width_m() * (diameter / DESIGN_DIAMETER).sqrt()
    }

    /// The tire of this size and `compound` (in `tire::COMPOUNDS`).
    pub fn params(&self, compound: usize) -> TireParams {
        let compound = COMPOUNDS[compound];
        let wider = self.width_m() / REFERENCE_WIDTH;
        let taller = f32::from(self.aspect) / REFERENCE_ASPECT;
        let stiffer = wider.sqrt() * taller.powf(-SIDEWALL_STIFFNESS);
        TireParams {
            radius: self.radius(),
            width: self.width_m(),
            stiffness: STIFFNESS * stiffer,
            damping: DAMPING * stiffer,
            compound,
            mu: GRIP * compound.grip,
            nominal_load: self.design_load(),
            sidewall: taller.powf(SIDEWALL_PEAK),
            tread_area: self.areas().0,
            rolling: ROLLING * compound.rolling * wider.sqrt(),
            relaxation: RELAXATION * taller.powf(SIDEWALL_RELAXATION),
            ..TireParams::default()
        }
    }

    /// Areas of the tread and of both sidewalls (m²).
    fn areas(&self) -> (f32, f32) {
        let (outside, inside) = (self.radius(), self.rim_radius());
        let tread = self.width_m() * std::f32::consts::TAU * outside;
        let sidewalls = std::f32::consts::TAU * (outside.powi(2) - inside.powi(2));
        (tread, sidewalls)
    }

    pub fn mass(&self) -> f32 {
        let (tread, sidewalls) = self.areas();
        TREAD_MASS * tread + SIDEWALL_MASS * sidewalls
    }

    /// Moment of inertia about the axle (kg·m²): the tread near the outside, the sidewalls
    /// as rings.
    pub fn inertia(&self) -> f32 {
        let (tread, sidewalls) = self.areas();
        let (outside, inside) = (self.radius(), self.rim_radius());
        TREAD_MASS * tread * (TREAD_DEPTH * outside).powi(2)
            + SIDEWALL_MASS * sidewalls * (outside.powi(2) + inside.powi(2)) / 2.0
    }

    /// Whether it goes on `rim`: of its diameter, and not too narrow or wide for it.
    pub fn fits(&self, rim: &Rim) -> bool {
        let ideal = f32::from(self.width) * IDEAL_RIM;
        rim.diameter == self.rim && (rim.width_inches() - ideal).abs() <= RIM_SPREAD
    }

    /// E.g. `205/55 R16`.
    pub fn label(&self) -> String {
        format!("{}/{} R{}", self.width, self.aspect, self.rim)
    }

    /// The tire of `width` (mm) for rims of `rim` (inches) whose outside diameter is
    /// nearest `diameter` (m): the same rolling size on other rims or tires.
    pub fn nearest(width: u16, rim: u8, diameter: f32) -> TireSize {
        (ASPECTS.step_by(ASPECT_STEP))
            .map(|aspect| TireSize { width, aspect, rim })
            .min_by(|a, b| {
                let off = |tire: &TireSize| (2.0 * tire.radius() - diameter).abs();
                off(a).total_cmp(&off(b))
            })
            .expect("some sizes")
    }

    /// The rim width on offer (tenths of an inch) that suits this tire nearest `width`.
    pub fn rim_width_near(&self, width: u8) -> u8 {
        (RIM_WIDTHS.step_by(RIM_WIDTH_STEP))
            .filter(|&w| {
                self.fits(&Rim {
                    diameter: self.rim,
                    width: w,
                })
            })
            .min_by_key(|&w| w.abs_diff(width))
            .unwrap_or(width)
    }

    /// Every tire on offer.
    pub fn all() -> impl Iterator<Item = TireSize> {
        RIM_DIAMETERS.flat_map(|rim| {
            (TIRE_WIDTHS.step_by(TIRE_WIDTH_STEP)).flat_map(move |width| {
                (ASPECTS.step_by(ASPECT_STEP)).map(move |aspect| TireSize { width, aspect, rim })
            })
        })
    }
}

/// An axle's wheels: rims, and tires of a size and a compound (in `tire::COMPOUNDS`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Wheel {
    pub rim: Rim,
    pub tire: TireSize,
    pub compound: usize,
}

impl Wheel {
    /// Mass of one rim with its tire (kg).
    pub fn mass(&self) -> f32 {
        self.rim.mass() + self.tire.mass()
    }

    /// Moment of inertia of one rim with its tire (kg·m²).
    pub fn inertia(&self) -> f32 {
        self.rim.inertia() + self.tire.inertia()
    }

    /// The same rims and tires in `compound`.
    pub fn with_compound(self, compound: usize) -> Self {
        Self { compound, ..self }
    }

    /// E.g. `205/55 R16 street on 16×7.0"`.
    pub fn label(&self) -> String {
        format!(
            "{} {} on {}",
            self.tire.label(),
            COMPOUNDS[self.compound].name.to_lowercase(),
            self.rim.label()
        )
    }
}

/// The room in a wheel well: the largest tire (outside diameter, m, and width, mm), and the
/// smallest rim that clears the brakes (inches).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelRoom {
    pub diameter: f32,
    pub width: u16,
    pub min_rim: u8,
}

impl WheelRoom {
    /// Why `wheel` doesn't go on here, if it doesn't.
    pub fn misfit(&self, wheel: &Wheel) -> Option<String> {
        let (rim, tire) = (&wheel.rim, &wheel.tire);
        if !tire.fits(rim) {
            Some(if tire.rim != rim.diameter {
                format!("{} tires need {}\" rims", tire.label(), tire.rim)
            } else {
                format!("{} tires don't suit {} rims", tire.label(), rim.label())
            })
        } else if rim.diameter < self.min_rim {
            Some(format!(
                "{}\" rims don't clear the brakes (at least {}\")",
                rim.diameter, self.min_rim
            ))
        } else if 2.0 * tire.radius() > self.diameter + 1e-4 {
            Some(format!(
                "{} tires are too tall for the wheel wells ({:.0} mm, room for {:.0})",
                tire.label(),
                2000.0 * tire.radius(),
                1000.0 * self.diameter
            ))
        } else if tire.width > self.width {
            Some(format!(
                "{} tires are too wide for the wheel wells (room for {} mm)",
                tire.label(),
                self.width
            ))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: u16, aspect: u8, rim: u8) -> TireSize {
        TireSize { width, aspect, rim }
    }

    #[test]
    fn sizes_weigh_what_real_ones_do() {
        let tire = size(205, 55, 16);
        assert!((tire.radius() - 0.316).abs() < 0.001);
        assert_eq!(tire.label(), "205/55 R16");
        // Tires from about 6.5 kg (a city car's) to 10.5 kg (a sports car's).
        assert!((size(165, 65, 14).mass() - 6.5).abs() < 0.3);
        assert!((tire.mass() - 8.8).abs() < 0.3);
        assert!((size(245, 40, 18).mass() - 10.5).abs() < 0.4);
        let rim = Rim {
            diameter: 16,
            width: 70,
        };
        assert!((rim.mass() - 8.6).abs() < 0.2);
        // A wheel and tire spins with about 1 kg·m².
        let inertia = rim.inertia() + tire.inertia();
        assert!(inertia > 0.8 && inertia < 1.1, "{inertia}");
    }

    #[test]
    fn wider_tires_grip_more_but_roll_heavier() {
        use crate::physics::tire::STREET;
        let (narrow, wide) = (size(205, 45, 17), size(245, 40, 17));
        let (n, w) = (narrow.params(STREET), wide.params(STREET));
        // About 5 % more grip under the same load for a fifth more width.
        let more = w.friction(3500.0) / n.friction(3500.0);
        assert!(more > 1.04 && more < 1.07, "{more}");
        assert!(w.rolling > n.rolling && w.tread_area > n.tread_area);
        assert!(wide.mass() > narrow.mass() && wide.inertia() > narrow.inertia());
        // A lower sidewall is stiffer and peaks sooner.
        assert!(w.sidewall < n.sidewall && w.stiffness > n.stiffness);
    }

    #[test]
    fn tires_go_on_rims_of_their_size() {
        let tire = size(205, 55, 16);
        let rim = |diameter, width| Rim { diameter, width };
        assert!(tire.fits(&rim(16, 65)) && tire.fits(&rim(16, 75)));
        assert!(!tire.fits(&rim(17, 70)) && !tire.fits(&rim(16, 90)));
        let room = WheelRoom {
            diameter: 0.66,
            width: 225,
            min_rim: 15,
        };
        let wheel = |rim, tire| Wheel {
            rim,
            tire,
            compound: 0,
        };
        assert_eq!(room.misfit(&wheel(rim(16, 70), tire)), None);
        assert!(room.misfit(&wheel(rim(17, 70), tire)).is_some());
        assert!(
            room.misfit(&wheel(rim(14, 60), size(205, 65, 14)))
                .is_some()
        );
        assert!(
            room.misfit(&wheel(rim(16, 70), size(205, 75, 16)))
                .is_some()
        );
        assert!(
            room.misfit(&wheel(rim(16, 80), size(245, 45, 16)))
                .is_some()
        );
        // On bigger rims, the tire that rolls the same size has a lower sidewall.
        let plus = TireSize::nearest(245, 18, 2.0 * size(225, 45, 17).radius());
        assert_eq!(plus, size(245, 35, 18));
        assert_eq!(plus.rim_width_near(70), 70);
        assert_eq!(plus.rim_width_near(55), 65);
    }
}
