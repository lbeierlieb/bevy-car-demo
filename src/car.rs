//! Car setups, spawning and driver input.

use bevy::prelude::*;

use crate::physics::drivetrain::{Drivetrain, EngineSetup};
use crate::physics::rigid_body::RigidBody;
use crate::physics::suspension::{ArmGeometry, AxleSetup, Corner};
use crate::physics::tire::TireParams;
use crate::terrain::Terrain;

#[derive(Clone, Debug)]
pub struct CarSetup {
    pub name: &'static str,
    pub color: Color,
    pub sprung_mass: f32,
    /// Box used to approximate the chassis inertia.
    pub inertia_box: Vec3,
    /// Axle positions along the chassis z axis (forward is -z).
    pub front_z: f32,
    pub rear_z: f32,
    pub front: AxleSetup,
    pub rear: AxleSetup,
    pub max_steer: f32,
    /// Engine, clutch and gearbox, driving the rear wheels.
    pub engine: EngineSetup,
    /// Total brake torque, the front axle's share of it, and handbrake torque per rear wheel.
    pub brake_torque: f32,
    pub brake_bias: f32,
    pub handbrake_torque: f32,
}

impl CarSetup {
    pub fn baseline() -> Self {
        let axle = AxleSetup {
            geometry: ArmGeometry::default(),
            spring_rate: 90_000.0,
            damping: 5_000.0,
            unsprung_mass: 40.0,
            wheel_inertia: 1.2,
            tire: TireParams::default(),
        };
        Self {
            name: "Baseline",
            color: Color::srgb(0.85, 0.20, 0.15),
            sprung_mass: 1_100.0,
            inertia_box: Vec3::new(1.6, 0.8, 4.0),
            front_z: -1.3,
            rear_z: 1.3,
            front: axle,
            rear: AxleSetup {
                spring_rate: 65_000.0,
                ..axle
            },
            max_steer: 0.5,
            engine: EngineSetup::default(),
            brake_torque: 3_500.0,
            brake_bias: 0.7,
            handbrake_torque: 2_500.0,
        }
    }

    /// Soft springs and a short upper arm: lots of roll, lots of camber gain.
    pub(crate) fn soft() -> Self {
        let mut setup = Self::baseline();
        setup.name = "Soft";
        setup.color = Color::srgb(0.20, 0.45, 0.85);
        for axle in [&mut setup.front, &mut setup.rear] {
            axle.spring_rate *= 0.55;
            axle.damping *= 0.6;
            axle.geometry.upper_pivot.x = 0.50;
        }
        setup
    }

    /// Stiff springs, more static camber and a long upper arm (little camber gain). The
    /// stiff rear unloads quickly under braking, so the brakes are biased further forward.
    pub(crate) fn track() -> Self {
        let mut setup = Self::baseline();
        setup.name = "Track";
        setup.color = Color::srgb(0.95, 0.75, 0.10);
        for axle in [&mut setup.front, &mut setup.rear] {
            axle.spring_rate *= 1.8;
            axle.damping *= 1.6;
            axle.geometry.upper_pivot.x = 0.32;
            axle.geometry.static_camber = -0.045;
            axle.tire.mu = 1.25;
        }
        setup.brake_bias = 0.8;
        setup
    }

    pub fn axle(&self, front: bool) -> &AxleSetup {
        if front { &self.front } else { &self.rear }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Controls {
    /// 0 to 1 (full throttle).
    pub throttle: f32,
    pub brake: f32,
    /// -1 (right) to 1 (left).
    pub steer: f32,
    pub handbrake: bool,
}

#[derive(Component)]
pub struct Car {
    /// Position in the Tab cycling order.
    pub index: usize,
    pub setup: CarSetup,
    pub body: RigidBody,
    pub corners: [Corner; 4],
    pub drivetrain: Drivetrain,
    pub controls: Controls,
}

impl Car {
    /// A car at rest, slightly above the ground at (x, z), facing along `yaw`.
    pub fn spawn_at(
        index: usize,
        setup: CarSetup,
        terrain: &Terrain,
        x: f32,
        z: f32,
        yaw: f32,
    ) -> Self {
        let position = Vec3::new(x, terrain.sample(x, z).0 + 0.6, z);
        Self::new(index, setup, position, Quat::from_rotation_y(yaw))
    }

    fn new(index: usize, setup: CarSetup, position: Vec3, rotation: Quat) -> Self {
        let mass = setup.sprung_mass;
        let body = RigidBody::new(
            position,
            rotation,
            mass,
            RigidBody::box_inertia(mass, setup.inertia_box),
        );
        let corners =
            [(-1.0, true), (1.0, true), (-1.0, false), (1.0, false)].map(|(side, front)| {
                let z = if front { setup.front_z } else { setup.rear_z };
                Corner::new(setup.axle(front), side, z, front)
            });
        Self {
            index,
            drivetrain: Drivetrain::new(&setup.engine),
            setup,
            body,
            corners,
            controls: Controls::default(),
        }
    }

    /// Places the car upright, at rest, just above the ground at its current spot.
    fn reset(&mut self, terrain: &Terrain) {
        let p = self.body.position;
        let yaw = heading(self.body.rotation);
        let automatic = self.drivetrain.automatic;
        *self = Self::spawn_at(self.index, self.setup.clone(), terrain, p.x, p.z, yaw);
        self.drivetrain.automatic = automatic;
    }

    pub fn forward_speed(&self) -> f32 {
        self.body.velocity.dot(self.body.rotation * Vec3::NEG_Z)
    }
}

/// Yaw angle of an orientation's forward (-z) direction.
pub fn heading(rotation: Quat) -> f32 {
    let forward = rotation * Vec3::NEG_Z;
    (-forward.x).atan2(-forward.z)
}

/// The car currently driven (and followed by the camera).
#[derive(Resource, Default)]
pub struct ActiveCar(pub usize);

pub struct CarPlugin;

impl Plugin for CarPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveCar>()
            .add_systems(Startup, spawn_cars)
            .add_systems(
                Update,
                (switch_car, drive, reset_car, sync_transforms).chain(),
            );
    }
}

