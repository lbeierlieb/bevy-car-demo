//! Small bumps in road surfaces: a road's unevenness (ISO 8608's classes, from its
//! roughness) as a sum of waves, and things built into it (speed bumps, humps, tables,
//! potholes). Heights and slopes are worked out exactly, so the tires feel what's drawn.

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;

use crate::terrain::hash;

/// The waves making a road's unevenness: lengths from `SHORTEST` to `LONGEST` metres
/// (longer ones are the road's profile, shorter ones the tire swallows), spaced evenly in
/// log.
const WAVES: usize = 20;
const SHORTEST: f32 = 0.5;
const LONGEST: f32 = 20.0;
/// The unevenness fades out over this far towards the road's edges (m), so the terrain
/// beside the road never shows through it.
const EDGE_FADE: f32 = 0.6;

#[derive(Clone, Copy, Debug)]
struct Wave {
    /// Cycles per metre along the road and across it (rightward).
    along: f32,
    across: f32,
    amplitude: f32,
    /// Where it starts (turns).
    phase: f32,
}

/// A road's unevenness.
#[derive(Clone, Debug, Default)]
pub struct Waves(Vec<Wave>);

impl Waves {
    /// Waves with ISO 8608's displacement spectrum G(n) = G₀ (n / 0.1)⁻², `roughness` being
    /// G₀ in 10⁻⁶ m³ (class A is 16), each wave standing for its share of the spectrum,
    /// heading in a random direction; `seed` picks them.
    pub fn new(roughness: f32, seed: u32) -> Self {
        if roughness <= 0.0 {
            return Self::default();
        }
        let g0 = roughness * 1e-6;
        let (low, high) = (1.0 / LONGEST, 1.0 / SHORTEST);
        let ratio = (high / low).ln() / (WAVES - 1) as f32;
        let waves = (0..WAVES)
            .map(|k| {
                let n = low * (ratio * k as f32).exp();
                // The band it stands for: n·ratio wide.
                let amplitude = (2.0 * g0 * (n / 0.1).powi(-2) * n * ratio).sqrt();
                let random = |i: u32| hash(seed.wrapping_mul(31).wrapping_add(k as u32 * 7 + i));
                let heading = random(1) * PI;
                Wave {
                    along: n * heading.cos(),
                    across: n * heading.sin(),
                    amplitude,
                    phase: random(2),
                }
            })
            .collect();
        Self(waves)
    }

    /// Height (m) at `along` metres along the road and `right` of its centreline, `margin`
    /// metres inside its edge, and its slope along and rightward.
    pub fn at(&self, along: f32, right: f32, margin: f32) -> (f32, Vec2) {
        if self.0.is_empty() {
            return (0.0, Vec2::ZERO);
        }
        let (mut height, mut slope) = (0.0, Vec2::ZERO);
        for w in &self.0 {
            let (sin, cos) = sin_cos_turns(w.along * along + w.across * right + w.phase);
            height += w.amplitude * cos;
            slope -= Vec2::new(w.along, w.across) * (TAU * w.amplitude * sin);
        }
        // Faded towards the edge (smoothstep; its own slope points inward, across).
        let t = (margin / EDGE_FADE).clamp(0.0, 1.0);
        let fade = t * t * (3.0 - 2.0 * t);
        let dfade = if t < 1.0 {
            6.0 * t * (1.0 - t) / EDGE_FADE
        } else {
            0.0
        };
        // Moving right takes the point towards the right edge if it's right of the middle.
        let inward = -right.signum();
        (
            height * fade,
            slope * fade + Vec2::new(0.0, height * dfade * inward),
        )
    }
}

