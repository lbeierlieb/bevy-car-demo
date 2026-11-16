//! Handling tests on flat ground with a simple driver: how a car slides, and whether the
//! slide can be caught.

use bevy::prelude::*;

use super::{CHASSIS_HZ, step_car};
use crate::car::{Car, CarSetup};
use crate::terrain::Terrain;

/// The car's body slip angle (rad; positive when it moves to the left of where it points)
/// and yaw rate (rad/s, positive turning left).
fn slip_and_yaw(car: &Car) -> (f32, f32) {
    let rotation = car.body.rotation;
    let velocity = car.body.velocity;
    let forward = velocity.dot(rotation * Vec3::NEG_Z);
    let left = velocity.dot(rotation * Vec3::NEG_X);
    (left.atan2(forward.max(0.5)), car.body.angular_velocity.y)
}

/// The direction the front axle moves in, relative to where the car points (rad).
fn front_axle_direction(car: &Car) -> f32 {
    let rotation = car.body.rotation;
    let velocity = car.body.velocity;
    let forward = velocity.dot(rotation * Vec3::NEG_Z);
    let left = velocity.dot(rotation * Vec3::NEG_X);
    // The front axle is ahead of the centre of mass (-z), so turning left moves it left.
    let ahead = -car.setup.front_z;
    (left + ahead * car.body.angular_velocity.y).atan2(forward.max(0.5))
}

#[derive(Debug)]
struct Slide {
    /// Deepest body slip angle in the slide (degrees, positive towards the outside of the
    /// turn it started in).
    deepest: f32,
    /// Largest body slip angle the other way afterwards: the snap back (degrees).
    snap_back: f32,
    /// Body slip angle and yaw rate at the end (degrees, degrees per second).
    end_slip: f32,
    end_yaw: f32,
    /// How far traction control moved the throttle in all, up and down (shares of the
    /// pedal): the more it hunts, the more.
    traction_travel: f32,
}

impl Slide {
    fn spun(&self) -> bool {
        self.deepest > 75.0
    }
}

/// A driver going round a left-hand circle in second gear on warm tires, then flooring it
/// until the rear steps out, and catching the slide a human's reaction time later by
/// pointing the front wheels where the front axle goes, at a human's steering speed,
/// easing the throttle.
fn power_slide(setup: CarSetup, traction_control: bool) -> Slide {
    let terrain = Terrain::flat();
    let mut car = Car::spawn_at(0, setup, &terrain, 0.0, 0.0, 0.0);
    car.warm_tires();
    // Getting up to speed with traction control, so the strong ones don't do a burnout.
    car.traction_control = true;
    car.drivetrain.gear = 2;
    let dt = 1.0 / CHASSIS_HZ as f32;
    let max_steer = car.setup.max_steer;
    // Seconds before the driver notices, and how fast the wheel turns (fraction of lock
    // per second: half the 540° range in about a third of a second).
    const REACTION: f32 = 0.15;
    const STEER_RATE: f32 = 3.0;
    /// Body slip angle where the driver notices the slide (rad).
    const NOTICE: f32 = 0.1;
    /// How far the driver steers against the yaw rate, on top of pointing the front wheels
    /// where the front axle goes (rad per rad/s): unwinding before the car swings back.
    const YAW_DAMPING: f32 = 0.05;
    let delay = (REACTION / dt) as usize;
    // What the driver saw: where the front axle went, and the yaw rate.
    let mut seen: Vec<(f32, f32)> = Vec::new();

    let circle_steer = 0.15;
    let speed = 13.0;
    let mut t = 0.0;
    let mut noticed: Option<f32> = None;
    let mut deepest = 0.0f32;
    let mut snap_back = 0.0f32;
    let trace = std::env::var("SLIDE_TRACE").is_ok();
    let mut traction_travel = 0.0;
    while t < 13.0 {
        let (slip, yaw) = slip_and_yaw(&car);
        let forward = car.forward_speed();
        seen.push((front_axle_direction(&car), yaw));
        if t >= 6.0 {
            car.traction_control = traction_control;
        }
        let c = &mut car.controls;
        if t < 6.0 {
            // Round the circle at a steady speed.
            c.steer = circle_steer;
            c.throttle = (0.3 + 0.5 * (speed - forward)).clamp(0.0, 1.0);
        } else {
            if noticed.is_none() && -slip > NOTICE {
                noticed = Some(t);
            }
            match noticed {
                Some(at) if t >= at + REACTION => {
                    // Point the front wheels where the front axle went a reaction time ago,
                    // less a little against the yaw.
                    let (direction, yaw) = seen[seen.len().saturating_sub(delay + 1)];
                    let target = (direction - YAW_DAMPING * yaw) / max_steer;
                    let step = STEER_RATE * dt;
                    c.steer += (target.clamp(-1.0, 1.0) - c.steer).clamp(-step, step);
                    // Lifting the further the car is sideways, off at about 30°.
                    c.throttle = (0.4 * (1.0 - slip.abs() / 0.5)).max(0.0);
                }
                _ => c.throttle = 1.0,
            }
            if noticed.is_some() {
                deepest = deepest.max(-slip);
                if deepest > 0.0 && -slip < deepest * 0.5 {
                    snap_back = snap_back.max(slip);
                }
            }
        }
        if trace && t > 5.8 && ((t * CHASSIS_HZ as f32) as usize).is_multiple_of(6) {
            print_state(&car, t);
        }
        let traction = car.traction;
        step_car(&mut car, &terrain, dt);
        traction_travel += (car.traction - traction).abs();
        t += dt;
    }
    let (slip, yaw) = slip_and_yaw(&car);
    Slide {
        deepest: deepest.to_degrees(),
        snap_back: snap_back.to_degrees(),
        end_slip: slip.to_degrees(),
        end_yaw: yaw.to_degrees(),
        traction_travel,
    }
}

