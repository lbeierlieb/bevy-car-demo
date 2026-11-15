//! Procedural low-poly car meshes, posed every frame from the simulated linkage.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::car::Car;
use crate::physics::suspension::CornerPose;

/// Fore-aft spacing of each A-arm's two inner mounts.
const ARM_SPREAD: f32 = 0.18;

pub struct VisualsPlugin;

impl Plugin for VisualsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (add_car_meshes, pose_parts));
    }
}

#[derive(Clone, Copy)]
enum PartKind {
    Wheel,
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
        let mut flat = |color: Color| {
            materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: 0.7,
                ..default()
            })
        };
        let paint = flat(car.setup.color);
        let glass = flat(Color::srgb(0.12, 0.14, 0.18));
        let rubber = flat(Color::srgb(0.08, 0.08, 0.08));
        let metal = flat(Color::srgb(0.55, 0.57, 0.60));
        let spring = flat(Color::srgb(0.90, 0.30, 0.10));

        // Side profiles (z, y) of the tub and cabin, extruded across the car. Forward is -z.
        let tub = prism(
            &[
                [-2.05, -0.12],
                [-1.85, -0.26],
                [1.85, -0.26],
                [2.0, -0.05],
                [1.95, 0.14],
                [-1.6, 0.04],
            ],
            0.36,
        );
        let cabin = prism(
            &[[-0.85, 0.06], [1.3, 0.12], [1.05, 0.40], [0.05, 0.42]],
            0.32,
        );
        let wheel = Cylinder::new(1.0, 1.0).mesh().resolution(10).build();
        let bar = Cuboid::new(1.0, 1.0, 1.0);

        commands
            .entity(entity)
            .insert(Visibility::default())
            .with_children(|parent| {
                parent.spawn((Mesh3d(meshes.add(tub)), MeshMaterial3d(paint)));
                parent.spawn((Mesh3d(meshes.add(cabin)), MeshMaterial3d(glass)));
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
                    part(PartKind::Wheel, &wheel, &rubber);
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
        let tire = &car.setup.axle(corner.front).tire;
        *transform = match part.kind {
            PartKind::Wheel => Transform {
                translation: pose.wheel_center,
                // Spin about the axle (x); the cylinder's own axis is y.
                rotation: pose.upright
                    * Quat::from_rotation_x(-corner.spin)
                    * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
                scale: Vec3::new(tire.radius, 0.22, tire.radius),
            },
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

/// Flat-shaded prism: a convex side profile of (z, y) points, extruded from -x to +x.
fn prism(profile: &[[f32; 2]], half_width: f32) -> Mesh {
    let point = |i: usize, x: f32| {
        let [z, y] = profile[i % profile.len()];
        Vec3::new(x, y, z)
    };
    let center = point(0, 0.0).lerp(point(profile.len() / 2, 0.0), 0.5);
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    // Each triangle is wound so that its normal faces away from the prism's centre.
    let mut tri = |a: Vec3, b: Vec3, c: Vec3| {
        let mut n = (b - a).cross(c - a).normalize();
        let (b, c) = if n.dot((a + b + c) / 3.0 - center) < 0.0 {
            n = -n;
            (c, b)
        } else {
            (b, c)
        };
        for v in [a, b, c] {
            positions.push(v.to_array());
            normals.push(n.to_array());
        }
    };
    for i in 0..profile.len() {
        let (w, j) = (half_width, i + 1);
        tri(point(i, -w), point(j, -w), point(j, w));
        tri(point(i, -w), point(j, w), point(i, w));
        if i >= 1 && j < profile.len() {
            tri(point(0, w), point(i, w), point(j, w));
            tri(point(0, -w), point(i, -w), point(j, -w));
        }
    }
    let indices = (0..positions.len() as u32).collect();
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}
