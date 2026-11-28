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

/// A driver going round a left-hand circle in `gear` on warm tires, then flooring it
/// until the rear steps out, and catching the slide a human's reaction time later by
/// pointing the front wheels where the front axle goes, at a human's steering speed,
/// easing the throttle.
fn power_slide(setup: CarSetup, traction_control: bool, gear: i8) -> Slide {
    let terrain = Terrain::flat();
    let mut car = Car::spawn_at(0, setup, &terrain, 0.0, 0.0, 0.0);
    car.warm_tires();
    // Getting up to speed with traction control, so the strong ones don't do a burnout.
    car.traction_control = true;
    car.drivetrain.gear = gear;
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
        car.rack.total(),
        car.controls.throttle,
        tires.join(" | "),
    );
}

/// The Merrow's V10 (630 N·m at the rear) floored round the circle in second and in third
/// with traction control: the rear steps out a few degrees and the driver catches it.
/// Without traction control this driver spins it in both gears (the rear
/// is at 40° before full opposite lock), and in fourth it doesn't slide at all; see
/// `power_slides`.
#[test]
fn a_power_slide_can_be_caught() {
    let merrow = crate::catalogue::builtin()
        .find_chassis("Merrow")
        .expect("the roadster");
    for gear in [2, 3] {
        let slide = power_slide(CarSetup::factory(merrow, 0), true, gear);
        assert!(slide.deepest > 3.0, "gear {gear}: no slide: {slide:?}");
        assert!(
            !slide.spun() && slide.snap_back < 15.0,
            "gear {gear}: {slide:?}"
        );
        assert!(
            slide.end_slip.abs() < 2.0 && slide.end_yaw.abs() < 3.0,
            "gear {gear}: {slide:?}"
        );
    }
}

