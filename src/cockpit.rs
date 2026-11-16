//! Low-poly interior for the driver view: dashboard, pillars, a steering wheel that turns
//! with the front wheels, and a rev counter and speedometer with live needles. It sits
//! inside the opaque cabin, so it's only seen from the driver's seat.

use std::f32::consts::{FRAC_PI_2, PI};

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::camera::EYE;
use crate::car::Car;
use crate::input::WheelConfig;
use crate::visuals::bar;

/// Gauge faces lean back towards the driver; the speedometer reads up to 260 km/h.
const GAUGE_TILT: f32 = 0.35;
const GAUGE_RADIUS: f32 = 0.045;
const SPEEDO_TOP: f32 = 260.0;

pub struct CockpitPlugin;

impl Plugin for CockpitPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (add_interior, move_dials));
    }
}

#[derive(Clone, Copy)]
enum Dial {
    Tach,
    Speedo,
    Steering,
}

/// A part that turns with a dial: posed as `mount * turn * local` in the chassis frame.
#[derive(Component)]
struct DialPart {
    dial: Dial,
    mount: Transform,
    local: Transform,
}

fn tach_mount() -> Transform {
    gauge_mount(-0.225)
}

fn speedo_mount() -> Transform {
    gauge_mount(-0.115)
}

fn gauge_mount(x: f32) -> Transform {
    Transform::from_xyz(x, 0.175, -0.012).with_rotation(Quat::from_rotation_x(-GAUGE_TILT))
}

/// The steering wheel's hub; the wheel turns about its local y axis, which points at
/// the driver, and its local +z points down.
fn wheel_mount() -> Transform {
    Transform::from_xyz(EYE.x, 0.17, 0.12)
        .with_rotation(Quat::from_rotation_x(-GAUGE_TILT) * Quat::from_rotation_x(FRAC_PI_2))
}

/// Needle angle for a reading from 0 to 1: 270° clockwise from the lower left, like the HUD.
fn needle_angle(fraction: f32) -> f32 {
    1.25 * PI - 1.5 * PI * fraction.clamp(0.0, 1.0)
}