fn spawn_cars(mut commands: Commands, terrain: Res<Terrain>) {
    // One behind the other along the road at the spawn point, the first car in front.
    let setups = [CarSetup::baseline(), CarSetup::soft(), CarSetup::track()];
    let spawns = terrain.road_spawns(setups.len(), 9.0);
    for (index, (setup, (p, yaw))) in setups.into_iter().zip(spawns).enumerate() {
        let car = Car::spawn_at(index, setup, &terrain, p.x, p.y, yaw);
        commands.spawn((Transform::from_translation(car.body.position), car));
    }
}

fn switch_car(keys: Res<ButtonInput<KeyCode>>, mut active: ResMut<ActiveCar>, cars: Query<&Car>) {
    if keys.just_pressed(KeyCode::Tab) {
        active.0 = (active.0 + 1) % cars.iter().len().max(1);
    }
}

/// Keyboard to controls for the active car; parked cars hold all brakes. G switches all
/// cars between manual and automatic gearboxes.
fn drive(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
) {
    let pressed = |a, b| keys.pressed(a) || keys.pressed(b);
    let toggle_automatic = keys.just_pressed(KeyCode::KeyG);
    for mut car in &mut cars {
        car.drivetrain.automatic ^= toggle_automatic;
        if car.index != active.0 {
            car.controls = Controls {
                brake: 1.0,
                handbrake: true,
                ..default()
            };
            continue;
        }
        let speed = car.forward_speed();
        let forward = pressed(KeyCode::KeyW, KeyCode::ArrowUp);
        let back = pressed(KeyCode::KeyS, KeyCode::ArrowDown);
        let steer_target = pressed(KeyCode::KeyA, KeyCode::ArrowLeft) as i32 as f32
            - pressed(KeyCode::KeyD, KeyCode::ArrowRight) as i32 as f32;

        let Car {
            setup,
            corners,
            drivetrain,
            controls: c,
            ..
        } = &mut *car;
        let gear = drivetrain.selected_gear();
        let (go, stop) = if drivetrain.automatic {
            // W drives forward and S backward: each brakes while rolling the other way
            // and selects first or reverse once (nearly) stopped.
            let reverse = gear < 0;
            let (go, stop) = if reverse {
                (back, forward)
            } else {
                (forward, back)
            };
            let along = if reverse { -speed } else { speed };
            if stop && !go && along < 1.0 {
                drivetrain.shift_to(&setup.engine, if reverse { 1 } else { -1 });
            }
            (go, stop)
        } else {
            // Q/E shift down/up. Reverse only engages (nearly) stopped, and downshifts
            // that would over-rev the engine are refused.
            let target = gear + keys.just_pressed(KeyCode::KeyE) as i8
                - keys.just_pressed(KeyCode::KeyQ) as i8;
            let wheel_omega = (corners[2].omega + corners[3].omega) / 2.0;
            let over_rev =
                target > 0 && setup.engine.rpm_in(target, wheel_omega) > setup.engine.redline_rpm;
            if target != gear && !over_rev && (target >= 0 || speed.abs() < 2.0) {
                drivetrain.shift_to(&setup.engine, target);
            }
            (forward, back)
        };
        (c.throttle, c.brake) = match (go, stop) {
            (true, _) => (1.0, stop as u8 as f32),
            (false, true) => (0.0, 1.0),
            // Hill hold: without input, keep a (nearly) stopped car from rolling away.
            _ if speed.abs() < 2.0 => (0.0, 1.0),
            _ => (0.0, 0.0),
        };
        c.handbrake = keys.pressed(KeyCode::Space);
        // Steering is rate-limited, like turning a real wheel.
        let max_delta = 3.0 * time.delta_secs();
        c.steer += (steer_target - c.steer).clamp(-max_delta, max_delta);
    }
}

fn reset_car(
    keys: Res<ButtonInput<KeyCode>>,
    active: Res<ActiveCar>,
    terrain: Res<Terrain>,
    mut cars: Query<&mut Car>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        for mut car in cars.iter_mut().filter(|car| car.index == active.0) {
            car.reset(&terrain);
        }
    }
}

pub fn sync_transforms(mut cars: Query<(&Car, &mut Transform)>) {
    for (car, mut transform) in &mut cars {
        transform.translation = car.body.position;
        transform.rotation = car.body.rotation;
    }
}