/// Prints how each car's power slide goes, without and with traction control, for tuning
/// the tires: `cargo test --profile release-dev power_slides -- --ignored --nocapture`.
#[test]
#[ignore]
fn power_slides() {
    for (chassis, made) in crate::catalogue::builtin().chassis.iter().enumerate() {
        for option in 0..made.engines.len() {
            for (gear, traction_control) in [(2, false), (2, true), (3, false), (3, true)] {
                let setup = CarSetup::factory(chassis, option);
                let slide = power_slide(setup, traction_control, gear);
                println!(
                    "{:<10} {:<6} gear {gear} {} deepest {:5.1}°  snap back {:5.1}°  end {:5.1}° {:6.1}°/s  \
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
    let spec = std::env::var("SLIDE_CAR").unwrap_or("0,0".into());
    let [chassis, option] = [0, 1].map(|i| spec.split(',').nth(i).unwrap().parse().unwrap());
    let gear = std::env::var("SLIDE_GEAR").map_or(2, |gear| gear.parse().unwrap());
    let traction_control = std::env::var("SLIDE_TC").is_ok();
    let setup = CarSetup::factory(chassis, option);
    println!("{:?}", power_slide(setup, traction_control, gear));
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
    let catalogue = crate::catalogue::builtin();
    for (tires, compound) in catalogue.compounds.iter().enumerate() {
        for speed in [8.0, 14.0, 17.0, 19.0] {
            let factory = CarSetup::factory(0, 0);
            let build = factory.build.clone().on_compound(tires);
            let setup = CarSetup::new(build, factory.engine.parts, factory.engine.build);
            circle(setup, speed, 120, |second, car| {
                if [10, 30, 60, 120].contains(&second) {
                    let corners: Vec<String> = car
                        .corners
                        .iter()
                        .map(|corner| {
                            let tire = &car.setup.axle(corner.front).tire;
                            let tread = &corner.tread;
                            let surface: Vec<String> =
                                tread.surface.iter().map(|t| format!("{t:.0}")).collect();
                            format!(
                                "{} / {:3.0}°C {:.2} bar {:4.1}% {:3.0}%",
                                surface.join(" "),
                                tread.core_temperature(),
                                tire.pressure(tread),
                                tread.wear() * 100.0,
                                tire.condition(tread) * 100.0,
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

/// Fuel burnt cruising straight on at `speed` (m/s) with the automatic gearbox, once
/// there (L/100 km).
fn cruising_consumption(setup: CarSetup, speed: f32) -> f32 {
    let terrain = Terrain::flat();
    // From one corner of the flat square along its diagonal.
    let mut car = Car::spawn_at(
        0,
        setup,
        &terrain,
        -480.0,
        -480.0,
        -0.75 * std::f32::consts::PI,
    );
    car.drivetrain.automatic = true;
    let dt = 1.0 / CHASSIS_HZ as f32;
    let (mut litres, mut distance) = (0.0, 0.0f64);
    for step in 0..(60.0 / dt) as usize {
        let forward = car.forward_speed();
        car.controls.throttle = (0.3 + 0.5 * (speed - forward)).clamp(0.0, 1.0);
        car.controls.steer = 0.0;
        let before = car.fuel.burnt;
        step_car(&mut car, &terrain, dt);
        if step as f32 * dt > 30.0 {
            litres += car.fuel.burnt - before;
            distance += f64::from(car.forward_speed() * dt);
        }
    }
    (litres / distance * 100e3) as f32
}

/// The coupé's 2-litre burns about what such a car does at 100 km/h.
#[test]
fn cruising_consumption_is_plausible() {
    let consumption = cruising_consumption(CarSetup::factory(0, 0), 100.0 / 3.6);
    assert!(
        (5.0..9.0).contains(&consumption),
        "{consumption:.1} L/100 km"
    );
}

/// Prints every factory car's consumption at 100 and 150 km/h:
/// `cargo test --profile release-dev consumption -- --ignored --nocapture`.
#[test]
#[ignore]
fn consumption() {
    for (chassis, made) in crate::catalogue::builtin().chassis.iter().enumerate() {
        for option in 0..made.engines.len() {
            let at = |kmh: f32| cruising_consumption(CarSetup::factory(chassis, option), kmh / 3.6);
            println!(
                "{:<10} {:<6} {:5.1} L/100 km at 100 km/h, {:5.1} at 150",
                made.name,
                made.engines[option],
                at(100.0),
                at(150.0),
            );
        }
    }
}

/// Going round the circle near the limit with `litres` in the tank: the least share of its
/// fuel the engine got.
fn starved(litres: f32) -> f32 {
    let terrain = Terrain::flat();
    let mut car = Car::spawn_at(0, CarSetup::factory(0, 0), &terrain, 0.0, 0.0, 0.0);
    car.refuel(litres / car.setup.tank.litres);
    car.drivetrain.gear = 2;
    let dt = 1.0 / CHASSIS_HZ as f32;
    let mut least = 1.0f32;
    for _ in 0..(20.0 / dt) as usize {
        let forward = car.forward_speed();
        car.controls.steer = 0.15;
        car.controls.throttle = (0.3 + 0.5 * (14.0 - forward)).clamp(0.0, 1.0);
        step_car(&mut car, &terrain, dt);
        least = least.min(car.drivetrain.engine.fuel_supply);
    }
    least
}

#[test]
fn a_nearly_empty_tank_starves_the_engine_in_a_long_corner() {
    assert!(starved(15.0) > 0.99);
    // It leans out and slows the car until the fuel just about reaches the pickup.
    assert!(starved(3.0) < 0.8);
}

/// The coupé driving at `speed` (m/s) along -z on flat ground towards `obstacle`, for
/// `seconds`, without the driver doing anything once it's let go.
fn drive_into(obstacle: crate::physics::collision::Upright, speed: f32, seconds: f32) -> Car {
    use crate::physics::collision::Obstacles;
    let mut terrain = Terrain::flat();
    terrain.obstacles = Obstacles::new(vec![obstacle], 8.0);
    let mut car = Car::spawn_at(0, CarSetup::factory(0, 0), &terrain, 0.0, 0.0, 0.0);
    let dt = 1.0 / CHASSIS_HZ as f32;
    // Let it settle on its springs, then send it off.
    for _ in 0..60 {
        step_car(&mut car, &terrain, dt);
    }
    car.body.velocity = car.body.rotation * Vec3::new(0.0, 0.0, -speed);
    for corner in &mut car.corners {
        corner.omega = speed / car.setup.axle(corner.front).tire.radius;
    }
    for _ in 0..(seconds / dt) as usize {
        step_car(&mut car, &terrain, dt);
    }
    car
}

#[test]
fn a_car_stops_at_a_wall_and_a_tree() {
    use crate::physics::collision::Upright;
    let upright = |half: Vec2| Upright {
        center: Vec2::new(0.0, -10.0),
        half,
        bottom: -0.5,
        top: 6.0,
        angle: 0.0,
    };
    // The car's nose is about 2 m ahead of its centre: a wall 10 m ahead, across.
    let wall = upright(Vec2::new(10.0, 0.3));
    let car = drive_into(wall, 20.0, 2.0);
    // A crash (that's how hard it hit, for its sound).
    assert!(car.knocks > 15.0, "{}", car.knocks);
    let nose = car.setup.hull.placed(car.body.position, car.body.rotation);
    let front = nose.corners().iter().map(|c| c.z).fold(f32::MAX, f32::min);
    assert!(front > -9.75, "went into the wall: front at {front}");
    assert!(car.body.velocity.length() < 3.0, "{}", car.body.velocity);
    // A trunk in front of the middle of the bonnet.
    let tree = upright(Vec2::splat(0.3));
    let car = drive_into(tree, 20.0, 2.0);
    assert!(
        car.body.position.z > -8.0,
        "went through the tree: {}",
        car.body.position
    );
}

#[test]
fn a_car_stops_at_a_fence_and_a_guard_rail() {
    use crate::fences::{self, Fence, FenceKind};
    for (kind, height) in [(FenceKind::Rails, 1.2), (FenceKind::GuardRail, 0.75)] {
        // One piece across the way, 10 m ahead.
        let fence = Fence {
            kind,
            height,
            points: vec![Vec2::new(-1.2, -10.0), Vec2::new(1.2, -10.0)],
        };
        let piece = fences::pieces(&fence, 0)[0];
        let car = drive_into(fences::solid(&Terrain::flat(), &fence, &piece), 20.0, 2.0);
        // Stopped with its nose at the fence, its middle under 2 m behind it.
        assert!(
            car.body.position.z > -8.5,
            "went through the {kind:?}: {}",
            car.body.position
        );
    }
}

#[test]
fn a_car_on_its_roof_rests_on_it() {
    let terrain = Terrain::flat();
    let mut car = Car::spawn_at(0, CarSetup::factory(0, 0), &terrain, 0.0, 0.0, 0.0);
    car.body.position.y += 2.0;
    car.body.rotation = Quat::from_rotation_z(std::f32::consts::PI);
    let dt = 1.0 / CHASSIS_HZ as f32;
    for _ in 0..(4.0 / dt) as usize {
        step_car(&mut car, &terrain, dt);
    }
    let hull = car.setup.hull.placed(car.body.position, car.body.rotation);
    let lowest = hull.corners().iter().map(|c| c.y).fold(f32::MAX, f32::min);
    assert!(lowest > -0.03 && lowest < 0.02, "lowest corner at {lowest}");
    // It landed on the roof, and lies on it (gravity, every step).
    assert!(car.knocks > 3.0, "{}", car.knocks);
    assert!(car.body.velocity.length() < 0.1, "{}", car.body.velocity);
}

#[test]
fn two_cars_collide() {
    let terrain = Terrain::flat();
    let mut a = Car::spawn_at(0, CarSetup::factory(0, 0), &terrain, 0.0, 0.0, 0.0);
    let mut b = Car::spawn_at(1, CarSetup::factory(0, 0), &terrain, 0.0, -12.0, 0.0);
    let dt = 1.0 / CHASSIS_HZ as f32;
    a.body.velocity = Vec3::new(0.0, 0.0, -15.0);
    for _ in 0..(2.0 / dt) as usize {
        step_car(&mut a, &terrain, dt);
        step_car(&mut b, &terrain, dt);
        super::collide_cars(&mut a, &mut b, dt);
    }
    // The one hit was pushed along; they didn't pass into each other.
    assert!(b.body.position.z < -13.0, "{}", b.body.position);
    let gap = b.body.position.z - a.body.position.z;
    assert!(gap < -3.5, "{gap}");
}

/// The car's peak sideways grip at rest (g): its tires' friction under their loads.
fn grip_at_rest(setup: &CarSetup) -> f32 {
    let axle = |front: bool| {
        let tire = &setup.axle(front).tire;
        let load = (setup.corner_mass(front) + setup.axle(front).unsprung_mass) * 9.81;
        (tire.friction(load) * load, load)
    };
    let (front, rear) = (axle(true), axle(false));
    (front.0 + rear.0) / (front.1 + rear.1)
}

/// A car held on a circle of `radius` (m) at `share` of its grip for `seconds`, steered
/// to the yaw rate the circle needs, starting on cold tires: the outside treads' core
/// temperatures (front, rear; °C) at the end.
fn cornering_heat(setup: CarSetup, radius: f32, share: f32, seconds: f32) -> [f32; 2] {
    let terrain = Terrain::flat();
    let speed = (share * grip_at_rest(&setup) * 9.81 * radius).sqrt();
    let mut car = Car::spawn_at(0, setup, &terrain, 0.0, 0.0, 0.0);
    car.drivetrain.gear = 2;
    let dt = 1.0 / CHASSIS_HZ as f32;
    let mut t = 0.0;
    while t < seconds {
        let forward = car.forward_speed();
        let (_, yaw) = slip_and_yaw(&car);
        let wanted = forward.max(1.0) / radius;
        let c = &mut car.controls;
        c.steer = (c.steer + 2.0 * (wanted - yaw) * dt).clamp(0.0, 1.0);
        c.throttle = (0.3 + 0.5 * (speed - forward)).clamp(0.0, 1.0);
        step_car(&mut car, &terrain, dt);
        t += dt;
    }
    // Turning left, the right-hand tires are on the outside.
    [1, 3].map(|i| car.corners[i].tread.core_temperature())
}

/// Light, middling and heavy cars on the same compound heat their tires alike at the same
/// share of their grip, as their tires are sized for them; on the middling car's tires,
/// the light car's stay cool and the heavy car's cook.
#[test]
fn tires_are_sized_for_their_cars() {
    // A heavier hatch runs its tires hotter at the same share of its grip, wider ones less.
    use crate::physics::tire::STREET;
    let catalogue = crate::catalogue::builtin_changed(|c| c.chassis[0].mass += 450.0);
    let hottest = |wide: bool| {
        let factory = CarSetup::factory_in(&catalogue, 0, 0);
        let mut build = factory.build.clone().on_compound(STREET);
        if wide {
            let wheel = crate::wheels::Wheel {
                rim: crate::wheels::Rim {
                    diameter: 17,
                    width: 75,
                },
                tire: crate::wheels::TireSize {
                    width: 235,
                    aspect: 40,
                    rim: 17,
                },
                compound: STREET,
            };
            build.wheels = [wheel; 2];
        }
        let setup = CarSetup::new(build, factory.engine.parts, factory.engine.build);
        let [front, rear] = cornering_heat(setup, 40.0, 0.7, 60.0);
        println!("wide {wide}: outside treads {front:.0} °C, {rear:.0} °C");
        front.max(rear)
    };
    let (narrow, wide) = (hottest(false), hottest(true));
    assert!(wide < narrow - 2.0, "{wide} against {narrow} °C");
}

/// Prints how warm every car as it leaves the factory gets its outside treads at 70 % of
/// its grip for a minute, and how much grip that leaves them: `cargo test --profile
/// release-dev cornering_temperatures -- --ignored --nocapture`.
#[test]
#[ignore]
fn cornering_temperatures() {
    for chassis in 0..crate::catalogue::builtin().chassis.len() {
        let setup = CarSetup::factory(chassis, 0);
        let compound_name = setup.build.catalogue.compounds[setup.build.wheels[0].compound]
            .name
            .clone();
        let (name, compound) = (setup.name.clone(), setup.front.tire.compound);
        let grip = grip_at_rest(&setup);
        let [front, rear] = cornering_heat(setup, 40.0, 0.7, 60.0);
        let warmth = |t: f32| compound.warmth(t) * 100.0;
        println!(
            "{name:<8} {:<8} grip {grip:.2}: front {front:3.0} °C ({:3.0} %), rear {rear:3.0} °C \
             ({:3.0} %)",
            compound_name,
            warmth(front),
            warmth(rear)
        );
    }
}

/// The factory's tire pressures suit its tires under the car's weight.
#[test]
fn factory_pressures_suit_the_tires() {
    for chassis in 0..crate::catalogue::builtin().chassis.len() {
        let setup = CarSetup::factory(chassis, 0);
        for (axle, front) in [true, false].into_iter().enumerate() {
            let load = (setup.corner_mass(front) + setup.axle(front).unsprung_mass) * 9.81;
            let suited = setup.axle(front).tire.suited_pressure(load);
            let set = setup.build.pressure[axle];
            assert!(
                (set - suited).abs() < 0.06,
                "{}: {set} against {suited:.2}",
                setup.name
            );
        }
    }
}

/// Pushed sideways standing still (as if parked across a slope), or along it on the
/// brakes, the tires hold the car on their carcasses' flex instead of letting it creep
/// away, and it doesn't rock.
#[test]
fn tires_hold_a_parked_car() {
    let terrain = Terrain::flat();
    let mut car = Car::spawn_at(0, CarSetup::factory(0, 0), &terrain, 0.0, 0.0, 0.0);
    let dt = 1.0 / CHASSIS_HZ as f32;
    for _ in 0..(2.0 / dt) as usize {
        step_car(&mut car, &terrain, dt);
    }
    let start = car.body.position;
    let push = car.body.rotation * Vec3::X * 0.15 * 9.81 * car.body.mass;
    let mut moved = 0.0;
    for step in 0..(5.0 / dt) as usize {
        car.body.add_force_at_center(push);
        step_car(&mut car, &terrain, dt);
        let sideways = (car.body.position - start).dot(car.body.rotation * Vec3::X);
        if step as f32 * dt > 1.0 {
            moved = sideways;
        }
    }
    assert!(moved.abs() < 0.03, "moved {moved} m");
    assert!(car.body.velocity.length() < 0.01, "{}", car.body.velocity);
    // On the brakes, pushed along it (as if parked on a hill), it holds too.
    car.controls.brake = 1.0;
    let start = car.body.position;
    let push = car.body.rotation * Vec3::NEG_Z * 0.15 * 9.81 * car.body.mass;
    for _ in 0..(5.0 / dt) as usize {
        car.body.add_force_at_center(push);
        step_car(&mut car, &terrain, dt);
    }
    let moved = (car.body.position - start).length();
    assert!(moved < 0.03, "rolled {moved} m");
    assert!(car.body.velocity.length() < 0.01, "{}", car.body.velocity);
}

/// A car held on a 60 m circle at `lateral` g on warm tires: how far its body rolls (degrees
/// per g) and the front axle's share of the load moved across the car.
fn steady_roll(setup: CarSetup, lateral: f32) -> (f32, f32) {
    let terrain = Terrain::flat();
    let radius = 60.0;
    let speed = (lateral * 9.81 * radius).sqrt();
    let mut car = Car::spawn_at(0, setup, &terrain, 0.0, 0.0, 0.0);
    car.warm_tires();
    car.drivetrain.gear = 2;
    let dt = 1.0 / CHASSIS_HZ as f32;
    let (mut roll, mut front, mut moved, mut felt) = (0.0, 0.0, 0.0, 0.0);
    let steps = 12 * CHASSIS_HZ as usize;
    for step in 0..steps {
        let forward = car.forward_speed();
        let (_, yaw) = slip_and_yaw(&car);
        let wanted = forward.max(1.0) / radius;
        let c = &mut car.controls;
        c.steer = (c.steer + 2.0 * (wanted - yaw) * dt).clamp(0.0, 1.0);
        c.throttle = (0.3 + 0.5 * (speed - forward)).clamp(0.0, 1.0);
        step_car(&mut car, &terrain, dt);
        // Averaged over the last 3 s.
        if step >= steps - 3 * CHASSIS_HZ as usize {
            let load = |i: usize| car.corners[i].contact.map_or(0.0, |c| c.load);
            // Turning left, the right-hand wheels (1, 3) are on the outside.
            let (f, r) = (load(1) - load(0), load(3) - load(2));
            front += f;
            moved += f + r;
            // The body leans out: its right side drops.
            roll += -(car.body.rotation * Vec3::X).y.asin().to_degrees();
            felt += car.forward_speed() * slip_and_yaw(&car).1 / 9.81;
        }
    }
    (roll / felt, front / moved)
}

/// Every car as it leaves the factory rolls like a real one at 0.5 g (1.2–5.5° per g, the
/// vans most), and the front-engined rear-wheel-drive ones move more of the load across
/// the front than the front carries, so they push a little rather than step out.
#[test]
fn factory_cars_roll_like_real_ones() {
    use crate::physics::drivetrain::Drive;
    for (chassis, made) in crate::catalogue::builtin().chassis.iter().enumerate() {
        let setup = CarSetup::factory(chassis, 0);
        let weight = front_weight(&setup);
        let rear = setup.engine.drive == Drive::Rear;
        let (roll, moved) = steady_roll(setup, 0.5);
        assert!((1.2..5.5).contains(&roll), "{}: {roll}°/g", made.name);
        if rear && weight > 0.5 {
            assert!(moved > weight, "{}: {moved} against {weight}", made.name);
        }
    }
}

/// A stiffer front bar rolls the coupé less and moves more of the load across the front.
#[test]
fn a_stiffer_front_bar_moves_the_load_forward() {
    let factory = CarSetup::factory(0, 0);
    let mut build = factory.build.clone();
    build.bars[0] = build.chassis().bars[0].sizes.len() - 1;
    let stiff = CarSetup::new(
        build,
        factory.engine.parts.clone(),
        factory.engine.build.clone(),
    );
    let (roll, moved) = steady_roll(factory, 0.5);
    let (stiff_roll, stiff_moved) = steady_roll(stiff, 0.5);
    assert!(stiff_roll < roll - 0.2, "{stiff_roll} against {roll}°/g");
    assert!(stiff_moved > moved + 0.03, "{stiff_moved} against {moved}");
}

/// The share of a car's sprung weight on the front wheels.
fn front_weight(setup: &CarSetup) -> f32 {
    setup.corner_mass(true) / (setup.corner_mass(true) + setup.corner_mass(false))
}

/// Prints how much every car as it leaves the factory rolls and where the load moves:
/// `cargo test --profile release-dev roll_stiffness -- --ignored --nocapture`.
#[test]
#[ignore]
fn roll_stiffness() {
    for (chassis, made) in crate::catalogue::builtin().chassis.iter().enumerate() {
        let setup = CarSetup::factory(chassis, 0);
        let name = setup.name.clone();
        let weight = front_weight(&setup);
        let deg = std::f32::consts::PI / 180.0;
        // Roll stiffness of the springs and of the bars on each axle (N·m per degree).
        let stiffness = [true, false].map(|front| {
            let axle = setup.axle(front);
            let track = 2.0 * axle.geometry.wheel_center.x;
            let ride = made.ride[usize::from(!front)];
            let wheel_rate = setup.corner_mass(front) * (std::f32::consts::TAU * ride).powi(2);
            let springs = wheel_rate * track * track / 2.0 * deg;
            (springs, axle.roll_bar * track * track * deg)
        });
        let (roll, moved) = steady_roll(setup, 0.5);
        println!(
            "{name:<8} weight {:3.0} % front | springs {:4.0} / {:4.0}, bars {:4.0} / {:4.0} \
             N·m/° | roll {roll:.2}°/g, load moved {:3.0} % front",
            weight * 100.0,
            stiffness[0].0,
            stiffness[1].0,
            stiffness[0].1,
            stiffness[1].1,
            moved * 100.0
        );
    }
}

/// Prints what simulating one car costs (a server simulates every player's): on the map,
/// with its houses, trees and fences, following the roads from the spawn at about 50 km/h
/// (aiming at the road 12 m ahead) for 30 s (`STEP_SECONDS`). The drive is recorded, then
/// replayed from the start `STEP_REPEAT` times (3) with the clock running, each ending
/// where the first did to the bit.
#[test]
#[ignore]
fn step_cost() {
    let number = |name: &str, default: f32| {
        (std::env::var(name).ok())
            .and_then(|s| s.parse().ok())
            .unwrap_or(default)
    };
    let mut terrain = Terrain::baked();
    terrain.obstacles = crate::obstacles::build(&terrain);
    let (p, yaw) = terrain.road_spawns(&crate::terrain::Spawn::DEFAULT, 1, 0.0)[0];
    let mut start = Car::spawn_at(0, CarSetup::default(), &terrain, p.x, p.y, yaw);
    start.drivetrain.automatic = true;
    let dt = 1.0 / CHASSIS_HZ as f32;
    let seconds = number("STEP_SECONDS", 30.0);
    let steps = (seconds / dt) as usize;
    let mut car = start.clone();
    let mut controls = Vec::with_capacity(steps);
    let (mut aim, mut distance) = (p, 0.0);
    for step in 0..steps {
        let forward = (car.body.rotation * Vec3::NEG_Z).xz().normalize_or_zero();
        // Finding the road is slow: every tenth of a second.
        if step % 12 == 0 {
            let ahead = car.body.position.xz() + 12.0 * forward;
            aim = terrain
                .drive_spot(ahead, forward)
                .map_or(ahead, |(spot, _)| spot);
        }
        let to = (aim - car.body.position.xz()).normalize_or_zero();
        car.controls.steer = -(2.0 * forward.perp_dot(to)).clamp(-1.0, 1.0);
        let speed = car.forward_speed();
        car.controls.throttle = (0.3 + 0.3 * (14.0 - speed)).clamp(0.0, 1.0);
        car.controls.brake = (0.2 * (speed - 16.0)).clamp(0.0, 1.0);
        controls.push(car.controls);
        step_car(&mut car, &terrain, dt);
        distance += car.forward_speed().abs() * dt;
    }
    let mut fastest = f64::INFINITY;
    for _ in 0..number("STEP_REPEAT", 3.0) as usize {
        let mut again = start.clone();
        let clock = std::time::Instant::now();
        for c in &controls {
            again.controls = *c;
            step_car(&mut again, &terrain, dt);
        }
        fastest = fastest.min(clock.elapsed().as_secs_f64());
        assert_eq!(
            again.body.position, car.body.position,
            "the replay went elsewhere"
        );
    }
    println!(
        "{:.1} µs a step, {:.2} ms per second driven ({:.2} % of a core); drove {distance:.0} m, \
         {:.0} m from the start",
        fastest * 1e6 / steps as f64,
        fastest * 1e3 / f64::from(seconds),
        fastest / f64::from(seconds) * 100.0,
        car.body.position.xz().distance(p)
    );
}

/// Prints every factory car's time from standing to 100 km/h, flat out with traction
/// control and the automatic gearbox, on warm tires:
/// `cargo test --profile release-dev zero_to_hundred -- --ignored --nocapture`.
#[test]
#[ignore]
fn zero_to_hundred() {
    for (chassis, made) in crate::catalogue::builtin().chassis.iter().enumerate() {
        for option in 0..made.engines.len() {
            let time = time_to_hundred(CarSetup::factory(chassis, option));
            println!("{:<8} {:<5} {time:4.1} s", made.name, made.engines[option]);
        }
    }
}

/// A car's time from standing to 100 km/h (s), flat out with traction control and the
/// automatic gearbox, on warm tires (at most 30).
fn time_to_hundred(setup: CarSetup) -> f32 {
    let terrain = Terrain::flat();
    let angle = -0.75 * std::f32::consts::PI;
    let mut car = Car::spawn_at(0, setup, &terrain, -480.0, -480.0, angle);
    car.warm_tires();
    car.drivetrain.automatic = true;
    car.traction_control = true;
    car.parking_brake = false;
    let dt = 1.0 / CHASSIS_HZ as f32;
    let mut time = 0.0;
    while car.forward_speed() < 100.0 / 3.6 && time < 30.5 {
        car.controls.throttle = if time < 0.5 { 0.0 } else { 1.0 };
        step_car(&mut car, &terrain, dt);
        time += dt;
    }
    time - 0.5
}

/// The K-series cars pull away like their kinds: the hot hatch quickest (about 6 s), the
/// hatch and the saloons in 7 to 8.5 s, the heavy box slowest (its all-wheel-drive one
/// about as quick: more grip, more weight).
#[test]
fn the_k_cars_reach_a_hundred_in_their_time() {
    let catalogue = crate::catalogue::builtin();
    let time = |chassis: &str, engine: &str| {
        let c = catalogue.find_chassis(chassis).expect("the chassis");
        let made = &catalogue.chassis[c];
        let option = made.engines.iter().position(|e| e == engine).unwrap();
        let time = time_to_hundred(CarSetup::factory(c, option));
        println!("{chassis} {engine}: 0-100 in {time:.1} s");
        time
    };
    let hot = time("Tessel", "K20R");
    let hatch = time("Tessel", "K20");
    let saloon = time("Aldine", "K24");
    let lazy = time("Bothy", "K24");
    let awd = time_to_hundred(CarSetup::of_trim(&catalogue, "bothy.2-4-awd").unwrap());
    println!("Bothy K24 AWD: 0-100 in {awd:.1} s");
    assert!((lazy - 0.5..lazy + 1.0).contains(&awd), "{awd}");
    assert!((5.0..7.0).contains(&hot), "{hot}");
    assert!((6.5..8.5).contains(&hatch) && hot < hatch - 0.6, "{hatch}");
    assert!((6.5..9.0).contains(&saloon), "{saloon}");
    assert!((7.5..10.5).contains(&lazy) && lazy > saloon, "{lazy}");
}
