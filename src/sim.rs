//! The multi-rate simulation loop.
//!
//! The chassis integrates at `CHASSIS_HZ`. Within each chassis step, every corner runs
//! `SUBSTEPS` steps of its own against the chassis moving at constant velocity, and the
//! average force it exerts is applied to the chassis.

use bevy::prelude::*;

use crate::car::Car;
use crate::physics::collision::{self, GROUND, OBSTACLE};
use crate::physics::drivetrain::Drive;
use crate::physics::suspension::ChassisFrame;
use crate::terrain::Terrain;

pub mod steering;

pub const CHASSIS_HZ: f64 = 120.0;
/// A simulation step (s). Every step is exactly this, wherever a car is simulated (the
/// fixed clock's own step, a whole number of nanoseconds, is a hair off), so the server's
/// copy of a car steps exactly as the player's.
pub const STEP: f32 = (1.0 / CHASSIS_HZ) as f32;
pub const SUBSTEPS: usize = 10;
const GRAVITY: Vec3 = Vec3::new(0.0, -9.81, 0.0);
/// Density of the air (kg/m³).
const AIR_DENSITY: f32 = 1.2;
/// Traction control: the slip ratio of a driven wheel it holds (a little past the tires'
/// best), how fast it closes the throttle per share of that slip it's over and opens it
/// again per share it's under (shares of the pedal per second), and the least it leaves.
/// The rate follows the slip smoothly through the target, so it settles rather than
/// chopping the throttle.
const TRACTION_SLIP: f32 = 0.12;
const TRACTION_CUT: f32 = 4.0;
const TRACTION_OPEN: f32 = 2.0;
const TRACTION_LEAST: f32 = 0.08;
/// ABS: the slip ratio of a braked wheel it holds (about where the tires grip best), how
/// fast it eases the wheel's brake per share of that slip it's over and gives it back per
/// share it's under (shares of the pedal per second), and the speed below which it lets
/// the wheels lock to stop (m/s). Like traction control it follows the slip smoothly,
/// rather than pumping.
const ABS_SLIP: f32 = 0.1;
const ABS_CUT: f32 = 40.0;
const ABS_OPEN: f32 = 10.0;
const ABS_SPEED: f32 = 2.0;

pub struct SimPlugin;

impl Plugin for SimPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(CHASSIS_HZ))
            .add_systems(FixedUpdate, simulate);
    }
}

pub fn simulate(
    terrain: Res<Terrain>,
    mut cars: Query<&mut Car, Without<crate::car::ServerDriven>>,
) {
    let dt = STEP;
    let before: Vec<Vec3> = cars.iter().map(|car| car.body.position).collect();
    for mut car in &mut cars {
        step_car(&mut car, &terrain, dt);
    }
    // Cars against each other, each pair whose spheres touch.
    let mut pairs = cars.iter_combinations_mut();
    while let Some([mut a, mut b]) = pairs.fetch_next() {
        collide_cars(&mut a, &mut b, dt);
    }
    for (mut car, before) in cars.iter_mut().zip(before) {
        recover(&mut car, &terrain, before);
    }
}

/// A car whose simulation blew up (any number not finite) is put on the nearest road
/// from where it was before the step, as R would. It mustn't carry on: online, the
/// server refuses a state that isn't finite and drops the player.
fn recover(car: &mut Car, terrain: &Terrain, before: Vec3) {
    let body = &car.body;
    let finite = body.position.is_finite()
        && body.rotation.is_finite()
        && body.velocity.is_finite()
        && body.angular_velocity.is_finite()
        && car.drivetrain.rpm.is_finite()
        && car.drivetrain.load.is_finite()
        && car.drivetrain.engine.turbo_speed.is_finite()
        && (car.corners.iter()).all(|corner| {
            [corner.omega, corner.theta, corner.spin]
                .iter()
                .all(|x| x.is_finite())
        });
    if finite {
        return;
    }
    warn!("The car's simulation blew up at {before}: put back on the road");
    car.body.position = before;
    if !car.body.rotation.is_finite() {
        car.body.rotation = Quat::IDENTITY;
    }
    let (p, yaw) = car.road_spot(terrain);
    car.place(terrain, p, yaw);
}

