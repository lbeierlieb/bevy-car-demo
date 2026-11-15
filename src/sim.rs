//! The multi-rate simulation loop.
//!
//! The chassis integrates at `CHASSIS_HZ`. Within each chassis step, every corner runs
//! `SUBSTEPS` steps of its own against the chassis moving at constant velocity, and the
//! average force it exerts is applied to the chassis.

use bevy::prelude::*;

use crate::car::Car;
use crate::physics::suspension::ChassisFrame;
use crate::terrain::Terrain;

pub const CHASSIS_HZ: f64 = 120.0;
pub const SUBSTEPS: usize = 10;
const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);

pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(CHASSIS_HZ))
            .add_systems(FixedUpdate, simulate);
    }
}

fn simulate(time: Res<Time>, terrain: Res<Terrain>, mut cars: Query<&mut Car>) {
    for mut car in &mut cars {
        step_car(&mut car, &terrain, time.delta_secs());
    }
}

pub fn step_car(car: &mut Car, terrain: &Terrain, dt: f32) {
    apply_controls(car);

    let Car {
        setup,
        body,
        corners,
        ..
    } = car;
    let corner_mass = setup.sprung_mass / 4.0;
    let h = dt / SUBSTEPS as f32;
    let (mut force, mut torque) = (Vec3::ZERO, Vec3::ZERO);
    for corner in corners.iter_mut() {
        let axle = setup.axle(corner.front);
        let rest_length = axle.spring_rest_length(corner_mass);
        for k in 0..SUBSTEPS {
            let (position, rotation) = body.extrapolate(k as f32 * h);
            let frame = ChassisFrame {
                position,
                rotation,
                velocity: body.velocity,
                angular_velocity: body.angular_velocity,
            };
            corner.step(
                h,
                axle,
                rest_length,
                &frame,
                terrain,
                &mut force,
                &mut torque,
            );
        }
    }
    body.add_force(force / SUBSTEPS as f32 + GRAVITY * body.mass);
    body.add_torque(torque / SUBSTEPS as f32);
    body.integrate(dt);
}

/// Minimal drivetrain: power-limited torque split evenly over the rear wheels, 60/40 brakes,
/// rear handbrake, and speed-sensitive steering.
fn apply_controls(car: &mut Car) {
    let speed = car.forward_speed().abs();
    let Car {
        setup,
        corners,
        controls: c,
        ..
    } = car;

    let rear_omega = (corners[2].omega.abs() + corners[3].omega.abs()) / 2.0;
    let torque = setup
        .drive_torque
        .min(setup.drive_power / rear_omega.max(1.0))
        * c.throttle;
    let steer = c.steer * setup.max_steer / (1.0 + speed * 0.04);
    for corner in corners.iter_mut() {
        if corner.front {
            corner.steer = steer;
            corner.drive_torque = 0.0;
            corner.brake_torque = c.brake * setup.brake_torque * 0.3;
        } else {
            corner.steer = 0.0;
            corner.drive_torque = torque / 2.0;
            corner.brake_torque = c.brake * setup.brake_torque * 0.2
                + if c.handbrake {
                    setup.handbrake_torque
                } else {
                    0.0
                };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::{CarSetup, Controls, heading};

    fn run(car: &mut Car, terrain: &Terrain, seconds: f32) {
        let dt = 1.0 / CHASSIS_HZ as f32;
        for _ in 0..(seconds / dt) as usize {
            step_car(car, terrain, dt);
        }
    }

    /// A car on the road at the first spawn point, pointing along the road.
    fn test_car(terrain: &Terrain) -> Car {
        let (p, yaw) = terrain.road_spawns(1, 0.0)[0];
        Car::spawn_at(0, CarSetup::baseline(), terrain, p.x, p.y, yaw)
    }

    #[test]
    fn settles_at_design_ride_height() {
        let terrain = Terrain::baked();
        let mut car = test_car(&terrain);
        // Brakes on: the road isn't level.
        car.controls = Controls {
            brake: 1.0,
            handbrake: true,
            ..default()
        };
        run(&mut car, &terrain, 4.0);
        assert!(
            car.body.velocity.length() < 0.05,
            "still moving: {}",
            car.body.velocity
        );
        for corner in &car.corners {
            // Within a few cm of design height (tire compression, slope weight transfer).
            assert!(corner.travel.abs() < 0.04, "travel {}", corner.travel);
        }
    }

    #[test]
    fn accelerates_and_turns() {
        let terrain = Terrain::baked();
        let mut car = test_car(&terrain);
        run(&mut car, &terrain, 1.0);
        car.controls = Controls {
            throttle: 1.0,
            ..default()
        };
        run(&mut car, &terrain, 3.0);
        let speed = car.forward_speed();
        assert!(speed > 12.0, "speed {speed}");

        car.controls = Controls {
            throttle: 0.3,
            steer: 1.0,
            ..default()
        };
        let before = heading(car.body.rotation);
        run(&mut car, &terrain, 1.0);
        // Positive steer turns left, i.e. increases the heading angle (wrapped to ±π).
        let turned = (heading(car.body.rotation) - before + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        assert!(turned > 0.3, "turned {turned}");
        let up = car.body.rotation * Vec3::Y;
        assert!(up.y > 0.9, "rolled over");
    }
}
