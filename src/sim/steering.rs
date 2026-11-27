//! What the front wheels do to the steering: the moment each one's forces make about its
//! steering axis, summed at the rack. This is what force feedback plays.
//!
//! Worked out from the linkage as it stands each substep, so it carries everything a real
//! rack feels:
//!
//! - the tire's lateral force behind the axis: pneumatic trail (gone towards the limit, also
//!   under drive and braking) and the caster's mechanical trail;
//! - the tire's longitudinal force at the patch's lateral offset from the axis (scrub
//!   radius): braking and rolling resistance, uneven between the sides over bumps;
//! - the driveshaft's torque on the hub (front- and all-wheel drive): with the kingpin
//!   inclination it acts as if the drive force pushed at the wheel centre, far outboard of
//!   the axis, and with the shaft inclined as the wheel moves up and down, it tugs the
//!   wheel over bumps under power and straightens it with the body's roll in a corner.
//!   Beside a transverse engine the shafts differ in length, so the short one tilts more
//!   as the nose lifts: torque steer;
//! - the load through the inclined, castered axis: centring when steered (the car is lifted)
//!   and a pull towards the more loaded side (a bump under one wheel);
//! - the drive force acting where the carcass has moved the patch sideways in a corner.
//!
//! Equal forces on both sides cancel at the rack, as on a real car.

use bevy::prelude::*;

use crate::car::CarSetup;
use crate::chassis::GearboxKind;
use crate::physics::suspension::{AxleSetup, Corner};

/// Distance the front tires' contact points trail the steering axes (from caster, m).
/// Past the peak slip angle the pneumatic trail is gone and this is all that keeps
/// the wheel pulling towards where the front tires roll: into countersteer in a slide.
/// The linkage itself has no caster; the steering axis is tilted back for this only.
pub const MECHANICAL_TRAIL: f32 = 0.03;
/// Sideways stiffness of a tire's carcass (N/m): how far a lateral force moves the patch
/// out of the wheel's plane, where the longitudinal force then acts.
const CARCASS_RATE: f32 = 180_000.0;
/// Where a transverse gearbox's differential sits: left of the car's middle (m), so the
/// left driveshaft is the short one.
const TRANSVERSE_DIFFERENTIAL: f32 = -0.12;
/// Full scale per newton on the front axle (m): see `full_scale`.
const FULL_SCALE_LEVER: f32 = 0.055;

/// The moments about the front wheels' steering axes, summed at the rack, by what makes
/// them (N·m, positive turns the wheels left).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rack {
    /// The tires' lateral forces: pneumatic and mechanical trail.
    pub lateral: f32,
    /// The tires' longitudinal forces at the patches (braking, rolling resistance, bump
    /// impacts, the drive's reaction on the road), with the carcass' sideways deflection.
    pub longitudinal: f32,
    /// The loads through the inclined, castered axes.
    pub vertical: f32,
    /// The driveshafts' torque on the hubs.
    pub drive: f32,
}

impl Rack {
    pub fn total(&self) -> f32 {
        self.lateral + self.longitudinal + self.vertical + self.drive
    }

    /// The moments of one front corner about its own steering axis, added, its chassis at
    /// `position` and `rotation`, the driveshafts' inner joints either side of
    /// `differential` (x in the chassis).
    pub fn add(
        &mut self,
        axle: &AxleSetup,
        corner: &Corner,
        differential: f32,
        position: Vec3,
        rotation: Quat,
    ) {
        let pose = &corner.pose;
        let to_local = rotation.inverse();
        let kingpin = (pose.upper_ball - pose.lower_ball).normalize();

        // The driveshaft's torque, from the inner joint (beside the differential, at the
        // hub's design height) to the hub, pointing outwards like the axle. A positive
        // drive torque rolls the wheel forward, a spin about -x.
        let g = &axle.geometry;
        let x = differential + corner.side * g.lower_pivot.x;
        let inner = Vec3::new(x, g.wheel_center.y, corner.axle_z);
        let shaft = (pose.wheel_center - inner).normalize() * corner.side;
        self.drive -= corner.drive_torque * shaft.dot(kingpin);

        let Some(contact) = corner.contact else {
            return;
        };
        let point = to_local * (contact.point - position);
        let normal = to_local * contact.normal;
        let forward = normal.cross(pose.upright * Vec3::X).normalize();
        let right = forward.cross(normal);
        // The axis tilted back so that it meets the road the mechanical trail ahead of
        // the patch (forward is -z).
        let below = (pose.lower_ball.y - point.y).max(0.05);
        let axis = (kingpin + Vec3::Z * MECHANICAL_TRAIL * kingpin.y / below).normalize();
        let arm = point - pose.lower_ball;
        let moment = |force: Vec3| arm.cross(force).dot(axis);
        // The tire's own aligning moment about the road's normal: the lateral force
        // behind the patch's centre by the pneumatic trail, and the longitudinal force
        // beside it where the carcass has carried the patch.
        let tire = &contact.tire;
        let about_normal = normal.dot(axis);
        self.lateral += moment(right * tire.fy) + axle.tire.trail(tire) * tire.fy * about_normal;
        self.longitudinal +=
            moment(forward * tire.fx) + tire.fx * tire.fy / CARCASS_RATE * about_normal;
        self.vertical += moment(normal * contact.load);
    }

