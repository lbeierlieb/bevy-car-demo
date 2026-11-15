// Headless screenshot helper, copied to src/shot.rs by unstrip.sh while testing; never commit src/shot.rs.
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};

pub struct ShotPlugin;
impl Plugin for ShotPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("SHOT").is_ok() {
            app.add_systems(Update, shot);
            app.add_systems(PostUpdate, forest_cam.before(bevy::transform::TransformSystems::Propagate));
        }
    }
}

fn shot(mut commands: Commands, mut frame: Local<u32>, mut exit: MessageWriter<AppExit>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    *frame += 1;
    let at: u32 = std::env::var("FRAMES").ok().and_then(|s| s.parse().ok()).unwrap_or(120);
    if let Ok(k) = std::env::var("KEYS") {
        let (start, end) = (at / 3, at);
        if *frame == start { for c in k.chars() { keys.press(key(c)); } }
        if *frame == end { for c in k.chars() { keys.release(key(c)); } }
    }
    if *frame == at {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(std::env::var("SHOT").unwrap()));
    }
    if *frame == at + 20 { exit.write(AppExit::Success); }
}

fn key(c: char) -> KeyCode {
    match c { 'w' => KeyCode::KeyW, 'a' => KeyCode::KeyA, 'd' => KeyCode::KeyD, 's' => KeyCode::KeyS, 't' => KeyCode::Tab, 'f' => KeyCode::F1, 'c' => KeyCode::KeyC, _ => KeyCode::Space }
}

/// CAM="distance,height": look at the forest edge nearest the spawn from that offset.
pub fn forest_cam(terrain: Res<crate::terrain::Terrain>, mut cams: Query<&mut Transform, With<Camera3d>>, cars: Query<&Transform, (With<crate::car::Car>, Without<Camera3d>)>) {
    if let Ok(spec) = std::env::var("CAMAT") {
        // CAMAT="x,z,back,height": look at (x, z) from `back` m south and `height` m up.
        let v: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
        let ty = terrain.sample(v[0], v[1]).0;
        let eye = Vec3::new(v[0], ty + v[3], v[1] + v[2]);
        for mut t in &mut cams { *t = Transform::from_translation(eye).looking_at(Vec3::new(v[0], ty, v[1]), Vec3::Y); }
        return;
    }
    let Ok(spec) = std::env::var("CAM") else { return };
    let v: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
    let car = cars.iter().next().unwrap().translation;
    let mut best = (f32::MAX, Vec2::ZERO);
    for gz in -300..300 { for gx in -300..300 {
        let p = car.xz() + Vec2::new(gx as f32, gz as f32) * 4.0;
        if terrain.inside(p.x, p.y) && terrain.land_class(p.x, p.y) == 8 {
            let d = p.distance(car.xz()); if d < best.0 { best = (d, p); }
        }
    }}
    let target = best.1;
    let dir = (target - car.xz()).normalize();
    let eye = target - dir * v[0];
    let ty = terrain.sample(target.x, target.y).0;
    let ey = terrain.sample(eye.x, eye.y).0 + v[1];
    for mut t in &mut cams {
        *t = Transform::from_xyz(eye.x, ey, eye.y).looking_at(Vec3::new(target.x, ty + 5.0, target.y), Vec3::Y);
    }
}

/// WHEELLOG=1: print the wheel input and drivetrain state twice a second, and pretend the
/// window has focus (Xvfb never gives it, and force feedback fades out without it).
pub struct WheelLogPlugin;
impl Plugin for WheelLogPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("WHEELLOG").is_ok() {
            app.add_systems(Update, wheel_log)
                .add_systems(PreUpdate, |mut windows: Query<&mut Window>| for mut w in &mut windows { w.focused = true; });
        }
    }
}
fn wheel_log(time: Res<Time<Real>>, wheel: Res<crate::input::WheelInput>, cars: Query<&crate::car::Car>, ffb: Res<crate::wheel_base::ForceFeedback>, mut last: Local<f32>) {
    let t = time.elapsed_secs();
    if t - *last < 0.5 { return; }
    *last = t;
    let car = cars.iter().find(|c| c.index == 0).unwrap();
    eprintln!("WHEEL t {t:6.1} steer {:?} thr {:?} clutch {:?} shifter {:?} | gear {} rpm {:5.0} running {} speed {:5.2} | torque {:7.1} ffb {:.3} {:?}",
        wheel.steer, wheel.throttle, wheel.clutch, wheel.shifter, car.drivetrain.gear, car.drivetrain.rpm, car.drivetrain.running,
        car.forward_speed(), car.steering_torque, ffb.level, ffb.problem);
}
