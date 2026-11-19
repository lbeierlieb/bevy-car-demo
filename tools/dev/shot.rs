// Headless screenshot helper, copied to src/shot.rs by unstrip.sh while testing; never commit src/shot.rs.
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

pub struct ShotPlugin;
impl Plugin for ShotPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("SHOT").is_ok() {
            app.add_systems(PreUpdate, shot.after(bevy::input::InputSystems));
            app.add_systems(PreUpdate, click::<false>.before(bevy::input::InputSystems));
            app.add_systems(
                PostUpdate,
                forest_cam.before(bevy::transform::TransformSystems::Propagate),
            );
            // Before the car gets its meshes, which would be added to a despawned car.
            app.add_systems(PreUpdate, (pick_car, line_up, place_car));
            app.add_systems(
                PostUpdate,
                car_cam
                    .after(forest_cam)
                    .before(bevy::transform::TransformSystems::Propagate),
            );
            app.add_systems(
                PostUpdate,
                traffic_cam
                    .after(car_cam)
                    .before(bevy::transform::TransformSystems::Propagate),
            );
        }
    }
}

fn shot(
    mut commands: Commands,
    mut frame: Local<u32>,
    mut exit: MessageWriter<AppExit>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
) {
    *frame += 1;
    let at: u32 = std::env::var("FRAMES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120);
    if let Ok(k) = std::env::var("KEYS") {
        let (start, end) = (at / 3, at);
        if *frame == start {
            for c in k.chars() {
                keys.press(key(c));
            }
        }
        if *frame == end {
            for c in k.chars() {
                keys.release(key(c));
            }
        }
    }
    if *frame == at {
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(std::env::var("SHOT").unwrap()));
    }
    if *frame == at + 20 {
        exit.write(AppExit::Success);
    }
}

/// CLICK="x,y,frame[;x,y,frame…]": move the mouse to (x, y) in the window and click there
/// at that frame (as window events, which is what bevy_egui reads). The editor's hook reads
/// UICLICK instead (CLICK is its 3D clicks).
fn click<const UI: bool>(
    mut frame: Local<u32>,
    windows: Query<Entity, With<Window>>,
    mut events: MessageWriter<bevy::window::WindowEvent>,
) {
    *frame += 1;
    let Ok(spec) = std::env::var(if UI { "UICLICK" } else { "CLICK" }) else {
        return;
    };
    let window = windows.single().unwrap();
    for one in spec.split(';') {
        let v: Vec<f32> = one.split(',').map(|s| s.parse().unwrap()).collect();
        let at = v[2] as u32;
        if (at..at + 6).contains(&*frame) {
            events.write(
                CursorMoved {
                    window,
                    position: Vec2::new(v[0], v[1]),
                    delta: None,
                }
                .into(),
            );
        }
        let state = if *frame == at + 2 {
            bevy::input::ButtonState::Pressed
        } else if *frame == at + 3 {
            bevy::input::ButtonState::Released
        } else {
            continue;
        };
        events.write(
            bevy::input::mouse::MouseButtonInput {
                button: MouseButton::Left,
                state,
                window,
            }
            .into(),
        );
    }
}

fn key(c: char) -> KeyCode {
    match c {
        'w' => KeyCode::KeyW,
        'a' => KeyCode::KeyA,
        'd' => KeyCode::KeyD,
        's' => KeyCode::KeyS,
        't' => KeyCode::Tab,
        'f' => KeyCode::Digit1,
        'c' => KeyCode::KeyC,
        'k' => KeyCode::KeyK,
        '2' => KeyCode::Digit2,
        '3' => KeyCode::Digit3,
        '4' => KeyCode::Digit4,
        '5' => KeyCode::Digit5,
        '6' => KeyCode::Digit6,
        'T' => KeyCode::KeyT,
        'E' => KeyCode::Enter,
        _ => KeyCode::Space,
    }
}

/// CAR="chassis[,engine]": start in that car of the catalogue (its place in `CHASSIS`, and
/// the factory's engine option for it).
fn pick_car(
    mut commands: Commands,
    terrain: Res<crate::terrain::Terrain>,
    cars: Query<(Entity, &crate::car::Car)>,
    mut done: Local<bool>,
) {
    let Ok(spec) = std::env::var("CAR") else {
        return;
    };
    let Ok((entity, car)) = cars.single() else {
        return;
    };
    if std::mem::replace(&mut *done, true) {
        return;
    }
    let v: Vec<usize> = spec.split(',').map(|s| s.parse().unwrap()).collect();
    let setup = crate::car::CarSetup::factory(v[0], *v.get(1).unwrap_or(&0));
    let p = car.body.position;
    let new = crate::car::Car::spawn_at(
        0,
        setup,
        &terrain,
        p.x,
        p.z,
        crate::car::heading(car.body.rotation),
    );
    commands.entity(entity).despawn();
    commands.spawn((Transform::from_translation(new.body.position), new));
}