/// Two cars' bodies pushed apart where their boxes overlap.
pub fn collide_cars(a: &mut Car, b: &mut Car, dt: f32) {
    let hull_a = a.setup.hull.placed(a.body.position, a.body.rotation);
    let hull_b = b.setup.hull.placed(b.body.position, b.body.rotation);
    let reach = hull_a.radius() + hull_b.radius();
    if hull_a.center.distance_squared(hull_b.center) > reach * reach {
        return;
    }
    if let Some(contact) = hull_a.contact(&hull_b) {
        let impulse = collision::resolve(&mut a.body, Some(&mut b.body), &[contact], OBSTACLE, dt);
        a.knocks += impulse / a.body.mass;
        b.knocks += impulse / b.body.mass;
    }
}

pub fn step_car(car: &mut Car, terrain: &Terrain, dt: f32) {
    apply_controls(car, dt);

    let Car {
        setup,
        body,
        corners,
        rack,
        knocks,
        ..
    } = car;
    let h = dt / SUBSTEPS as f32;
    let (mut force, mut torque) = (Vec3::ZERO, Vec3::ZERO);
    let mut sum = steering::Rack::default();
    let rest_length = [true, false].map(|front| {
        let axle = setup.axle(front);
        axle.spring_rest_length(setup.corner_mass(front))
    });
    for k in 0..SUBSTEPS {
        let (position, rotation) = body.extrapolate(k as f32 * h);
        let frame = ChassisFrame {
            position,
            rotation,
            velocity: body.velocity,
            angular_velocity: body.angular_velocity,
        };
        // Each axle's anti-roll bar twists by how far one wheel is up relative to the
        // other, and pushes them back level.
        for (left, right) in [(0, 1), (2, 3)] {
            let rate = setup.axle(corners[left].front).roll_bar;
            let push = rate * (corners[right].travel - corners[left].travel);
            corners[left].bar_force = push;
            corners[right].bar_force = -push;
        }
        for corner in corners.iter_mut() {
            let axle = setup.axle(corner.front);
            let rest_length = rest_length[usize::from(!corner.front)];
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
        sum += steering::Rack::of(setup, corners, position, rotation);
    }
    // The brakes heat with what they held the wheels back with.
    let speed = body.velocity.length();
    for corner in corners.iter_mut() {
        let brake = &setup.brakes[usize::from(!corner.front)];
        corner.brake.step(brake, corner.omega, speed, dt);
    }
    // What the front wheels do to the steering, averaged over the substeps.
    *rack = sum.scaled(1.0 / SUBSTEPS as f32);
    // The air: drag against the motion, and downforce over each axle, both with the
    // square of the speed.
    let pressure = 0.5 * AIR_DENSITY * body.velocity.length_squared();
    let drag = -body.velocity.normalize_or_zero() * pressure * setup.drag_area;
    let down = body.rotation * Vec3::NEG_Y;
    // How far each end of the body is below its design height: the wheels up against it.
    let lowered = [0, 2].map(|i| (corners[i].travel + corners[i + 1].travel) / 2.0);
    let downforce = setup.downforce(lowered);
    for (area, z) in [(downforce[0], setup.front_z), (downforce[1], setup.rear_z)] {
        let push = down * pressure * area;
        force += push * SUBSTEPS as f32;
        torque += (body.rotation * Vec3::new(0.0, 0.0, z)).cross(push) * SUBSTEPS as f32;
    }
    body.add_force(force / SUBSTEPS as f32);
    body.add_force_at_center(GRAVITY * body.mass + drag);
    body.add_torque(torque / SUBSTEPS as f32);
    let velocity = body.velocity;
    body.integrate_velocity(dt);
    // The body's box against the ground, then against houses and trees, before it moves.
    let hull = setup.hull.placed(body.position, body.rotation);
    let mut contacts = Vec::new();
    collision::ground_contacts(&hull, |p| terrain.ground_under(p), &mut contacts);
    *knocks += collision::resolve(body, None, &contacts, GROUND, dt) / body.mass;
    contacts.clear();
    terrain.obstacles.contacts(&hull, &mut contacts);
    *knocks += collision::resolve(body, None, &contacts, OBSTACLE, dt) / body.mass;
    body.integrate_position(dt);

    // The engine drinks from the tank, and the fuel sloshes with what the car feels.
    let felt = body.rotation.inverse() * ((body.velocity - velocity) / dt - GRAVITY);
    let flow = car.drivetrain.engine.fuel_flow;
    car.fuel.step(&car.setup.tank, flow, felt, dt);
    car.fuel.distance += f64::from(car.body.velocity.length() * dt);
    car.load_fuel();
}

/// Engine torque to the driven wheels through the differentials (less what traction
/// control holds back), biased brakes (less what ABS holds back), rear handbrake, and
/// steering.
fn apply_controls(car: &mut Car, dt: f32) {
    let speed = car.forward_speed();
    car.drivetrain.engine.airspeed = speed;
    car.drivetrain.engine.fuel_supply = car.fuel.supply;
    car.setup.engine.build.octane = car.fuel.octane();
    let Car {
        setup,
        corners,
        drivetrain,
        controls: c,
        traction_control,
        traction,
        abs,
        abs_pressure,
        ..
    } = car;

    let engine = &setup.engine;
    let shares = engine.drive.shares();
    // Traction control eases the throttle while a driven wheel spins the way the gear
    // drives it (the slip ratio is positive spinning forwards, negative backwards).
    let way = if drivetrain.gear < 0 { -1.0 } else { 1.0 };
    let slip = (0..4)
        .filter(|&i| shares[i] > 0.0)
        .filter_map(|i| corners[i].contact)
        .map(|contact| way * contact.tire.slip_ratio)
        .fold(0.0, f32::max);
    let off = (slip - TRACTION_SLIP) / TRACTION_SLIP;
    let rate = if off > 0.0 {
        -TRACTION_CUT * off
    } else {
        -TRACTION_OPEN * off
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
    // ABS eases each wheel's brake while the wheel turns slower than the tire grips best at
    // (the slip ratio is negative braking forwards, positive braking backwards).
    for (pressure, corner) in abs_pressure.iter_mut().zip(corners.iter()) {
        let slip = corner
            .contact
            .map_or(0.0, |c| -c.tire.slip_ratio * speed.signum());
        let off = (slip - ABS_SLIP) / ABS_SLIP;
        let rate = if off > 0.0 {
            -ABS_CUT * off
        } else {
            -ABS_OPEN * off
        };
        *pressure = if *abs && speed.abs() > ABS_SPEED {
            (*pressure + rate * dt).clamp(0.0, 1.0)
        } else {
            1.0
        };
    }
    let steer = c.steer * setup.max_steer;
    for (i, corner) in corners.iter_mut().enumerate() {
        let brake = c.brake * abs_pressure[i];
        // Rolling resistance holds each wheel back like a light brake.
        let tire = &setup.axle(corner.front).tire;
        let rolling = corner.contact.map_or(0.0, |c| {
            tire.rolling_torque(c.load, &corner.tread) * c.surface.rolling()
        });
        corner.drive_torque = torque[i];
        corner.drive_inertia = shares[i] * drive.inertia;
        // The pads, as hot and worn as they are.
        let grip = corner.brake.grip(&setup.brakes[usize::from(!corner.front)]);
        let pads = if corner.front {
            corner.steer = steer;
            brake * setup.brake_torque * setup.brake_bias / 2.0
        } else {
            brake * setup.brake_torque * (1.0 - setup.brake_bias) / 2.0
                + c.handbrake * setup.handbrake_torque
        } * grip;
        corner.brake.torque = pads;
        corner.brake_torque = rolling + pads;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::car::{CarSetup, Controls, heading};
    use crate::physics::tire::RACE;
    use crate::terrain::Spawn;
    use std::f32::consts::{PI, TAU};

    fn run(car: &mut Car, terrain: &Terrain, seconds: f32) {
        let dt = 1.0 / CHASSIS_HZ as f32;
        for _ in 0..(seconds / dt) as usize {
            step_car(car, terrain, dt);
        }
    }

    /// A car on the road at the first spawn point, pointing along the road.
    fn test_car(terrain: &Terrain) -> Car {
        let (p, yaw) = terrain.road_spawns(&Spawn::DEFAULT, 1, 0.0)[0];
        Car::spawn_at(0, CarSetup::default(), terrain, p.x, p.y, yaw)
    }

    #[test]
    fn a_car_that_blew_up_is_put_back_on_the_road() {
        let terrain = Terrain::baked();
        let mut app = App::new();
        app.insert_resource(Terrain::baked())
            .insert_resource(Time::<()>::default())
            .add_systems(Update, simulate);
        let mut car = test_car(&terrain);
        let at = car.body.position;
        car.body.velocity = Vec3::NAN;
        let entity = app.world_mut().spawn(car).id();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(1.0 / 120.0));
        app.update();
        let car = app.world().get::<Car>(entity).unwrap();
        assert!(car.body.velocity.is_finite() && car.body.position.is_finite());
        assert!(car.body.position.xz().distance(at.xz()) < 5.0);
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
        let (p, yaw) = terrain.road_spawns(&Spawn::DEFAULT, 1, 0.0)[0];
        for (index, chassis) in crate::catalogue::builtin().chassis.iter().enumerate() {
            for option in 0..chassis.engines.len() {
                let setup = CarSetup::factory(index, option);
                let name = format!("{} {}", chassis.name, chassis.engines[option]);
                // The engine's place shows in the balance.
                let front = setup.front_weight_with(setup.tank.litres);
                let expected = match chassis.bay.place.as_str() {
                    "middle" => 0.38..0.47,
                    "rear" => 0.34..0.42,
                    _ => 0.46..0.66,
                };
                assert!(
                    expected.contains(&front),
                    "{name}: {front} on the front axle"
                );
                let mass = setup.mass_with(setup.tank.litres);
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
            car.rack.total() < -10.0,
            "steering torque {}",
            car.rack.total()
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

    /// The hatch (of `catalogue`) with its engine turbocharged and boosted hard, to spin
    /// its wheels.
    fn strong_hatch(catalogue: &crate::catalogue::Shared) -> CarSetup {
        let build = crate::chassis::CarBuild::factory_in(catalogue, 0);
        let mut parts = build
            .install(crate::physics::engine::parts::Parts::turbo())
            .unwrap();
        parts.chosen[crate::catalogue::TURBO] = 2;
        parts.boost = 1.2;
        let engine = crate::physics::engine::EngineBuild::of_parts(&parts);
        CarSetup::new(build, parts, engine)
    }

    #[test]
    fn traction_control_keeps_a_big_engine_from_spinning_its_wheels() {
        let terrain = Terrain::baked();
        let (p, yaw) = terrain.road_spawns(&Spawn::DEFAULT, 1, 0.0)[0];
        // The strong hatch, floored in first.
        let launch = |traction_control: bool| {
            let mut car = Car::spawn_at(
                0,
                strong_hatch(&crate::catalogue::builtin()),
                &terrain,
                p.x,
                p.y,
                yaw,
            );
            car.warm_tires();
            car.traction_control = traction_control;
            run(&mut car, &terrain, 1.0);
            car.controls = Controls {
                throttle: 1.0,
                ..default()
            };
            run(&mut car, &terrain, 2.5);
            let radius = car.setup.front.tire.radius;
            let wheels = (car.corners[0].omega + car.corners[1].omega) / 2.0 * radius;
            (car.forward_speed(), wheels)
        };
        let (speed, wheels) = launch(true);
        assert!(
            wheels < 1.25 * speed,
            "wheels at {wheels} m/s, car at {speed}"
        );
        // Without it the wheels spin away (a little less since the turbine's inlet sends
        // less of the header's wave back).
        let (spinning, wheels) = launch(false);
        assert!(
            wheels > 1.3 * spinning,
            "wheels at {wheels} m/s, car at {spinning}"
        );
        assert!(speed > spinning + 1.0, "{speed} against {spinning} m/s");
    }

    #[test]
    fn traction_control_launches_the_roadster() {
        let terrain = Terrain::flat();
        let merrow = crate::catalogue::builtin()
            .find_chassis("Merrow")
            .expect("the roadster");
        // The V10 floored in first from standing.
        let launch = |traction_control: bool| {
            let setup = CarSetup::factory(merrow, 0);
            let mut car = Car::spawn_at(0, setup, &terrain, 0.0, 400.0, 0.0);
            car.warm_tires();
            car.parking_brake = false;
            car.traction_control = traction_control;
            car.drivetrain.shift_to(&car.setup.engine, 1);
            run(&mut car, &terrain, 1.0);
            car.controls = Controls {
                throttle: 1.0,
                ..default()
            };
            run(&mut car, &terrain, 1.5);
            let radius = car.setup.rear.tire.radius;
            let wheels = (car.corners[2].omega + car.corners[3].omega) / 2.0 * radius;
            (car.forward_speed(), wheels)
        };
        let (speed, wheels) = launch(true);
        println!("with traction control: {speed:.1} m/s, rear wheels {wheels:.1} m/s");
        assert!(
            speed > 5.0 && wheels < 1.25 * speed,
            "wheels at {wheels} m/s, car at {speed}"
        );
        let (spinning, wheels) = launch(false);
        println!("without: {spinning:.1} m/s, rear wheels {wheels:.1} m/s");
        assert!(
            wheels > 1.5 * spinning,
            "wheels at {wheels} m/s, car at {spinning}"
        );
        assert!(speed > spinning, "{speed} against {spinning} m/s");
    }

    #[test]
    fn traction_control_works_in_reverse() {
        let terrain = Terrain::flat();
        // The strong hatch made to drive its rear wheels (weight going onto them in
        // reverse, which the front-wheel drive gains), floored in reverse.
        let rear = crate::catalogue::builtin_changed(|c| c.chassis[0].drive = Drive::Rear);
        let launch = |traction_control: bool| {
            let mut car = Car::spawn_at(0, strong_hatch(&rear), &terrain, 0.0, 400.0, 0.0);
            car.warm_tires();
            car.traction_control = traction_control;
            car.drivetrain.shift_to(&car.setup.engine, -1);
            run(&mut car, &terrain, 1.0);
            car.controls = Controls {
                throttle: 1.0,
                ..default()
            };
            run(&mut car, &terrain, 1.5);
            let radius = car.setup.rear.tire.radius;
            let wheels = (car.corners[2].omega + car.corners[3].omega) / 2.0 * radius;
            (-car.forward_speed(), -wheels)
        };
        let (speed, wheels) = launch(true);
        assert!(
            speed > 0.0 && wheels < 1.25 * speed,
            "wheels at {wheels} m/s, car at {speed}"
        );
        let (spinning, wheels) = launch(false);
        assert!(
            wheels > 1.5 * spinning,
            "wheels at {wheels} m/s, car at {spinning}"
        );
    }

    #[test]
    fn the_air_limits_the_top_speed() {
        // On flat asphalt: at this speed a real road's first bend comes too soon.
        let terrain = Terrain::flat();
        let mut car = Car::spawn_at(0, CarSetup::default(), &terrain, 0.0, 400.0, 0.0);
        car.parking_brake = false;
        run(&mut car, &terrain, 1.0);
        // Coasting in neutral from 180 km/h: drag and the tires slow the car by about
        // 1 m/s² (0.62 m² of drag area on 1260 kg).
        car.drivetrain.shift_to(&car.setup.engine, 0);
        car.body.velocity = car.body.rotation * Vec3::NEG_Z * 50.0;
        let radius = car.setup.front.tire.radius;
        for corner in car.corners.iter_mut() {
            corner.omega = 50.0 / radius;
        }
        run(&mut car, &terrain, 0.5);
        let lost = 50.0 - car.body.velocity.length();
        assert!((0.3..0.8).contains(&lost), "lost {lost} m/s");
    }

    #[test]
    fn rough_roads_shake_the_car_and_smooth_ones_hardly() {
        // Coasting at 54 km/h along a straight road: how much the body moves up and down.
        let shaking = |roughness: f32| {
            let terrain = Terrain::flat_road(roughness);
            let mut car = Car::spawn_at(0, CarSetup::default(), &terrain, 0.0, 400.0, 0.0);
            car.parking_brake = false;
            car.drivetrain.shift_to(&car.setup.engine, 0);
            run(&mut car, &terrain, 1.0);
            car.body.velocity = car.body.rotation * Vec3::NEG_Z * 15.0;
            for corner in car.corners.iter_mut() {
                corner.omega = 15.0 / car.setup.axle(corner.front).tire.radius;
            }
            let dt = 1.0 / CHASSIS_HZ as f32;
            let (mut sum, mut steps, mut last) = (0.0, 0, car.body.velocity.y);
            for _ in 0..(4.0 / dt) as usize {
                step_car(&mut car, &terrain, dt);
                let lift = (car.body.velocity.y - last) / dt;
                last = car.body.velocity.y;
                sum += lift * lift;
                steps += 1;
            }
            assert!(car.body.position.x.abs() < 2.0, "left the road");
            (sum / steps as f32).sqrt()
        };
        let (smooth, rough) = (shaking(16.0), shaking(1024.0));
        println!("vertical acceleration: {smooth:.2} m/s² smooth, {rough:.2} m/s² rough");
        assert!(smooth < 0.3, "smooth road: {smooth} m/s²");
        assert!((0.5..5.0).contains(&rough), "rough road: {rough} m/s²");
    }

    #[test]
    fn automatic_shifts_up_without_hunting() {
        let terrain = Terrain::flat();
        let mut car = Car::spawn_at(0, CarSetup::default(), &terrain, 0.0, 400.0, 0.0);
        car.drivetrain.automatic = true;
        car.parking_brake = false;
        run(&mut car, &terrain, 1.0);
        car.controls = Controls {
            throttle: 1.0,
            ..default()
        };
        // Flat out from a standstill: the gears only ever go up (the clutch closing after
        // a shift spins the wheels up for a moment, which mustn't trigger another shift).
        // On flat asphalt: off the road it would slow down, and shifting down is right.
        let dt = 1.0 / CHASSIS_HZ as f32;
        let mut gears = vec![car.drivetrain.selected_gear()];
        for _ in 0..(9.0 / dt) as usize {
            step_car(&mut car, &terrain, dt);
            let gear = car.drivetrain.selected_gear();
            if gear != *gears.last().unwrap() {
                gears.push(gear);
            }
        }
        assert!(gears.windows(2).all(|w| w[1] > w[0]), "gears {gears:?}");
        assert!(gears.len() >= 3, "gears {gears:?}");
    }

    /// Full brakes from 90 km/h in the car as it starts (cold tires), with ABS on or off:
    /// the distance to stop, the least share of the road speed a wheel turned at, and how
    /// far steering turned the car on the way (rad).
    fn emergency_stop(chassis: usize, abs: bool, steer: f32) -> (f32, f32, f32) {
        let terrain = Terrain::flat();
        let mut car = Car::spawn_at(0, CarSetup::factory(chassis, 0), &terrain, 0.0, 0.0, 0.0);
        car.abs = abs;
        car.drivetrain.shift_to(&car.setup.engine, 4);
        run(&mut car, &terrain, 1.0);
        car.body.velocity = car.body.rotation * Vec3::NEG_Z * 25.0;
        for corner in car.corners.iter_mut() {
            corner.omega = 25.0 / car.setup.axle(corner.front).tire.radius;
        }
        car.controls = Controls {
            brake: 1.0,
            steer,
            ..default()
        };
        let dt = 1.0 / CHASSIS_HZ as f32;
        let (start, heading_before) = (car.body.position, heading(car.body.rotation));
        let mut turning = f32::MAX;
        while car.body.velocity.length() > 0.1 {
            step_car(&mut car, &terrain, dt);
            let speed = car.forward_speed();
            if speed > 5.0 {
                for corner in &car.corners {
                    let radius = car.setup.axle(corner.front).tire.radius;
                    turning = turning.min(corner.omega * radius / speed);
                }
            }
        }
        let turned = (heading(car.body.rotation) - heading_before + PI).rem_euclid(TAU) - PI;
        let turned = turned.abs();
        (car.body.position.distance(start), turning, turned)
    }

    #[test]
    fn abs_keeps_the_wheels_turning() {
        for (chassis, spec) in crate::catalogue::builtin().chassis.iter().enumerate() {
            let name = &spec.name;
            let (with, turning, _) = emergency_stop(chassis, true, 0.0);
            let (without, locked, _) = emergency_stop(chassis, false, 0.0);
            println!(
                "{name}: {with:.1} m with ABS (wheels at least {turning:.2} of the road \
                 speed), {without:.1} m without ({locked:.2})"
            );
            assert!(
                turning > 0.7,
                "{name}: a wheel at {turning:.2} of the road speed"
            );
            assert!(
                with < without * 1.03,
                "{name}: {with:.1} m, {without:.1} m without"
            );
            // Steering while braking: the wheels keep turning, and so does the car.
            let (_, turning, turned) = emergency_stop(chassis, true, 0.1);
            let (_, locked, straight) = emergency_stop(chassis, false, 0.1);
            println!(
                "    steering: turned {turned:.2} rad with ABS (wheels {turning:.2}), \
                 {straight:.2} without ({locked:.2})"
            );
            assert!(turning > 0.7, "{name}: a wheel at {turning:.2} steering");
            assert!(turned > 0.2, "{name}: turned {turned:.2} rad");
        }
    }

    #[test]
    fn brakes_without_rear_lock_or_spinning() {
        // On flat asphalt: the slight steering would take it off a real road.
        let terrain = Terrain::flat();
        let (p, yaw) = (Vec2::new(0.0, 300.0), 0.0);
        // Every car as it leaves the factory, and on the track suspension and tires.
        let factory = (0..crate::catalogue::builtin().chassis.len())
            .map(|chassis| CarSetup::factory(chassis, 0));
        let track = (0..crate::catalogue::builtin().chassis.len()).map(|chassis| {
            let stock = CarSetup::factory(chassis, 0);
            let build = stock.build.clone().with_suspension(2).on_compound(RACE);
            CarSetup::new(build, stock.engine.parts, stock.engine.build)
        });
        for setup in factory.chain(track) {
            let name = format!("{} ({})", setup.name, setup.build.suspension().name);
            let mut car = Car::spawn_at(0, setup, &terrain, p.x, p.y, yaw);
            car.warm_tires();
            // The brakes' balance on its own.
            car.abs = false;
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
                // from, and a third: the weight shifts forward, and short cars on grippy
                // tires turn in for a moment before settling (it's spinning that's wrong).
                let wheelbase = car.setup.rear_z - car.setup.front_z;
                let asked = 25.0 * 0.05 * car.setup.max_steer / wheelbase;
                let yaw_rate = car.body.angular_velocity.y.abs();
                assert!(
                    yaw_rate < 1.35 * asked,
                    "{name}: yaw rate {yaw_rate:.2} at {time:.2} s"
                );
            }
        }
    }
}

#[cfg(test)]
mod handling;
