//! Chase camera that follows the active car; drag with the left mouse button to orbit,
//! scroll to zoom.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

use crate::car::{ActiveCar, Car, heading};
use crate::terrain::Terrain;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OrbitInput>().add_systems(
            Update,
            (orbit, follow).chain().after(crate::car::sync_transforms),
        );
    }
}

#[derive(Component)]
pub struct ChaseCamera {
    /// Smoothed car heading the camera trails behind.
    yaw: f32,
    orbit_yaw: f32,
    pitch: f32,
    distance: f32,
}

impl Default for ChaseCamera {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            orbit_yaw: 0.0,
            pitch: 0.3,
            distance: 9.0,
        }
    }
}

/// Whether mouse input may drive the camera (the UI clears this while it uses the pointer).
#[derive(Resource)]
pub struct OrbitInput(pub bool);

impl Default for OrbitInput {
    fn default() -> Self {
        Self(true)
    }
}

fn orbit(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    enabled: Res<OrbitInput>,
    mut cameras: Query<&mut ChaseCamera>,
) {
    if !enabled.0 {
        return;
    }
    for mut camera in &mut cameras {
        if buttons.pressed(MouseButton::Left) {
            camera.orbit_yaw -= motion.delta.x * 0.005;
            camera.pitch = (camera.pitch + motion.delta.y * 0.005).clamp(-0.1, 1.4);
        }
        camera.distance = (camera.distance * (1.0 - scroll.delta.y * 0.1)).clamp(3.0, 40.0);
    }
}

fn follow(
    time: Res<Time>,
    active: Res<ActiveCar>,
    terrain: Res<Terrain>,
    cars: Query<&Car>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform)>,
) {
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return;
    };
    let target = car.body.position + Vec3::Y * 0.8;
    for (mut camera, mut transform) in &mut cameras {
        // Trail the car's heading with a little lag, taking the short way around.
        let delta = (heading(car.body.rotation) - camera.yaw + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        camera.yaw += delta * (1.0 - (-3.0 * time.delta_secs()).exp());

        let rotation = Quat::from_rotation_y(camera.yaw + camera.orbit_yaw)
            * Quat::from_rotation_x(-camera.pitch);
        let mut position = target + rotation * Vec3::new(0.0, 0.0, camera.distance);
        position.y = position
            .y
            .max(terrain.sample(position.x, position.z).0 + 0.5);
        *transform = Transform::from_translation(position).looking_at(target, Vec3::Y);
    }
}
