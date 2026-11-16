//! The cars' bodies (see `body`), and their wheels and suspension, posed every frame from
//! the simulated linkage.

use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;

use crate::car::Car;
use crate::physics::suspension::CornerPose;

/// Fore-aft spacing of each A-arm's two inner mounts.
const ARM_SPREAD: f32 = 0.18;
/// The rim's radius relative to the tire's.
const RIM: f32 = 0.64;

pub struct VisualsPlugin;

impl Plugin for VisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (add_car_meshes, pose_parts));
    }
}

#[derive(Clone, Copy)]
enum PartKind {
    Tire,
    Rim,
    /// One of the rim's two crossed spokes, at this angle around the axle.
    Spoke(f32),
    Upright,
    Coilover,
    /// The two links of an A-arm: (upper arm?, front link?).
    ArmLink {
        upper: bool,
        front: bool,
    },
}

#[derive(Component)]
struct Part {
    corner: usize,
    kind: PartKind,
}

fn add_car_meshes(
    mut commands: Commands,
    cars: Query<(Entity, &Car), Added<Car>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, car) in &cars {
        let mut flat = |color: Color, roughness: f32| {
            materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: roughness,
                ..default()
            })
        };
        // The body's colours are in its vertices.
        let paint = flat(Color::WHITE, 0.45);
        let rubber = flat(Color::srgb(0.06, 0.06, 0.06), 0.9);
        let alloy = flat(Color::srgb(0.62, 0.64, 0.67), 0.4);
        let dark = flat(Color::srgb(0.12, 0.12, 0.13), 0.6);
        let metal = flat(Color::srgb(0.4, 0.42, 0.45), 0.7);
        let spring = flat(Color::srgb(0.90, 0.30, 0.10), 0.7);

        let chassis = car.setup.build.chassis();
        let body = chassis.body.mesh(&chassis.wheels(), car.setup.color);
        let wheel = Cylinder::new(1.0, 1.0).mesh().resolution(14).build();
        let bar = Cuboid::new(1.0, 1.0, 1.0);

        commands
            .entity(entity)
            .insert(Visibility::default())
            .with_children(|parent| {
                parent.spawn((
                    Mesh3d(meshes.add(body)),
                    MeshMaterial3d(paint),
                    Transform::from_translation(car.setup.body_offset),
                ));
                let wheel = meshes.add(wheel);
                let bar = meshes.add(bar);
                for corner in 0..4 {
                    let mut part =
                        |kind, mesh: &Handle<Mesh>, material: &Handle<StandardMaterial>| {
                            parent.spawn((
                                Part { corner, kind },
                                Mesh3d(mesh.clone()),
                                MeshMaterial3d(material.clone()),
                            ));
                        };
                    part(PartKind::Tire, &wheel, &rubber);
                    part(PartKind::Rim, &wheel, &alloy);
                    part(PartKind::Spoke(0.0), &bar, &dark);
                    part(PartKind::Spoke(FRAC_PI_2), &bar, &dark);
                    part(PartKind::Upright, &bar, &metal);
                    part(PartKind::Coilover, &bar, &spring);
                    for upper in [false, true] {
                        for front in [false, true] {
                            part(PartKind::ArmLink { upper, front }, &bar, &metal);
                        }
                    }
                }
            });
    }
}

/// Parts are children of the car, so they're posed directly in the chassis frame.
fn pose_parts(cars: Query<&Car>, mut parts: Query<(&Part, &ChildOf, &mut Transform)>) {
    for (part, child_of, mut transform) in &mut parts {
        let Ok(car) = cars.get(child_of.parent()) else {
            continue;
        };
        let corner = &car.corners[part.corner];
        let pose: &CornerPose = &corner.pose;
        let radius = car.setup.axle(corner.front).tire.radius;
        let width = car.setup.build.chassis().tire_width[usize::from(!corner.front)];
        // Spinning about the axle (x); a cylinder's own axis is y.
        let spinning =
            pose.upright * Quat::from_rotation_x(-corner.spin) * Quat::from_rotation_z(FRAC_PI_2);
        let wheel = |rotation: Quat, scale: Vec3| Transform {
            translation: pose.wheel_center,
            rotation,
            scale,
        };
        *transform = match part.kind {
            PartKind::Tire => wheel(spinning, Vec3::new(radius, width, radius)),
            // The rim and its spokes stand a little proud of the tire's sides.
            PartKind::Rim => wheel(
                spinning,
                Vec3::new(RIM * radius, width + 0.01, RIM * radius),
            ),
            PartKind::Spoke(angle) => wheel(
                spinning * Quat::from_rotation_y(angle),
                Vec3::new(1.9 * RIM * radius, width + 0.02, 0.22 * radius),
            ),
            PartKind::Upright => bar(pose.lower_ball, pose.upper_ball, 0.06),
            PartKind::Coilover => bar(pose.spring_bottom, pose.spring_top, 0.07),
            PartKind::ArmLink { upper, front } => {
                let (pivot, ball) = if upper {
                    (pose.upper_pivot, pose.upper_ball)
                } else {
                    (pose.lower_pivot, pose.lower_ball)
                };
                let offset = Vec3::Z * if front { -ARM_SPREAD } else { ARM_SPREAD };
                bar(pivot + offset, ball, 0.035)
            }
        };
    }
}

/// A unit cube stretched into a bar from `a` to `b`.
pub(crate) fn bar(a: Vec3, b: Vec3, thickness: f32) -> Transform {
    let d = b - a;
    Transform {
        translation: (a + b) / 2.0,
        rotation: Quat::from_rotation_arc(Vec3::Y, d.normalize()),
        scale: Vec3::new(thickness, d.length(), thickness),
    }
}