    /// The front corners' moments in a car built as `setup`.
    pub fn of(setup: &CarSetup, corners: &[Corner], position: Vec3, rotation: Quat) -> Self {
        let differential = match setup.build.gearbox().kind {
            GearboxKind::Transverse => TRANSVERSE_DIFFERENTIAL,
            GearboxKind::Inline | GearboxKind::Transaxle => 0.0,
        };
        let mut rack = Self::default();
        for corner in corners.iter().filter(|corner| corner.front) {
            rack.add(&setup.front, corner, differential, position, rotation);
        }
        rack
    }

    pub fn scaled(self, by: f32) -> Self {
        Self {
            lateral: self.lateral * by,
            longitudinal: self.longitudinal * by,
            vertical: self.vertical * by,
            drive: self.drive * by,
        }
    }
}

impl std::ops::AddAssign for Rack {
    fn add_assign(&mut self, other: Self) {
        self.lateral += other.lateral;
        self.longitudinal += other.longitudinal;
        self.vertical += other.vertical;
        self.drive += other.drive;
    }
}

/// Steering torque that drives the wheel at full force with the strength at 1 (N·m): in
/// proportion to the weight on the front axle, like power steering sized for the car, so
/// a light car's wheel is as alive as a heavy one's. Near the limit the tires reach about
/// this much.
pub fn full_scale(setup: &CarSetup) -> f32 {
    let front = 2.0 * (setup.corner_mass(true) + setup.front.unsprung_mass) * 9.81;
    FULL_SCALE_LEVER * front
}

