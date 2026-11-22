//! The fly camera: hold the right mouse button to look, WASD to move, Q/E down and up,
//! Shift faster, the mouse wheel sets the speed. A click on the map (M) flies there.

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::input::EguiWantsInput;

use crate::map::{MapClicked, MapView};
use crate::terrain::{Spawn, Terrain};

/// Speeds the wheel steps through (m/s), and the one to start at.
const SPEEDS: [f32; 9] = [2.0, 5.0, 10.0, 20.0, 40.0, 80.0, 150.0, 300.0, 600.0];
const START_SPEED: usize = 3;
/// Shift multiplies the speed by this.
const FAST: f32 = 4.0;
/// Radians per pixel of mouse motion.
const LOOK: f32 = 0.003;
/// The camera stays this far above the ground (m).
const CLEARANCE: f32 = 0.5;

pub struct FlyCameraPlugin;

impl Plugin for FlyCameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn)
            .add_systems(Update, (teleport, fly).chain());
    }
}

#[derive(Component)]
pub struct FlyCamera {
    pub yaw: f32,
    pub pitch: f32,
    /// Index into `SPEEDS`.
    speed: usize,
    /// Looking around (the right button went down over the 3D view).
    looking: bool,
}

impl FlyCamera {
    pub fn speed(&self) -> f32 {
        SPEEDS[self.speed]
    }

    fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }
}

fn spawn(mut commands: Commands, terrain: Res<Terrain>, start: Res<Spawn>) {
    let (spot, _) = terrain.road_spawns(&start, 1, 0.0)[0];
    let camera = FlyCamera {
        yaw: 0.0,
        pitch: -0.5,
        speed: START_SPEED,
        looking: false,
    };
    let target = spot.extend(terrain.sample(spot.x, spot.y).0).xzy();
    let position = target - camera.rotation() * Vec3::NEG_Z * 60.0;
    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(position).with_rotation(camera.rotation()),
        camera,
        crate::fog(),
    ));
}

/// Moves the camera so it looks at `target` from `distance` away, keeping its heading.
pub fn look_at(camera: &mut FlyCamera, transform: &mut Transform, target: Vec3, distance: f32) {
    camera.pitch = camera.pitch.min(-0.35);
    transform.rotation = camera.rotation();
    transform.translation = target - transform.rotation * Vec3::NEG_Z * distance;
}

fn teleport(
    mut clicks: MessageReader<MapClicked>,
    terrain: Res<Terrain>,
    mut view: ResMut<MapView>,
    mut cameras: Query<(&mut FlyCamera, &mut Transform)>,
) {
    for &MapClicked(at) in clicks.read() {
        let at = at.clamp(-terrain.half_size(), terrain.half_size());
        let target = at.extend(terrain.sample(at.x, at.y).0).xzy();
        for (mut camera, mut transform) in &mut cameras {
            look_at(&mut camera, &mut transform, target, 60.0);
        }
        view.open = false;
    }
}

#[allow(clippy::too_many_arguments)]
fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    egui: Res<EguiWantsInput>,
    view: Res<MapView>,
    terrain: Res<Terrain>,
    mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>,
    mut cameras: Query<(&mut FlyCamera, &mut Transform)>,
) {
    let Ok((mut camera, mut transform)) = cameras.single_mut() else {
        return;
    };
    let over_ui = egui.wants_pointer_input() || egui.is_pointer_over_area() || view.open;
    if buttons.just_pressed(MouseButton::Right) && !over_ui {
        camera.looking = true;
    }
    if !buttons.pressed(MouseButton::Right) {
        camera.looking = false;
    }
    if let Ok(mut cursor) = windows.single_mut() {
        let grab = if camera.looking {
            CursorGrabMode::Locked
        } else {
            CursorGrabMode::None
        };
        if cursor.grab_mode != grab {
            cursor.grab_mode = grab;
            cursor.visible = !camera.looking;
        }
    }
    if camera.looking {
        camera.yaw -= motion.delta.x * LOOK;
        camera.pitch = (camera.pitch - motion.delta.y * LOOK).clamp(-1.55, 1.55);
    }
    if !over_ui && scroll.delta.y != 0.0 {
        let step = scroll.delta.y.signum() as isize;
        camera.speed = (camera.speed as isize + step).clamp(0, SPEEDS.len() as isize - 1) as usize;
    }
    transform.rotation = camera.rotation();
    if view.open {
        return;
    }

    let flat = |v: Vec3| v.with_y(0.0).normalize_or_zero();
    let (forward, right) = (
        flat(transform.forward().into()),
        flat(transform.right().into()),
    );
    let mut wish = Vec3::ZERO;
    for (key, dir) in [
        (KeyCode::KeyW, forward),
        (KeyCode::KeyS, -forward),
        (KeyCode::KeyD, right),
        (KeyCode::KeyA, -right),
        (KeyCode::KeyE, Vec3::Y),
        (KeyCode::KeyQ, Vec3::NEG_Y),
    ] {
        if keys.pressed(key) {
            wish += dir;
        }
    }
    // Ctrl is for shortcuts (Ctrl+S, Ctrl+Z…), not moving.
    if keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) {
        wish = Vec3::ZERO;
    }
    let fast = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        FAST
    } else {
        1.0
    };
    let mut position = transform.translation
        + wish.normalize_or_zero() * camera.speed() * fast * time.delta_secs();
    let half = terrain.half_size();
    position.x = position.x.clamp(-half.x, half.x);
    position.z = position.z.clamp(-half.y, half.y);
    position.y = position
        .y
        .clamp(terrain.sample(position.x, position.z).0 + CLEARANCE, 3000.0);
    transform.translation = position;
}