fn add_interior(
    mut commands: Commands,
    cars: Query<(Entity, &Car), Added<Car>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut lit = |color: Color| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.8,
            ..default()
        })
    };
    let trim = lit(Color::srgb(0.07, 0.07, 0.08));
    let plastic = lit(Color::srgb(0.13, 0.13, 0.14));
    let mirror = lit(Color::srgb(0.5, 0.55, 0.6));
    let headliner = lit(Color::srgb(0.45, 0.45, 0.43));
    // Gauges are unlit, so they stay readable in the cabin's shadow.
    let mut unlit = |color: Color| {
        materials.add(StandardMaterial {
            base_color: color,
            unlit: true,
            ..default()
        })
    };
    let face = unlit(Color::srgb(0.03, 0.03, 0.04));
    let bezel = unlit(Color::srgb(0.35, 0.36, 0.38));
    let mark = unlit(Color::srgb(0.9, 0.9, 0.88));
    let red = unlit(Color::srgb(0.9, 0.15, 0.1));
    let needle = unlit(Color::srgb(1.0, 0.45, 0.1));

    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let disc = meshes.add(Circle::new(1.0).mesh().resolution(24));
    let rim = meshes.add(
        Torus::new(0.118, 0.142)
            .mesh()
            .minor_resolution(6)
            .major_resolution(24),
    );
    let hub = meshes.add(Cylinder::new(0.03, 0.03).mesh().resolution(10));

    for (entity, car) in &cars {
        commands.entity(entity).with_children(|parent| {
            let mut fixed = |transform: Transform, mesh: &Handle<Mesh>, material: &Handle<_>| {
                parent.spawn((
                    transform,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    NotShadowCaster,
                ));
            };
            let block =
                |center: Vec3, size: Vec3| Transform::from_translation(center).with_scale(size);
            // Dashboard with the instrument binnacle on top, roof lining, windscreen
            // pillars and header, rear-view mirror.
            fixed(
                block(Vec3::new(0.0, 0.11, -0.225), Vec3::new(0.62, 0.1, 0.35)),
                &cube,
                &plastic,
            );
            fixed(
                block(Vec3::new(-0.17, 0.175, -0.08), Vec3::new(0.24, 0.07, 0.08)),
                &cube,
                &plastic,
            );
            fixed(
                block(Vec3::new(0.0, 0.395, 0.55), Vec3::new(0.6, 0.01, 0.9)),
                &cube,
                &headliner,
            );
            for x in [-0.29, 0.29] {
                let pillar = bar(Vec3::new(x, 0.07, -0.8), Vec3::new(x, 0.395, 0.03), 0.04);
                fixed(pillar, &cube, &trim);
            }
            let header = bar(
                Vec3::new(-0.3, 0.395, 0.04),
                Vec3::new(0.3, 0.395, 0.04),
                0.04,
            );
            fixed(header, &cube, &trim);
            fixed(
                block(Vec3::new(0.0, 0.36, 0.0), Vec3::new(0.12, 0.035, 0.012)),
                &cube,
                &mirror,
            );

            // Gauge faces with a tick every thousand rpm and every 20 km/h.
            let redline = car.setup.engine.redline_rpm();
            let tach_top = car.setup.engine.rev_scale();
            for (mount, ticks, red_from) in [
                (
                    tach_mount(),
                    (tach_top / 1000.0) as usize,
                    redline / tach_top,
                ),
                (speedo_mount(), (SPEEDO_TOP / 20.0) as usize, 2.0),
            ] {
                let on_face =
                    |z: f32, size: Vec3| mount * Transform::from_xyz(0.0, 0.0, z).with_scale(size);
                fixed(
                    on_face(-0.001, Vec3::splat(GAUGE_RADIUS + 0.004)),
                    &disc,
                    &bezel,
                );
                fixed(on_face(0.0, Vec3::splat(GAUGE_RADIUS)), &disc, &face);
                for k in 0..=ticks {
                    let fraction = k as f32 / ticks as f32;
                    let angle = needle_angle(fraction);
                    let tick = mount
                        * Transform::from_rotation(Quat::from_rotation_z(angle))
                        * Transform::from_xyz(GAUGE_RADIUS - 0.007, 0.0, 0.001)
                            .with_scale(Vec3::new(0.008, 0.0025, 0.001));
                    let color = if fraction >= red_from - 1e-3 {
                        &red
                    } else {
                        &mark
                    };
                    fixed(tick, &cube, color);
                }
            }

            let mut dial = |dial: Dial,
                            mount: Transform,
                            local: Transform,
                            mesh: &Handle<Mesh>,
                            material: &Handle<_>| {
                parent.spawn((
                    DialPart { dial, mount, local },
                    mount * local,
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(material.clone()),
                    NotShadowCaster,
                ));
            };
            for (kind, mount) in [(Dial::Tach, tach_mount()), (Dial::Speedo, speedo_mount())] {
                let local = Transform::from_xyz(0.014, 0.0, 0.003)
                    .with_scale(Vec3::new(0.042, 0.003, 0.001));
                dial(kind, mount, local, &cube, &needle);
            }
            // Steering wheel: rim, hub, and three spokes.
            let wheel = wheel_mount();
            dial(Dial::Steering, wheel, Transform::IDENTITY, &rim, &trim);
            dial(Dial::Steering, wheel, Transform::IDENTITY, &hub, &plastic);
            for local in [
                block(Vec3::ZERO, Vec3::new(0.25, 0.012, 0.025)),
                block(Vec3::new(0.0, 0.0, 0.065), Vec3::new(0.025, 0.012, 0.12)),
            ] {
                dial(Dial::Steering, wheel, local, &cube, &plastic);
            }
        });
    }
}

fn move_dials(
    config: Res<WheelConfig>,
    cars: Query<&Car>,
    mut parts: Query<(&DialPart, &ChildOf, &mut Transform)>,
) {
    for (part, child_of, mut transform) in &mut parts {
        let Ok(car) = cars.get(child_of.parent()) else {
            continue;
        };
        let turn = match part.dial {
            Dial::Tach => {
                let top = car.setup.engine.rev_scale();
                Quat::from_rotation_z(needle_angle(car.drivetrain.rpm / top))
            }
            Dial::Speedo => {
                let kmh = car.forward_speed().abs() * 3.6;
                Quat::from_rotation_z(needle_angle(kmh / SPEEDO_TOP))
            }
            // Like a real wheel set to the configured rotation range.
            Dial::Steering => {
                Quat::from_rotation_y(car.controls.steer * config.steering_wheel_lock())
            }
        };
        *transform = part.mount * Transform::from_rotation(turn) * part.local;
    }
}