crate::snapshot::snapshot!(Rack {
    lateral,
    longitudinal,
    vertical,
    drive
});

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::Car;
    use crate::physics::drivetrain::Drive;
    use crate::sim::{CHASSIS_HZ, step_car};
    use crate::terrain::Terrain;

    /// Steering torque (shares of full scale) on a 60 m left-hand circle at `lateral` g,
    /// held steady, then with the throttle floored for half a second (traction control on).
    fn cornering(setup: CarSetup, lateral: f32) -> (f32, f32) {
        let terrain = Terrain::flat();
        let radius = 60.0;
        let speed = (lateral * 9.81 * radius).sqrt();
        let scale = full_scale(&setup);
        let mut car = Car::spawn_at(0, setup, &terrain, 0.0, 0.0, 0.0);
        car.warm_tires();
        car.parking_brake = false;
        car.drivetrain.gear = 2;
        let dt = 1.0 / CHASSIS_HZ as f32;
        let mut steady = 0.0;
        let steps = 10 * CHASSIS_HZ as usize;
        for step in 0..steps {
            let forward = car.forward_speed();
            let wanted = forward.max(1.0) / radius;
            let yaw = car.body.angular_velocity.y;
            let c = &mut car.controls;
            c.steer = (c.steer + 2.0 * (wanted - yaw) * dt).clamp(0.0, 1.0);
            c.throttle = (0.3 + 0.5 * (speed - forward)).clamp(0.0, 1.0);
            step_car(&mut car, &terrain, dt);
            if step >= steps - CHASSIS_HZ as usize {
                steady += car.rack.total() / CHASSIS_HZ as f32;
            }
        }
        let mut power = 0.0;
        let steps = CHASSIS_HZ as usize / 2;
        car.controls.throttle = 1.0;
        for step in 0..steps {
            step_car(&mut car, &terrain, dt);
            if step >= steps / 2 {
                power += car.rack.total() / (steps - steps / 2) as f32;
            }
        }
        (steady / scale, power / scale)
    }

    /// How much the steering torque shakes (standard deviation, shares of full scale; in all,
    /// then lateral, longitudinal, vertical and drive) driving straight at 20 m/s on a road
    /// as rough as `roughness`, on part throttle or floored; and the torque's mean.
    fn shake(setup: CarSetup, roughness: f32, floored: bool) -> ([f32; 5], f32) {
        let terrain = Terrain::flat_road(roughness);
        let scale = full_scale(&setup);
        let mut car = Car::spawn_at(0, setup, &terrain, 0.0, 400.0, 0.0);
        car.warm_tires();
        car.parking_brake = false;
        car.drivetrain.automatic = true;
        let dt = 1.0 / CHASSIS_HZ as f32;
        let (mut sum, mut squares, mut n) = ([0.0; 5], [0.0; 5], 0.0);
        for step in 0..10 * CHASSIS_HZ as usize {
            let forward = car.forward_speed();
            let cruise = (0.3 + 0.5 * (20.0 - forward)).clamp(0.0, 1.0);
            let floor = floored && step >= 6 * CHASSIS_HZ as usize;
            car.controls.throttle = if floor { 1.0 } else { cruise };
            // Keeping it on the road: steer against heading off and drifting sideways.
            let heading = (car.body.rotation * Vec3::NEG_Z).x;
            car.controls.steer = (0.5 * car.body.position.x + 2.0 * heading).clamp(-0.3, 0.3);
            step_car(&mut car, &terrain, dt);
            if step >= 6 * CHASSIS_HZ as usize {
                let r = car.rack;
                let parts = [r.total(), r.lateral, r.longitudinal, r.vertical, r.drive];
                for (i, part) in parts.iter().enumerate() {
                    let level = part / scale;
                    sum[i] += level;
                    squares[i] += level * level;
                }
                n += 1.0;
            }
        }
        let deviation = |i: usize| (squares[i] / n - (sum[i] / n).powi(2)).max(0.0).sqrt();
        (std::array::from_fn(deviation), sum[0] / n)
    }

    /// Light and heavy cars pull the wheel alike near the limit (the hatch, and the hatch
    /// 900 kg heavier), and the pull grows with the cornering.
    #[test]
    fn light_and_heavy_cars_pull_alike() {
        let heavy = crate::catalogue::builtin_changed(|c| c.chassis[0].mass += 900.0);
        for (chassis, setup) in [CarSetup::factory(0, 0), CarSetup::factory_in(&heavy, 0, 0)]
            .into_iter()
            .enumerate()
        {
            let (mild, _) = cornering(setup.clone(), 0.3);
            let (hard, _) = cornering(setup, 0.7);
            // Turning left, the tires pull the wheels back to the right.
            assert!((-0.45..-0.2).contains(&mild), "{chassis}: {mild} at 0.3 g");
            assert!((-0.8..-0.45).contains(&hard), "{chassis}: {hard} at 0.7 g");
        }
    }

    /// The steering shakes on a rough road and hardly on a smooth one, and the front wheels
    /// putting the power down tug at it over the bumps (the hatch, front-wheel drive), unlike
    /// the rear ones (the hatch made to drive its rear wheels).
    #[test]
    fn bumps_and_driven_front_wheels_come_through() {
        let hatch = CarSetup::factory(0, 0);
        let (smooth, _) = shake(hatch.clone(), 16.0, false);
        let (rough, _) = shake(hatch.clone(), 256.0, false);
        let (floored, _) = shake(hatch, 256.0, true);
        assert!(rough[0] > 2.0 * smooth[0], "{rough:?} against {smooth:?}");
        assert!(floored[0] > 1.5 * rough[0], "{floored:?} against {rough:?}");
        assert!(floored[4] > 0.008, "the driveshafts: {floored:?}");
        let rear = crate::catalogue::builtin_changed(|c| c.chassis[0].drive = Drive::Rear);
        let rear = CarSetup::factory_in(&rear, 0, 0);
        let (rough, _) = shake(rear.clone(), 256.0, false);
        let (floored, _) = shake(rear, 256.0, true);
        assert!(floored[0] < 1.3 * rough[0], "{floored:?} against {rough:?}");
        assert_eq!(floored[4], 0.0);
    }

    /// Prints every car's steering torque in corners and on rough roads:
    /// `cargo test --profile release-dev steering_feel -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn steering_feel() {
        for (index, chassis) in crate::catalogue::builtin().chassis.iter().enumerate() {
            let setup = CarSetup::factory(index, 0);
            let drive = match setup.engine.drive {
                Drive::Front => "FWD",
                Drive::Rear => "RWD",
                Drive::All { .. } => "AWD",
            };
            let scale = full_scale(&setup);
            let (mild, mild_floored) = cornering(setup.clone(), 0.3);
            let (hard, floored) = cornering(setup.clone(), 0.7);
            let smooth = shake(setup.clone(), 16.0, false).0[0];
            let (_, pull) = shake(setup.clone(), 16.0, true);
            let rough = shake(setup.clone(), 256.0, false);
            let (power, _) = shake(setup, 256.0, true);
            let rough = rough.0;
            let f = |s: [f32; 5]| s.map(|x| format!("{x:.3}")).join(" ");
            println!(
                "{:10} {drive}: full scale {scale:3.0} N·m, 0.3 g {mild:5.2} floored {mild_floored:5.2}, 0.7 g {hard:5.2}, \
                 floored {floored:5.2}, pull {pull:5.2}, shake {smooth:.3} smooth, rough {}, floored {}",
                chassis.name,
                f(rough),
                f(power),
            );
        }
    }
}
