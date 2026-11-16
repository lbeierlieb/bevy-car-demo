//! Simplified Pacejka "Magic Formula" tire with combined slip.

/// Below this speed, slips are computed against this speed instead, which keeps the
/// forces bounded and stable when the car is (nearly) standing still.
const LOW_SPEED: f32 = 3.0;

#[derive(Clone, Copy, Debug)]
pub struct TireParams {
    pub radius: f32,
    /// Vertical stiffness (N/m) and damping (N·s/m) of the tire carcass.
    pub stiffness: f32,
    pub damping: f32,
    /// Peak friction coefficient at the nominal load: the Magic Formula `D = mu · Fz`.
    pub mu: f32,
    /// Load (N) at which the friction is `mu` (the corner's static load), and how it falls
    /// off with more: `mu · (Fz / nominal)^-load_sensitivity`. A loaded tire grips harder,
    /// but less than in proportion, so weight shifting across an axle costs it grip.
    pub nominal_load: f32,
    pub load_sensitivity: f32,
    /// Magic Formula stiffness `B` and shape `C`, longitudinal (per unit slip ratio).
    pub b_long: f32,
    pub c_long: f32,
    /// Magic Formula stiffness `B` and shape `C`, lateral (per radian of slip angle).
    pub b_lat: f32,
    pub c_lat: f32,
    /// How far behind the contact centre the lateral force acts at small slip angles (m).
    pub pneumatic_trail: f32,
}

impl Default for TireParams {
    fn default() -> Self {
        Self {
            radius: 0.33,
            stiffness: 200_000.0,
            damping: 1_500.0,
            mu: 1.1,
            nominal_load: 3500.0,
            load_sensitivity: 0.1,
            b_long: 14.0,
            c_long: 1.6,
            b_lat: 12.0,
            c_lat: 1.4,
            pneumatic_trail: 0.04,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TireForce {
    /// Longitudinal force along the tire's forward direction.
    pub fx: f32,
    /// Lateral force along the tire's right direction.
    pub fy: f32,
    pub slip_ratio: f32,
    pub slip_angle: f32,
}

/// The pure-slip Magic Formula (with `E = 0`): `D · sin(C · atan(B · s))`.
fn magic_formula(slip: f32, b: f32, c: f32, d: f32) -> f32 {
    d * (c * (b * slip).atan()).sin()
}

/// Slip at which the pure Magic Formula peaks: `C · atan(B · s) = π/2`.
fn peak_slip(b: f32, c: f32) -> f32 {
    (std::f32::consts::FRAC_PI_2 / c).tan() / b
}

impl TireParams {
    /// Peak friction coefficient under a load of `fz` (N).
    pub fn friction(&self, fz: f32) -> f32 {
        // Light loads (a wheel lifting) don't grip without bound.
        let relative = (fz / self.nominal_load).max(0.25);
        self.mu * relative.powf(-self.load_sensitivity)
    }

    /// Pneumatic trail at a slip: it shrinks as more of the contact patch slides and is
    /// gone at the peak slip angle, so the self-aligning torque (lateral force times trail)
    /// fades before the grip does. That's what makes the steering go light near the limit.
    pub fn trail(&self, force: &TireForce) -> f32 {
        let slip = (force.slip_angle / peak_slip(self.b_lat, self.c_lat)).abs();
        self.pneumatic_trail * (1.0 - slip).max(0.0)
    }

    /// Tire forces for a contact patch moving at (`vx` forward, `vy` right) with the wheel
    /// spinning at `omega` (rad/s, positive rolls forward) under normal load `fz`.
    pub fn force(&self, vx: f32, vy: f32, omega: f32, fz: f32) -> TireForce {
        let denom = vx.abs().max(LOW_SPEED);
        let slip_ratio = (omega * self.radius - vx) / denom;
        let slip_angle = (vy / denom).atan();

        // Combined slip: normalise both slips by their peak, so that the total force follows
        // the direction of the combined slip vector and saturates along a friction ellipse.
        let peak_long = peak_slip(self.b_long, self.c_long);
        let peak_lat = peak_slip(self.b_lat, self.c_lat);
        let sx = slip_ratio / peak_long;
        let sy = slip_angle / peak_lat;
        let rho = sx.hypot(sy);
        if rho < 1e-6 {
            return TireForce {
                slip_ratio,
                slip_angle,
                ..Default::default()
            };
        }
        let d = self.friction(fz) * fz;
        let fx = sx / rho * magic_formula(rho * peak_long, self.b_long, self.c_long, d);
        // Lateral force opposes sideways sliding.
        let fy = -sy / rho * magic_formula(rho * peak_lat, self.b_lat, self.c_lat, d);
        TireForce {
            fx,
            fy,
            slip_ratio,
            slip_angle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_force_equals_mu_times_load() {
        let tire = TireParams::default();
        let peak = peak_slip(tire.b_long, tire.c_long);
        let f = tire.force(20.0, 0.0, 20.0 * (1.0 + peak) / tire.radius, 4000.0);
        assert!((f.fx - tire.friction(4000.0) * 4000.0).abs() < 1.0);
        assert!(f.fy.abs() < 1e-3);
    }

    #[test]
    fn combined_slip_stays_inside_friction_circle() {
        let tire = TireParams::default();
        let f = tire.force(20.0, 5.0, 30.0 / tire.radius, 4000.0);
        assert!(f.fx > 0.0 && f.fy < 0.0);
        assert!(f.fx.hypot(f.fy) <= tire.friction(4000.0) * 4000.0 + 1.0);
    }

    #[test]
    fn more_load_grips_harder_but_less_than_in_proportion() {
        let tire = TireParams::default();
        assert_eq!(tire.friction(tire.nominal_load), tire.mu);
        let (light, heavy) = (tire.nominal_load * 0.5, tire.nominal_load * 1.5);
        assert!(tire.friction(light) > tire.mu && tire.friction(heavy) < tire.mu);
        // Weight moving from one side of an axle to the other costs it grip.
        let axle = |left: f32, right: f32| left * tire.friction(left) + right * tire.friction(right);
        let level = axle(tire.nominal_load, tire.nominal_load);
        assert!(axle(light, heavy) < 0.99 * level);
    }
}