/// One line of what the car and its tires do (slip ratio, slip angle in degrees, load).
fn print_state(car: &Car, t: f32) {
    let (slip, yaw) = slip_and_yaw(car);
    let tires: Vec<String> = car
        .corners
        .iter()
        .map(|corner| {
            corner.contact.map_or("in the air".into(), |c| {
                let (ratio, angle) = (c.tire.slip_ratio, c.tire.slip_angle.to_degrees());
                format!("{ratio:5.2} {angle:5.1}° {:4.0} N", c.load)
            })
        })
        .collect();
    println!(
        "{t:5.2} s  slip {:5.1}°  yaw {:6.1}°/s  {:4.1} m/s  steer {:5.2} ({:4.0} N·m)  throttle {:.1} | {}",
        slip.to_degrees(),
        yaw.to_degrees(),
        car.body.velocity.length(),
        car.controls.steer,
        car.steering_torque,
        car.controls.throttle,
        tires.join(" | "),
    );
}

/// Front-engined rear-wheel-drive cars, stock: a slide on the power can be caught without
/// spinning or snapping round the other way.
#[test]
fn a_power_slide_can_be_caught() {
    // Tarn W20, Halden S30, Harrow V35.
    for (chassis, option) in [(3, 0), (4, 1), (6, 2)] {
        let slide = power_slide(CarSetup::factory(chassis, option), false);
        assert!(slide.deepest > 8.0, "{chassis}: no slide: {slide:?}");
        assert!(!slide.spun() && slide.snap_back < 30.0, "{chassis}: {slide:?}");
        assert!(slide.end_slip.abs() < 2.0 && slide.end_yaw.abs() < 3.0, "{chassis}: {slide:?}");
    }
}

/// Prints how each car's power slide goes, without and with traction control, for tuning
/// the tires: `cargo test --profile release-dev power_slides -- --ignored --nocapture`.
#[test]
#[ignore]
fn power_slides() {
    use crate::chassis::CHASSIS;
    for (chassis, made) in CHASSIS.iter().enumerate() {
        for option in 0..made.engines.len() {
            for traction_control in [false, true] {
                let slide = power_slide(CarSetup::factory(chassis, option), traction_control);
                println!(
                    "{:<10} {:<6} {} deepest {:5.1}°  snap back {:5.1}°  end {:5.1}° {:6.1}°/s  \
                     TC travel {:5.2}{}",
                    made.name,
                    made.engines[option],
                    if traction_control { "TC" } else { "  " },
                    slide.deepest,
                    slide.snap_back,
                    slide.end_slip,
                    slide.end_yaw,
                    slide.traction_travel,
                    if slide.spun() { "  SPUN" } else { "" },
                );
            }
        }
    }
}

/// Prints one car's power slide step by step: `SLIDE_CAR=chassis,engine SLIDE_TRACE=1
/// cargo test --profile release-dev trace_power_slide -- --ignored --nocapture`.
#[test]
#[ignore]
fn trace_power_slide() {
    let spec = std::env::var("SLIDE_CAR").unwrap_or("3,0".into());
    let [chassis, option] = [0, 1].map(|i| spec.split(',').nth(i).unwrap().parse().unwrap());
    println!("{:?}", power_slide(CarSetup::factory(chassis, option), false));
}

/// A car going round the left-hand circle at `speed` (m/s) for `seconds`, calling `each`
/// every simulated second.
fn circle(setup: CarSetup, speed: f32, seconds: usize, mut each: impl FnMut(usize, &Car)) {
    let terrain = Terrain::flat();
    let mut car = Car::spawn_at(0, setup, &terrain, 0.0, 0.0, 0.0);
    car.drivetrain.gear = 2;
    let dt = 1.0 / CHASSIS_HZ as f32;
    for second in 1..=seconds {
        for _ in 0..CHASSIS_HZ as usize {
            let forward = car.forward_speed();
            car.controls.steer = 0.15;
            car.controls.throttle = (0.3 + 0.5 * (speed - forward)).clamp(0.0, 1.0);
            step_car(&mut car, &terrain, dt);
        }
        each(second, &car);
    }
}

/// Prints how warm and worn the tires get going round a circle at three speeds, for each
/// compound: `cargo test --profile release-dev tire_temperatures -- --ignored --nocapture`.
#[test]
#[ignore]
fn tire_temperatures() {
    use crate::chassis::CarBuild;
    use crate::physics::tire::COMPOUNDS;
    for (tires, compound) in COMPOUNDS.iter().enumerate() {
        for speed in [8.0, 14.0, 17.0, 19.0] {
            let factory = CarSetup::factory(3, 0);
            let build = CarBuild {
                tires,
                ..factory.build.clone()
            };
            let setup = CarSetup::new(build, factory.engine.parts, factory.engine.build);
            circle(setup, speed, 120, |second, car| {
                if [10, 30, 60, 120].contains(&second) {
                    let corners: Vec<String> = car
                        .corners
                        .iter()
                        .map(|corner| {
                            let tire = &car.setup.axle(corner.front).tire;
                            format!(
                                "{:4.0}°C {:4.1}% {:3.0}%",
                                corner.tread.temperature,
                                corner.tread.wear * 100.0,
                                tire.condition(&corner.tread) * 100.0,
                            )
                        })
                        .collect();
                    let (slip, _) = slip_and_yaw(car);
                    println!(
                        "{:<6} {speed:4.1} m/s {second:3} s  slip {:5.1}°  {:4.1} m/s | {}",
                        compound.name,
                        slip.to_degrees(),
                        car.forward_speed(),
                        corners.join(" | ")
                    );
                }
            });
        }
    }
}