/// LINEUP="first,count": instead of the one car, `count` cars of the catalogue from the
/// `first`, parked one behind the other (6 m apart) on the flattest, most open straight
/// of paved road there is, so they can be looked at from the side.
fn line_up(
    mut commands: Commands,
    terrain: Res<crate::terrain::Terrain>,
    cars: Query<Entity, With<crate::car::Car>>,
    mut done: Local<bool>,
) {
    let Ok(spec) = std::env::var("LINEUP") else {
        return;
    };
    if std::mem::replace(&mut *done, true) {
        return;
    }
    let v: Vec<usize> = spec.split(',').map(|s| s.parse().unwrap()).collect();
    for entity in &cars {
        commands.entity(entity).despawn();
    }
    let length = 6.0 * v[1] as f32 + 6.0;
    let height = |p: Vec2| terrain.sample(p.x, p.y).0;
    let mut best = (f32::MAX, Vec2::ZERO, Vec2::X);
    for road in terrain
        .roads
        .iter()
        .filter(|r| r.kind.paved() && r.width >= 5.0)
    {
        for pair in road.points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if a.distance(b) < length || !terrain.inside(a.x, a.y) || !terrain.inside(b.x, b.y) {
                continue;
            }
            let along = (b - a).normalize();
            let side = along.perp();
            // Level along and across, and no buildings near.
            let mut score = (height(b) - height(a)).abs() / a.distance(b) * 40.0;
            for k in 0..4 {
                let p = a + along * (length * k as f32 / 3.0);
                for d in [-30.0, -20.0, -10.0, 10.0, 20.0, 30.0] {
                    score = score.max((height(p + side * d) - height(p)).abs());
                }
            }
            let middle = a + along * length / 2.0;
            let near = terrain
                .buildings
                .iter()
                .any(|building| building.outline.iter().any(|q| q.distance(middle) < 80.0));
            if near || terrain.land_class(middle.x + side.x * 15.0, middle.y + side.y * 15.0) == 8 {
                continue;
            }
            if score < best.0 {
                best = (score, a, along);
            }
        }
    }
    let (_, start, along) = best;
    for i in 0..v[1] {
        // The first car in front; cars face along -z of their heading.
        let p = start + along * (length - 6.0 - 6.0 * i as f32);
        let yaw = (-along.x).atan2(-along.y);
        let setup = crate::car::CarSetup::factory(v[0] + i, 0);
        let car = crate::car::Car::spawn_at(i, setup, &terrain, p.x, p.y, yaw);
        commands.spawn((Transform::from_translation(car.body.position), car));
    }
}

/// CARCAM="side,height,along": look at the middle of the cars from `side` m to their right
/// (negative: left), `height` m up and `along` m ahead of them.
fn car_cam(
    mut cams: Query<&mut Transform, With<Camera3d>>,
    cars: Query<&Transform, (With<crate::car::Car>, Without<Camera3d>)>,
) {
    let Ok(spec) = std::env::var("CARCAM") else {
        return;
    };
    let v: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
    let Some(first) = cars.iter().next() else {
        return;
    };
    let middle = cars.iter().map(|t| t.translation).sum::<Vec3>() / cars.iter().count() as f32;
    let eye = middle + first.rotation * Vec3::new(v[0], 0.0, -v[2]) + Vec3::Y * v[1];
    for mut t in &mut cams {
        *t = Transform::from_translation(eye).looking_at(middle + Vec3::Y * 0.3, Vec3::Y);
    }
}

/// CAM="distance,height": look at the forest edge nearest the spawn from that offset.
pub fn forest_cam(
    terrain: Res<crate::terrain::Terrain>,
    mut cams: Query<&mut Transform, With<Camera3d>>,
    cars: Query<&Transform, (With<crate::car::Car>, Without<Camera3d>)>,
) {
    if let Ok(spec) = std::env::var("CAMAT") {
        // CAMAT="x,z,back,height": look at (x, z) from `back` m south and `height` m up.
        let v: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
        let ty = terrain.sample(v[0], v[1]).0;
        let eye = Vec3::new(v[0], ty + v[3], v[1] + v[2]);
        for mut t in &mut cams {
            *t = Transform::from_translation(eye).looking_at(Vec3::new(v[0], ty, v[1]), Vec3::Y);
        }
        return;
    }
    let Ok(spec) = std::env::var("CAM") else {
        return;
    };
    let v: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
    let car = cars.iter().next().unwrap().translation;
    let mut best = (f32::MAX, Vec2::ZERO);
    for gz in -300..300 {
        for gx in -300..300 {
            let p = car.xz() + Vec2::new(gx as f32, gz as f32) * 4.0;
            if terrain.inside(p.x, p.y) && terrain.land_class(p.x, p.y) == 8 {
                let d = p.distance(car.xz());
                if d < best.0 {
                    best = (d, p);
                }
            }
        }
    }
    let target = best.1;
    let dir = (target - car.xz()).normalize();
    let eye = target - dir * v[0];
    let ty = terrain.sample(target.x, target.y).0;
    let ey = terrain.sample(eye.x, eye.y).0 + v[1];
    for mut t in &mut cams {
        *t = Transform::from_xyz(eye.x, ey, eye.y)
            .looking_at(Vec3::new(target.x, ty + 5.0, target.y), Vec3::Y);
    }
}

