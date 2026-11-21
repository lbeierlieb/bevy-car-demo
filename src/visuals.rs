//! The cars' bodies (see `body`), and their wheels and suspension, made from each car's
//! `Looks` and posed every frame from its `Motion`.

use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;

use bevy::prelude::*;

use crate::looks::{Looks, Motion};

/// Fore-aft spacing of each A-arm's two inner mounts.
const ARM_SPREAD: f32 = 0.18;
/// How far the rim's flange stands out past its bead seat (m).
const FLANGE: f32 = 0.012;

pub struct VisualsPlugin;

impl Plugin for VisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (add_car_meshes, pose_parts).after(crate::looks::show_cars),
        );
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

/// A car's body mesh.
#[derive(Component)]
struct Body;

#[derive(Component)]
struct Part {
    corner: usize,
    kind: PartKind,
}

/// A car's meshes (its body and parts), which go when its looks change.
type CarMeshes = Or<(With<Body>, With<Part>)>;

/// What every car's meshes share: the materials, the wheel and bar meshes, and each body
/// (made once per chassis and paint), so that many cars (traffic) cost little and batch.
#[derive(Default)]
struct Shared {
    materials: Option<Materials>,
    wheel: Handle<Mesh>,
    bar: Handle<Mesh>,
    bodies: HashMap<(usize, usize), Handle<Mesh>>,
}

struct Materials {
    paint: Handle<StandardMaterial>,
    rubber: Handle<StandardMaterial>,
    alloy: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
    metal: Handle<StandardMaterial>,
    spring: Handle<StandardMaterial>,
}

/// The meshes of every car whose looks are new or changed (the old ones go).
fn add_car_meshes(
    mut commands: Commands,
    cars: Query<(Entity, &Looks, Option<&Children>), Changed<Looks>>,
    old: Query<(), CarMeshes>,
    mut shared: Local<Shared>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let shared = &mut *shared;
    if shared.materials.is_none() {
        let mut flat = |color: Color, roughness: f32| {
            materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: roughness,
                ..default()
            })
        };
        shared.materials = Some(Materials {
            // The body's colours are in its vertices.
            paint: flat(Color::WHITE, 0.45),
            rubber: flat(Color::srgb(0.06, 0.06, 0.06), 0.9),
            alloy: flat(Color::srgb(0.62, 0.64, 0.67), 0.4),
            dark: flat(Color::srgb(0.12, 0.12, 0.13), 0.6),
            metal: flat(Color::srgb(0.4, 0.42, 0.45), 0.7),
            spring: flat(Color::srgb(0.90, 0.30, 0.10), 0.7),
        });
        shared.wheel = meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(14).build());
        shared.bar = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    }
    let m = shared.materials.as_ref().unwrap();
    for (entity, looks, children) in &cars {
        for &child in children.into_iter().flatten() {
            if old.contains(child) {
                commands.entity(child).despawn();
            }
        }
        let build = &looks.build;
        let body = (shared.bodies)
            .entry((build.chassis, build.paint))
            .or_insert_with(|| {
                let chassis = build.chassis();
                meshes.add(chassis.body.mesh(&chassis.wheels(), build.paint()))
            })
            .clone();
        let (wheel, bar) = (&shared.wheel, &shared.bar);
        commands
            .entity(entity)
            .insert(Visibility::default())
            .with_children(|parent| {
                parent.spawn((
                    Body,
                    Mesh3d(body),
                    MeshMaterial3d(m.paint.clone()),
                    Transform::from_translation(looks.body_offset),
                ));
                for corner in 0..4 {
                    let mut part =
                        |kind, mesh: &Handle<Mesh>, material: &Handle<StandardMaterial>| {
                            parent.spawn((
                                Part { corner, kind },
                                Mesh3d(mesh.clone()),
                                MeshMaterial3d(material.clone()),
                            ));
                        };
                    part(PartKind::Tire, wheel, &m.rubber);
                    part(PartKind::Rim, wheel, &m.alloy);
                    part(PartKind::Spoke(0.0), bar, &m.dark);
                    part(PartKind::Spoke(FRAC_PI_2), bar, &m.dark);
                    part(PartKind::Upright, bar, &m.metal);
                    part(PartKind::Coilover, bar, &m.spring);
                    for upper in [false, true] {
                        for front in [false, true] {
                            part(PartKind::ArmLink { upper, front }, bar, &m.metal);
                        }
                    }
                }
            });
    }
}

/// Parts are children of the car, so they're posed directly in the chassis frame.
fn pose_parts(cars: Query<(&Looks, &Motion)>, mut parts: Query<(&Part, &ChildOf, &mut Transform)>) {
    for (part, child_of, mut transform) in &mut parts {
        let Ok((looks, motion)) = cars.get(child_of.parent()) else {
            continue;
        };
        let wheel_motion = &motion.wheels[part.corner];
        let pose = looks.corner_pose(part.corner, wheel_motion);
        let axle = part.corner / 2;
        let radius = looks.axles[axle].radius;
        let wheel_size = &looks.build.wheels[axle];
        let width = wheel_size.tire.width_m();
        let rim = wheel_size.rim.radius() + FLANGE;
        // Spinning about the axle (x); a cylinder's own axis is y.
        let spinning = pose.upright
            * Quat::from_rotation_x(-wheel_motion.spin)
            * Quat::from_rotation_z(FRAC_PI_2);
        let wheel = |rotation: Quat, scale: Vec3| Transform {
            translation: pose.wheel_center,
            rotation,
            scale,
        };
        *transform = match part.kind {
            PartKind::Tire => wheel(spinning, Vec3::new(radius, width, radius)),
            // The rim and its spokes stand a little proud of the tire's sides.
            PartKind::Rim => wheel(spinning, Vec3::new(rim, width + 0.01, rim)),
            PartKind::Spoke(angle) => wheel(
                spinning * Quat::from_rotation_y(angle),
                Vec3::new(1.9 * rim, width + 0.02, 0.22 * radius),
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
