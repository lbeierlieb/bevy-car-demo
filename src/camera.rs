//! Camera on the active car: a chase view (drag with the left mouse button to orbit, scroll
//! to zoom) and a driver's view from the seat (drag to look around). C switches views,
//! facing forward again (the chase view keeps its zoom); B (or the wheel's look-back
//! button) held looks backwards in either.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;

use crate::car::{ActiveCar, Car, heading};
use crate::input::WheelInput;
use crate::terrain::Terrain;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (switch_view, orbit, follow)
                .chain()
                .after(crate::car::sync_transforms),
        );
    }
}

/// Vertical field of view in the chase and driver's views.
const CHASE_FOV: f32 = std::f32::consts::FRAC_PI_4;
const DRIVER_FOV: f32 = 0.95;
/// The driver looks slightly down by default, so the gauges are in view.
const DRIVER_PITCH: f32 = -0.08;
/// The chase view's default height angle.
const CHASE_PITCH: f32 = 0.3;

#[derive(Component)]
pub struct ChaseCamera {
    driver: bool,
    /// Smoothed car heading the camera trails behind.
    yaw: f32,
    orbit_yaw: f32,
    pitch: f32,
    distance: f32,
    /// Head turn in the driver's view.
    look_yaw: f32,
    look_pitch: f32,
    /// Looking backwards (held).
    back: bool,
}

impl Default for ChaseCamera {
    fn default() -> Self {
        Self {
            driver: false,
            yaw: 0.0,
            orbit_yaw: 0.0,
            pitch: CHASE_PITCH,
            distance: 9.0,
            look_yaw: 0.0,
            look_pitch: DRIVER_PITCH,
            back: false,
        }
    }
}

impl ChaseCamera {
    /// Whether the camera is in the driver's seat.
    pub fn driver(&self) -> bool {
        self.driver
    }

    /// The field of view and near plane for the view it's in.
    pub fn lens(&self) -> (f32, f32) {
        if self.driver {
            (DRIVER_FOV, 0.02)
        } else {
            (CHASE_FOV, 0.1)
        }
    }
}

fn switch_view(wheel: Res<WheelInput>, mut cameras: Query<(&mut ChaseCamera, &mut Projection)>) {
    let back = wheel.look_back;
    for (mut camera, _) in &mut cameras {
        camera.back = back;
    }
    if !wheel.view {
        return;
    }
    for (mut camera, mut projection) in &mut cameras {
        // Facing forward again, at the same zoom.
        camera.driver = !camera.driver;
        camera.orbit_yaw = 0.0;
        camera.pitch = CHASE_PITCH;
        camera.look_yaw = 0.0;
        camera.look_pitch = DRIVER_PITCH;
        if let Projection::Perspective(perspective) = &mut *projection {
            // The roof lining is only a few centimetres above the driver's eyes.
            (perspective.fov, perspective.near) = camera.lens();
        }
    }
}

fn orbit(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut cameras: Query<&mut ChaseCamera>,
) {
    for mut camera in &mut cameras {
        if camera.driver {
            if buttons.pressed(MouseButton::Left) {
                camera.look_yaw = (camera.look_yaw - motion.delta.x * 0.005).clamp(-2.2, 2.2);
                camera.look_pitch = (camera.look_pitch - motion.delta.y * 0.005).clamp(-0.8, 0.6);
            }
            continue;
        }
        if buttons.pressed(MouseButton::Left) {
            camera.orbit_yaw -= motion.delta.x * 0.005;
            camera.pitch = (camera.pitch + motion.delta.y * 0.005).clamp(-0.1, 1.4);
        }
        camera.distance = (camera.distance * (1.0 - scroll.delta.y * 0.1)).clamp(3.0, 40.0);
    }
}

pub fn follow(
    time: Res<Time>,
    active: Res<ActiveCar>,
    terrain: Res<Terrain>,
    cars: Query<&Car>,
    mut cameras: Query<(&mut ChaseCamera, &mut Transform)>,
) {
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return;
    };
    let target = car.body.position + Vec3::Y * 0.5;
    for (mut camera, mut transform) in &mut cameras {
        // Trail the car's heading with a little lag, taking the short way around.
        let delta = (heading(car.body.rotation) - camera.yaw + std::f32::consts::PI)
            .rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        camera.yaw += delta * (1.0 - (-3.0 * time.delta_secs()).exp());

        if camera.driver {
            // Fixed to the seat: the view moves with every bump and roll of the body.
            // Looking back, the head turns all the way round, level.
            let rotation = car.body.rotation;
            let (look_yaw, look_pitch) = if camera.back {
                (std::f32::consts::PI, 0.0)
            } else {
                (camera.look_yaw, camera.look_pitch)
            };
            *transform = Transform::from_translation(car.body.position + rotation * car.setup.eye)
                .with_rotation(
                    rotation * Quat::from_rotation_y(look_yaw) * Quat::from_rotation_x(look_pitch),
                );
            continue;
        }
        // Looking back, from ahead of the car.
        let yaw = if camera.back {
            camera.yaw + std::f32::consts::PI
        } else {
            camera.yaw + camera.orbit_yaw
        };
        let rotation = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-camera.pitch);
        let mut position = target + rotation * Vec3::new(0.0, 0.0, camera.distance);
        position.y = position
            .y
            .max(terrain.sample(position.x, position.z).0 + 0.5);
        *transform = Transform::from_translation(position).looking_at(target, Vec3::Y);
    }
}
