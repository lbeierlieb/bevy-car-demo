//! The multi-rate simulation loop.
//!
//! The chassis integrates at `CHASSIS_HZ`. Within each chassis step, every corner runs
//! `SUBSTEPS` steps of its own against the chassis moving at constant velocity, and the
//! average force it exerts is applied to the chassis.

use bevy::prelude::*;

use crate::car::Car;
use crate::physics::drivetrain::Drive;
use crate::physics::suspension::ChassisFrame;
use crate::terrain::Terrain;

pub const CHASSIS_HZ: f64 = 120.0;
pub const SUBSTEPS: usize = 10;
const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);
/// Distance the front tires' contact points trail the steering axes (from caster, m).
/// Past the peak slip angle the pneumatic trail is gone and this is all that keeps
/// the wheel pulling towards where the front tires roll: into countersteer in a slide.
const MECHANICAL_TRAIL: f32 = 0.03;
/// Density of the air (kg/m³).
const AIR_DENSITY: f32 = 1.2;
/// Rolling resistance of the tires: the share of their load that holds them back.
const ROLLING_RESISTANCE: f32 = 0.012;
/// Traction control: the slip ratio of a driven wheel it steps in at (a little past the
/// tires' best), how fast it closes the throttle at that slip and opens it again (shares
/// of the pedal per second), and the least it leaves.
const TRACTION_SLIP: f32 = 0.14;
const TRACTION_CUT: f32 = 5.0;
const TRACTION_RESTORE: f32 = 2.0;
const TRACTION_LEAST: f32 = 0.08;

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
    let h = dt / SUBSTEPS as f32;
    let (mut force, mut torque) = (Vec3::ZERO, Vec3::ZERO);
    for corner in corners.iter_mut() {
        let axle = setup.axle(corner.front);
        let rest_length = axle.spring_rest_length(setup.corner_mass(corner.front));
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

/// Engine torque to the driven wheels through the differentials (less what traction
/// control holds back), biased brakes, rear handbrake, and steering.
fn apply_controls(car: &mut Car, dt: f32) {
    car.drivetrain.engine.airspeed = car.forward_speed();
    let Car {
        setup,
        corners,
        drivetrain,
        controls: c,
        traction_control,
        traction,
        ..
    } = car;

    let engine = &setup.engine;
    let shares = engine.drive.shares();
    // Traction control eases the throttle while a driven wheel spins.
    let slip = (0..4)
        .filter(|&i| shares[i] > 0.0)
        .filter_map(|i| corners[i].contact)
        .map(|contact| contact.tire.slip_ratio)
        .fold(0.0, f32::max);
    let rate = if slip > TRACTION_SLIP {
        -TRACTION_CUT * slip / TRACTION_SLIP
    } else {
        TRACTION_RESTORE
    };
    *traction = if *traction_control {
        (*traction + rate * dt).clamp(TRACTION_LEAST, 1.0)
    } else {
        1.0
    };
    let throttle = c.throttle * *traction;
    let omega = [0, 1, 2, 3].map(|i| corners[i].omega);
    let driven_omega = (0..4).map(|i| shares[i] * omega[i]).sum();
    let grounded = (0..4).all(|i| shares[i] == 0.0 || corners[i].contact.is_some());
    let drive = drivetrain.step(engine, throttle, c.clutch, driven_omega, grounded, dt);
    let mut torque = shares.map(|share| share * drive.torque);
    // Limited-slip differentials move torque from the faster side to the slower one.
    for (left, right) in [(0, 1), (2, 3)] {
        if shares[left] > 0.0 {
            let moved = engine.differential.transfer(omega[left], omega[right]);
            torque[left] -= moved;
            torque[right] += moved;
        }
    }
    if matches!(engine.drive, Drive::All { .. }) {
        let (front, rear) = ((omega[0] + omega[1]) / 2.0, (omega[2] + omega[3]) / 2.0);
        let moved = engine.centre.transfer(front, rear) / 2.0;
        for (i, torque) in torque.iter_mut().enumerate() {
            *torque += if i < 2 { -moved } else { moved };
        }
    }
    let steer = c.steer * setup.max_steer;
    for (i, corner) in corners.iter_mut().enumerate() {
        // Rolling resistance holds each wheel back like a light brake.
        let tire = &setup.axle(corner.front).tire;
        let rolling = ROLLING_RESISTANCE * corner.contact.map_or(0.0, |c| c.load) * tire.radius;
        corner.drive_torque = torque[i];
        corner.drive_inertia = shares[i] * drive.inertia;
        if corner.front {
            corner.steer = steer;
            corner.brake_torque = rolling + c.brake * setup.brake_torque * setup.brake_bias / 2.0;
        } else {
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
    use crate::chassis::{CHASSIS, CarBuild, SUSPENSIONS};

    fn run(car: &mut Car, terrain: &Terrain, seconds: f32) {
        let dt = 1.0 / CHASSIS_HZ as f32;
        for _ in 0..(seconds / dt) as usize {
            step_car(car, terrain, dt);
        }
    }

    /// A car on the road at the first spawn point, pointing along the road.
    fn test_car(terrain: &Terrain) -> Car {
        let (p, yaw) = terrain.road_spawns(1, 0.0)[0];
        Car::spawn_at(0, CarSetup::default(), terrain, p.x, p.y, yaw)
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
    fn every_car_stands_on_its_wheels_and_pulls_away() {
        let terrain = Terrain::baked();
        let (p, yaw) = terrain.road_spawns(1, 0.0)[0];
        for (index, chassis) in CHASSIS.iter().enumerate() {
            for option in 0..chassis.engines.len() {
                let setup = CarSetup::factory(index, option);
                let name = format!("{} {}", chassis.name, chassis.engines[option]);
                // The engine's place shows in the balance.
                let front = setup.front_weight();
                let expected = match chassis.bay.place {
                    "middle" => 0.38..0.47,
                    "rear" => 0.34..0.42,
                    _ => 0.46..0.66,
                };
                assert!(
                    expected.contains(&front),
                    "{name}: {front} on the front axle"
                );
                let mass = setup.mass();
                let mut car = Car::spawn_at(0, setup, &terrain, p.x, p.y, yaw);
                car.controls = Controls {
                    brake: 1.0,
                    handbrake: 1.0,
                    ..default()
                };
                run(&mut car, &terrain, 3.0);
                assert!(car.body.velocity.length() < 0.05, "{name}: still moving");
                for corner in &car.corners {
                    assert!(
                        corner.travel.abs() < 0.04,
                        "{name}: travel {}",
                        corner.travel
                    );
                }
                // The body's lowest point, the floor, is clear of the road.
                let ground = terrain.sample(car.body.position.x, car.body.position.z).0;
                let height = car.body.position.y - ground;
                assert!(
                    (height - chassis.cg_height).abs() < 0.05,
                    "{name}: centre of mass {height} m up"
                );
                // Flat out with the automatic gearbox.
                car.drivetrain.automatic = true;
                car.controls = Controls {
                    throttle: 1.0,
                    ..default()
                };
                // Unsteered, the quick ones are off the road soon after.
                run(&mut car, &terrain, 3.0);
                let speed = car.forward_speed();
                println!(
                    "{name}: {mass:.0} kg, {:.0} % front, {speed:.1} m/s after 3 s",
                    front * 100.0
                );
                assert!(speed > 6.0, "{name}: {speed} m/s");
                assert!((car.body.rotation * Vec3::Y).y > 0.9, "{name}: rolled over");
            }
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
    fn front_and_all_wheel_drive_pull_with_their_wheels() {
        use crate::physics::drivetrain::Differential;
        let terrain = Terrain::baked();
        let launch = |drive: Drive, differential: Differential| {
            let mut car = test_car(&terrain);
            car.setup.engine.drive = drive;
            car.setup.engine.differential = differential;
            car.setup.engine.centre = differential;
            run(&mut car, &terrain, 1.0);
            car.controls = Controls {
                throttle: 1.0,
                ..default()
            };
            // Forces along the car from the front and the rear tires, while pulling away.
            run(&mut car, &terrain, 1.0);
            let pull = |front: bool| -> f32 {
                let corners = car.corners.iter().filter(|c| c.front == front);
                corners.filter_map(|c| c.contact).map(|c| c.tire.fx).sum()
            };
            let pulls = (pull(true), pull(false));
            run(&mut car, &terrain, 2.0);
            (pulls, car.forward_speed())
        };
        let ((front, rear), speed) = launch(Drive::Front, Differential::OPEN);
        assert!(front > 2000.0 && rear < 0.0, "front {front}, rear {rear}");
        assert!(speed > 8.0, "front-wheel drive: {speed} m/s");
        let ((front, rear), all) = launch(Drive::All { front: 0.4 }, Differential::LIMITED_SLIP);
        assert!(front > 1000.0 && rear > front, "front {front}, rear {rear}");
        // Four wheels put more down than two.
        assert!(all > speed, "all-wheel drive {all} m/s, front {speed} m/s");
    }

    #[test]
    fn traction_control_keeps_a_big_engine_from_spinning_its_wheels() {
        let terrain = Terrain::baked();
        let (p, yaw) = terrain.road_spawns(1, 0.0)[0];
        // The muscle car with the 6.2 V8, floored in first.
        let launch = |traction_control: bool| {
            let mut car = Car::spawn_at(0, CarSetup::factory(6, 1), &terrain, p.x, p.y, yaw);
            car.traction_control = traction_control;
            run(&mut car, &terrain, 1.0);
            car.controls = Controls {
                throttle: 1.0,
                ..default()
            };
            run(&mut car, &terrain, 2.5);
            let radius = car.setup.rear.tire.radius;
            let wheels = (car.corners[2].omega + car.corners[3].omega) / 2.0 * radius;
            (car.forward_speed(), wheels)
        };
        let (speed, wheels) = launch(true);
        assert!(
            wheels < 1.25 * speed,
            "wheels at {wheels} m/s, car at {speed}"
        );
        let (spinning, wheels) = launch(false);
        assert!(
            wheels > 1.5 * spinning,
            "wheels at {wheels} m/s, car at {spinning}"
        );
        assert!(speed > spinning + 3.0, "{speed} against {spinning} m/s");
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
        // Unsteered, the car leaves the road after a few seconds and crosses fields, where
        // it slows down and shifting down is right.
        let dt = 1.0 / CHASSIS_HZ as f32;
        let mut gears = vec![car.drivetrain.selected_gear()];
        for _ in 0..(6.0 / dt) as usize {
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
        // Every car as it leaves the factory, and on the track suspension and tires.
        let factory = (0..CHASSIS.len()).map(|chassis| CarSetup::factory(chassis, 0));
        let track = (0..CHASSIS.len()).map(|chassis| {
            let stock = CarSetup::factory(chassis, 0);
            let build = CarBuild {
                suspension: 2,
                tires: 2,
                ..stock.build
            };
            CarSetup::new(build, stock.engine.parts, stock.engine.build)
        });
        for setup in factory.chain(track) {
            let name = format!("{} ({})", setup.name, SUSPENSIONS[setup.build.suspension].0);
            let mut car = Car::spawn_at(0, setup, &terrain, p.x, p.y, yaw);
            // In fourth, as a driver would be at this speed.
            car.drivetrain.shift_to(&car.setup.engine, 4);
            run(&mut car, &terrain, 1.0);
            // Full brakes from 90 km/h, with a touch of steering to upset the car.
            car.body.velocity = car.body.rotation * Vec3::NEG_Z * 25.0;
            for corner in car.corners.iter_mut() {
                corner.omega = 25.0 / car.setup.axle(corner.front).tire.radius;
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
                // It turns no faster than the steering asks for at the speed it started
                // from (and a quarter, for the weight shifting forward).
                let wheelbase = car.setup.rear_z - car.setup.front_z;
                let asked = 25.0 * 0.05 * car.setup.max_steer / wheelbase;
                let yaw_rate = car.body.angular_velocity.y.abs();
                assert!(
                    yaw_rate < 1.25 * asked,
                    "{name}: yaw rate {yaw_rate:.2} at {time:.2} s"
                );
            }
        }
    }
}

#[cfg(test)]
mod handling;
