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
/// Distance the front tires' contact points trail the steering axes (from caster, m).
const MECHANICAL_TRAIL: f32 = 0.015;
/// Density of the air (kg/m³).
const AIR_DENSITY: f32 = 1.2;
/// Rolling resistance of the tires: the share of their load that holds them back.
const ROLLING_RESISTANCE: f32 = 0.012;

pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(CHASSIS_HZ))
            .add_systems(FixedUpdate, simulate);
    }
}

pub fn simulate(time: Res<Time>, terrain: Res<Terrain>, mut cars: Query<&mut Car>) {
    for mut car in &mut cars {
        step_car(&mut car, &terrain, time.delta_secs());
    }
}

pub fn step_car(car: &mut Car, terrain: &Terrain, dt: f32) {
    apply_controls(car, dt);

    let Car {
        setup,
        body,
        corners,
        steering_torque,
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
    // The front tires' lateral forces act behind the steering axes, turning the wheels
    // towards where they're rolling: self-aligning torque.
    *steering_torque = corners
        .iter()
        .filter(|corner| corner.front)
        .filter_map(|corner| corner.contact)
        .map(|contact| (setup.front.tire.trail(&contact.tire) + MECHANICAL_TRAIL) * contact.tire.fy)
        .sum();
    // The air: drag against the motion, and downforce over each axle, both with the
    // square of the speed.
    let pressure = 0.5 * AIR_DENSITY * body.velocity.length_squared();
    let drag = -body.velocity.normalize_or_zero() * pressure * setup.drag_area;
    let down = body.rotation * Vec3::NEG_Y;
    for (area, z) in [
        (setup.downforce_area[0], setup.front_z),
        (setup.downforce_area[1], setup.rear_z),
    ] {
        let push = down * pressure * area;
        force += push * SUBSTEPS as f32;
        torque += (body.rotation * Vec3::new(0.0, 0.0, z)).cross(push) * SUBSTEPS as f32;
    }
    body.add_force(force / SUBSTEPS as f32 + GRAVITY * body.mass + drag);
    body.add_torque(torque / SUBSTEPS as f32);
    body.integrate(dt);
}

/// Engine torque to the rear wheels (split evenly, like an open differential), biased
/// brakes, rear handbrake, and steering.
fn apply_controls(car: &mut Car, dt: f32) {
    car.drivetrain.engine.airspeed = car.forward_speed();
    let Car {
        setup,
        corners,
        drivetrain,
        controls: c,
        ..
    } = car;

    let rear_omega = (corners[2].omega + corners[3].omega) / 2.0;
    let grounded = corners[2].contact.is_some() && corners[3].contact.is_some();
    let drive = drivetrain.step(
        &setup.engine,
        c.throttle,
        c.clutch,
        rear_omega,
        grounded,
        dt,
    );
    let steer = c.steer * setup.max_steer;
    for corner in corners.iter_mut() {
        // Rolling resistance holds each wheel back like a light brake.
        let tire = &setup.axle(corner.front).tire;
        let rolling = ROLLING_RESISTANCE * corner.contact.map_or(0.0, |c| c.load) * tire.radius;
        if corner.front {
            corner.steer = steer;
            corner.brake_torque = rolling + c.brake * setup.brake_torque * setup.brake_bias / 2.0;
        } else {
            corner.drive_torque = drive.torque / 2.0;
            corner.drive_inertia = drive.inertia / 2.0;
            corner.brake_torque = rolling
                + c.brake * setup.brake_torque * (1.0 - setup.brake_bias) / 2.0
                + c.handbrake * setup.handbrake_torque;
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
            handbrake: 1.0,
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
        // About 13 m/s on the flat; the start road in Gerach climbs at around 7 %.
        assert!(speed > 9.0, "speed {speed}");

        car.controls = Controls {
            throttle: 0.3,
            steer: 0.7,
            ..default()
        };
        let before = heading(car.body.rotation);
        run(&mut car, &terrain, 1.0);
        // The tires pull the steering back towards straight ahead.
        assert!(
            car.steering_torque < -10.0,
            "steering torque {}",
            car.steering_torque
        );
        // Positive steer turns left, i.e. increases the heading angle (wrapped to ±π).
        let turned = (heading(car.body.rotation) - before + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        assert!(turned > 0.3, "turned {turned}");
        let up = car.body.rotation * Vec3::Y;
        assert!(up.y > 0.9, "rolled over");
    }

    #[test]
    fn the_air_limits_the_top_speed() {
        let terrain = Terrain::baked();
        let mut car = test_car(&terrain);
        run(&mut car, &terrain, 1.0);
        // Coasting in neutral from 180 km/h: drag and the tires slow the car by about
        // 1 m/s² (0.62 m² of drag area on 1260 kg), and so does the climbing road.
        car.drivetrain.shift_to(&car.setup.engine, 0);
        car.body.velocity = car.body.rotation * Vec3::NEG_Z * 50.0;
        let radius = car.setup.front.tire.radius;
        for corner in car.corners.iter_mut() {
            corner.omega = 50.0 / radius;
        }
        run(&mut car, &terrain, 0.5);
        let lost = 50.0 - car.body.velocity.length();
        assert!((0.3..1.2).contains(&lost), "lost {lost} m/s");
    }

    #[test]
    fn automatic_shifts_up_without_hunting() {
        let terrain = Terrain::baked();
        let mut car = test_car(&terrain);
        car.drivetrain.automatic = true;
        run(&mut car, &terrain, 1.0);
        car.controls = Controls {
            throttle: 1.0,
            ..default()
        };
        // Flat out from a standstill: the gears only ever go up (the clutch closing after
        // a shift spins the wheels up for a moment, which mustn't trigger another shift).
        // Unsteered, the car leaves the road after a few seconds and crosses fields; much
        // longer and it spins out, after which shifting down is right.
        let dt = 1.0 / CHASSIS_HZ as f32;
        let mut gears = vec![car.drivetrain.selected_gear()];
        for _ in 0..(10.0 / dt) as usize {
            step_car(&mut car, &terrain, dt);
            let gear = car.drivetrain.selected_gear();
            if gear != *gears.last().unwrap() {
                gears.push(gear);
            }
        }
        assert!(gears.windows(2).all(|w| w[1] > w[0]), "gears {gears:?}");
        assert!(gears.len() >= 3, "gears {gears:?}");
    }

    #[test]
    fn brakes_without_rear_lock_or_spinning() {
        let terrain = Terrain::baked();
        let (p, yaw) = terrain.road_spawns(1, 0.0)[0];
        for setup in [CarSetup::baseline(), CarSetup::soft(), CarSetup::track()] {
            let name = setup.name;
            let mut car = Car::spawn_at(0, setup, &terrain, p.x, p.y, yaw);
            // In fourth, as a driver would be at this speed.
            car.drivetrain.shift_to(&car.setup.engine, 4);
            run(&mut car, &terrain, 1.0);
            // Full brakes from 90 km/h, with a touch of steering to upset the car.
            car.body.velocity = car.body.rotation * Vec3::NEG_Z * 25.0;
            let radius = car.setup.front.tire.radius;
            for corner in car.corners.iter_mut() {
                corner.omega = 25.0 / radius;
            }
            car.controls = Controls {
                brake: 1.0,
                steer: 0.05,
                ..default()
            };
            let dt = 1.0 / CHASSIS_HZ as f32;
            let mut time = 0.0;
            while car.body.velocity.length() > 3.0 {
                step_car(&mut car, &terrain, dt);
                time += dt;
                assert!(time < 4.0, "{name}: not stopping");
                // Locked rear wheels lose their side grip and let the car spin; a locked
                // front wheel only makes it push wide.
                for corner in car.corners.iter().filter(|c| !c.front) {
                    assert!(
                        corner.omega.abs() > 1.0,
                        "{name}: rear wheel locked at {time:.2} s"
                    );
                }
                let yaw_rate = car.body.angular_velocity.y.abs();
                assert!(
                    yaw_rate < 0.3,
                    "{name}: yaw rate {yaw_rate:.2} at {time:.2} s"
                );
            }
        }
    }
}