/// The sine and cosine of `turns` whole turns (2π each), from plain arithmetic: the
/// same to the bit on every platform (the C library's `sinf` isn't: glibc picks a version
/// by the processor), which a server simulating the players' cars needs, and several
/// times quicker. Within 1e-6 of the true values.
pub fn sin_cos_turns(turns: f32) -> (f32, f32) {
    // Within an eighth of a turn of a quarter, and which quarter (rounded by converting:
    // `round` is a call into the C library on processors without SSE 4.1).
    let x = turns * 4.0;
    let quarters = (x + 0.5f32.copysign(x)) as i32;
    let a = (turns - quarters as f32 * 0.25) * TAU;
    let a2 = a * a;
    let sin = a * (1.0 - a2 / 6.0 * (1.0 - a2 / 20.0 * (1.0 - a2 / 42.0 * (1.0 - a2 / 72.0))));
    let cos = 1.0 - a2 / 2.0 * (1.0 - a2 / 12.0 * (1.0 - a2 / 30.0 * (1.0 - a2 / 56.0)));
    match quarters & 3 {
        0 => (sin, cos),
        1 => (cos, -sin),
        2 => (-sin, -cos),
        _ => (-cos, sin),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureKind {
    /// A short bump with a rounded top, across the road.
    Bump,
    /// A long, gentle hump across the road.
    Hump,
    /// A flat top with ramps, across the road.
    Table,
    Pothole,
}

/// Something built into a road.
#[derive(Clone, Copy, Debug)]
pub struct Feature {
    pub kind: FeatureKind,
    /// Where: metres along the road and right of its centreline.
    pub along: f32,
    pub right: f32,
    /// Length along the road (potholes: diameter), and height (potholes: negative).
    pub size: f32,
    pub height: f32,
}

/// Length of a table's ramps (m).
const RAMP: f32 = 1.0;

impl Feature {
    pub fn kind_from_byte(byte: u8) -> FeatureKind {
        [
            FeatureKind::Bump,
            FeatureKind::Hump,
            FeatureKind::Table,
            FeatureKind::Pothole,
        ][byte as usize]
    }

    /// How far along the road from its middle it reaches (m).
    pub fn reach(&self) -> f32 {
        self.size / 2.0
    }

    /// Height (m) it adds at `along` and `right` (see `Waves::at`), and its slope along
    /// and rightward.
    pub fn at(&self, along: f32, right: f32) -> (f32, Vec2) {
        let (ds, dc) = (along - self.along, right - self.right);
        if ds.abs() >= self.reach() {
            return (0.0, Vec2::ZERO);
        }
        match self.kind {
            // cos² over its length.
            FeatureKind::Bump | FeatureKind::Hump => {
                let k = PI / self.size;
                let (sin, cos) = (k * ds).sin_cos();
                (
                    self.height * cos * cos,
                    Vec2::new(-2.0 * self.height * k * sin * cos, 0.0),
                )
            }
            FeatureKind::Table => {
                let to_end = self.reach() - ds.abs();
                if to_end >= RAMP {
                    (self.height, Vec2::ZERO)
                } else {
                    // Smoothstep ramps.
                    let t = to_end / RAMP;
                    let rise = self.height * 6.0 * t * (1.0 - t) / RAMP;
                    (
                        self.height * t * t * (3.0 - 2.0 * t),
                        Vec2::new(-ds.signum() * rise, 0.0),
                    )
                }
            }
            // A bowl: cos² of the distance from its middle.
            FeatureKind::Pothole => {
                let radius = self.size / 2.0;
                let rho = ds.hypot(dc);
                if rho >= radius {
                    return (0.0, Vec2::ZERO);
                }
                let k = PI / (2.0 * radius);
                let (sin, cos) = (k * rho).sin_cos();
                let along_rho = -2.0 * self.height * k * sin * cos;
                let direction = Vec2::new(ds, dc) / rho.max(1e-6);
                (self.height * cos * cos, direction * along_rho)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sines_of_turns_are_the_sines() {
        for i in -20000..20000 {
            let turns = i as f32 * 0.01237;
            let (sin, cos) = sin_cos_turns(turns);
            let angle = f64::from(turns) * std::f64::consts::TAU;
            assert!((f64::from(sin) - angle.sin()).abs() < 1e-6, "sin {turns}");
            assert!((f64::from(cos) - angle.cos()).abs() < 1e-6, "cos {turns}");
        }
    }

    /// Checks `at`'s slopes against differences of its heights.
    fn slopes_match(at: impl Fn(f32, f32) -> (f32, Vec2), points: &[(f32, f32)]) {
        let e = 1e-3;
        for &(s, c) in points {
            let (_, slope) = at(s, c);
            let ds = (at(s + e, c).0 - at(s - e, c).0) / (2.0 * e);
            let dc = (at(s, c + e).0 - at(s, c - e).0) / (2.0 * e);
            assert!(
                (slope - Vec2::new(ds, dc)).length() < 2e-3,
                "slope {slope} vs {ds}, {dc} at {s}, {c}"
            );
        }
    }

    #[test]
    fn waves_are_as_rough_as_their_class_and_their_slopes_fit() {
        let rms = |roughness: f32| {
            let waves = Waves::new(roughness, 7);
            let n = 20_000;
            let sum: f32 = (0..n)
                .map(|i| waves.at(i as f32 * 0.37, 0.5, 5.0).0.powi(2))
                .sum();
            (sum / n as f32).sqrt()
        };
        // Class A (16): under 2 mm; each class up doubles it.
        let a = rms(16.0);
        assert!((0.001..0.003).contains(&a), "class A: {a} m");
        assert!((rms(64.0) / a - 2.0).abs() < 0.2);
        let waves = Waves::new(256.0, 3);
        slopes_match(
            |s, c| waves.at(s, c, 2.0 - c.abs()),
            &[(1.0, 0.3), (17.3, -1.2), (40.0, 1.8), (3.3, -1.95)],
        );
        // Nothing at the very edge.
        assert_eq!(waves.at(12.0, 2.0, 0.0).0, 0.0);
    }

    #[test]
    fn features_have_their_shape_and_their_slopes_fit() {
        let feature = |kind, size, height| Feature {
            kind,
            along: 10.0,
            right: 0.5,
            size,
            height,
        };
        let bump = feature(FeatureKind::Bump, 0.6, 0.08);
        assert_eq!(bump.at(10.0, 3.0).0, 0.08);
        assert_eq!(bump.at(10.31, 0.0).0, 0.0);
        let table = feature(FeatureKind::Table, 8.0, 0.07);
        assert_eq!(table.at(12.5, 0.0).0, 0.07);
        let pothole = feature(FeatureKind::Pothole, 0.8, -0.06);
        assert!((pothole.at(10.0, 0.5).0 + 0.06).abs() < 1e-6);
        assert_eq!(pothole.at(10.0, 0.95).0, 0.0);
        for f in [bump, table, pothole, feature(FeatureKind::Hump, 3.7, 0.07)] {
            slopes_match(
                |s, c| f.at(s, c),
                &[
                    (9.9, 0.4),
                    (10.2, 0.6),
                    (8.5, 0.0),
                    (11.2, 0.2),
                    (13.5, 1.0),
                ],
            );
        }
    }
}