/// WHEELLOG=1: print the wheel input and drivetrain state twice a second, and pretend the
/// window has focus (Xvfb never gives it, and force feedback fades out without it).
pub struct WheelLogPlugin;
impl Plugin for WheelLogPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("WHEELLOG").is_ok() {
            app.add_systems(Update, wheel_log).add_systems(
                PreUpdate,
                |mut windows: Query<&mut Window>| {
                    for mut w in &mut windows {
                        w.focused = true;
                    }
                },
            );
        }
    }
}
fn wheel_log(
    time: Res<Time<Real>>,
    wheel: Res<crate::input::WheelInput>,
    cars: Query<&crate::car::Car>,
    ffb: Res<crate::wheel_base::ForceFeedback>,
    mut last: Local<f32>,
) {
    let t = time.elapsed_secs();
    if t - *last < 0.5 {
        return;
    }
    *last = t;
    let car = cars.iter().find(|c| c.index == 0).unwrap();
    eprintln!(
        "WHEEL t {t:6.1} steer {:?} thr {:?} clutch {:?} shifter {:?} | gear {} rpm {:5.0} running {} speed {:5.2} | torque {:7.1} ffb {:.3} {:?}",
        wheel.steer,
        wheel.throttle,
        wheel.clutch,
        wheel.shifter,
        car.drivetrain.gear,
        car.drivetrain.rpm,
        car.drivetrain.running,
        car.forward_speed(),
        car.steering_torque,
        ffb.level,
        ffb.problem
    );
}

/// TRAFFICCAM="back,height[,nth]": follow the nth (default 0) nearest traffic car to the
/// player from `back` m behind it and `height` m up.
fn traffic_cam(
    mut cams: Query<&mut Transform, With<Camera3d>>,
    cars: Query<&crate::car::Car>,
    traffic: Query<&crate::traffic::TrafficCar>,
) {
    let Ok(spec) = std::env::var("TRAFFICCAM") else {
        return;
    };
    let v: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
    let Some(player) = cars.iter().next() else {
        return;
    };
    let mut near: Vec<_> = traffic.iter().map(|t| &t.body).collect();
    near.sort_by(|a, b| {
        a.position
            .distance(player.body.position)
            .total_cmp(&b.position.distance(player.body.position))
    });
    let Some(body) = near.get(v.get(2).copied().unwrap_or(0.0) as usize) else {
        return;
    };
    let forward = body.rotation * Vec3::NEG_Z;
    let eye = body.position - forward * v[0] + Vec3::Y * v[1];
    for mut t in &mut cams {
        *t = Transform::from_translation(eye).looking_at(body.position + forward * 6.0, Vec3::Y);
    }
}

/// PLACE="x,z,frame": put the car being driven on the road nearest map point (x, z) at
/// that frame, e.g. on a career place's marker.
fn place_car(
    terrain: Res<crate::terrain::Terrain>,
    active: Res<crate::car::ActiveCar>,
    mut cars: Query<&mut crate::car::Car>,
    mut frame: Local<u32>,
) {
    *frame += 1;
    let Ok(spec) = std::env::var("PLACE") else {
        return;
    };
    let v: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
    if *frame != v[2] as u32 {
        return;
    }
    let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0) else {
        return;
    };
    let target = Vec2::new(v[0], v[1]);
    let heading = (car.body.rotation * Vec3::NEG_Z).xz();
    if let Some((spot, yaw)) = terrain.drive_spot(target, heading) {
        car.place(&terrain, spot, yaw);
    }
}

/// The map editor's hook: screenshots and clicks, and `EDITCAM="x,z,height,yaw,pitch"`
/// (degrees; yaw 0 looks north, pitch negative looks down) placing the fly camera.
pub struct EditorShotPlugin;
impl Plugin for EditorShotPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("SHOT").is_ok() {
            // No window-event clicks (as `click` sends for egui): egui would take the
            // pointer and bevy_egui release the button.
            app.add_systems(PreUpdate, shot.after(bevy::input::InputSystems));
            app.add_systems(PreUpdate, edit_cam);
            app.add_systems(PreUpdate, click::<true>.before(bevy::input::InputSystems));
            app.add_systems(PreUpdate, edit_keys.after(bevy::input::InputSystems));
            app.add_systems(PreUpdate, edit_click.after(bevy::input::InputSystems));
        }
    }
}

fn edit_cam(
    mut frame: Local<u32>,
    terrain: Res<crate::terrain::Terrain>,
    mut cams: Query<(&mut crate::editor::camera::FlyCamera, &mut Transform)>,
) {
    // Only at the start (the scenery loads around it), so the camera can move later.
    *frame += 1;
    let Ok(spec) = std::env::var("EDITCAM") else {
        return;
    };
    if *frame > 20 {
        return;
    }
    let v: Vec<f32> = spec.split(',').map(|s| s.parse().unwrap()).collect();
    for (mut cam, mut transform) in &mut cams {
        cam.yaw = v[3].to_radians();
        cam.pitch = v[4].to_radians();
        transform.translation = Vec3::new(v[0], terrain.sample(v[0], v[1]).0 + v[2], v[1]);
    }
}

/// The editor reads the window's cursor and the mouse buttons (not window events), so
/// CLICK sets those too; `CLICK="x,y,frame,x2,y2"` drags from (x, y) to (x2, y2).
fn edit_click(
    mut frame: Local<u32>,
    mut windows: Query<&mut Window>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
) {
    *frame += 1;
    let Ok(spec) = std::env::var("CLICK") else {
        return;
    };
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    for one in spec.split(';') {
        let v: Vec<f32> = one.split(',').map(|s| s.parse().unwrap()).collect();
        let at = v[2] as u32;
        let drag = v.len() >= 5;
        let (from, to) = (
            Vec2::new(v[0], v[1]),
            if drag {
                Vec2::new(v[3], v[4])
            } else {
                Vec2::new(v[0], v[1])
            },
        );
        let end = if drag { at + 12 } else { at + 3 };
        if (at..=end + 3).contains(&*frame) {
            let t = ((*frame as f32 - at as f32 - 2.0) / (end - at - 2) as f32).clamp(0.0, 1.0);
            window.set_cursor_position(Some(from.lerp(to, t)));
        }
        // Held every frame (something releases an injected press otherwise).
        if (at + 2..end).contains(&*frame) {
            buttons.press(MouseButton::Left);
        }
        if *frame == end {
            buttons.release(MouseButton::Left);
        }
    }
}

/// `EDITKEYS="Digit2@25;Enter@60"`: taps those keys (by `KeyCode` name) at those frames;
/// `ControlLeft+KeyZ@50` holds the first while tapping the second.
fn edit_keys(mut frame: Local<u32>, mut keys: ResMut<ButtonInput<KeyCode>>) {
    *frame += 1;
    let Ok(spec) = std::env::var("EDITKEYS") else {
        return;
    };
    let code = |name: &str| match name {
        "Digit1" => KeyCode::Digit1,
        "Digit2" => KeyCode::Digit2,
        "Digit3" => KeyCode::Digit3,
        "Digit4" => KeyCode::Digit4,
        "Digit5" => KeyCode::Digit5,
        "Digit6" => KeyCode::Digit6,
        "Digit7" => KeyCode::Digit7,
        "Enter" => KeyCode::Enter,
        "Escape" => KeyCode::Escape,
        "Delete" => KeyCode::Delete,
        "Backspace" => KeyCode::Backspace,
        "KeyX" => KeyCode::KeyX,
        "KeyZ" => KeyCode::KeyZ,
        "KeyY" => KeyCode::KeyY,
        "KeyS" => KeyCode::KeyS,
        "KeyM" => KeyCode::KeyM,
        "PageUp" => KeyCode::PageUp,
        "PageDown" => KeyCode::PageDown,
        "ControlLeft" => KeyCode::ControlLeft,
        "ShiftLeft" => KeyCode::ShiftLeft,
        "F5" => KeyCode::F5,
        other => panic!("EDITKEYS: unknown key {other}"),
    };
    for one in spec.split(';') {
        let (names, at) = one.split_once('@').unwrap();
        let at: u32 = at.parse().unwrap();
        let names: Vec<KeyCode> = names.split('+').map(code).collect();
        if *frame == at {
            names.iter().for_each(|k| keys.press(*k));
        }
        if *frame == at + 1 {
            names.iter().for_each(|k| keys.release(*k));
        }
    }
}
