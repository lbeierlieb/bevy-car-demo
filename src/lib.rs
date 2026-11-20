//! The game's library, shared by its apps: the sandbox (`main.rs`: every car and part
//! free, with the developer tools) and the map editor (`bin/editor.rs`, native only, no
//! cars). Each app is the plugins it adds; `CorePlugin` is what a driving app has: the
//! world, driving a car, and seeing and hearing it.

pub mod audio;
pub mod body;
pub mod buildings;
pub mod camera;
pub mod car;
pub mod chassis;
pub mod cockpit;
pub mod debug;
pub mod dyno;
pub mod ecu_ui;
#[cfg(not(target_arch = "wasm32"))]
pub mod editor;
pub mod fences;
pub mod garage;
pub mod help;
pub mod hud;
pub mod input;
pub mod looks;
pub mod map;
pub mod net;
pub mod obstacles;
pub mod online;
pub mod osm;
pub mod physics;
pub mod relief;
pub mod scenery;
pub mod sim;
pub mod sky;
pub mod storage;
pub mod terrain;
pub mod traffic;
pub mod trees;
pub mod tunes;
pub mod ui;
pub mod visuals;
pub mod wheel_base;
pub mod wheels;
pub mod workshop;

use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;

/// The engine's own plugins, with the game's window titled `title`, and F11 for
/// fullscreen.
pub fn default_plugins(title: &str) -> impl PluginGroup {
    DefaultPlugins
        .set(WindowPlugin {
            primary_window: Some(Window {
                title: title.into(),
                canvas: Some("#bevy".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        })
        .add(FullscreenPlugin)
}

/// F11 switches the window between windowed and borderless fullscreen. On the web the
/// page keeps the F keys from the game and the browser does it.
struct FullscreenPlugin;

impl Plugin for FullscreenPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, toggle_fullscreen);
    }
}

fn toggle_fullscreen(keys: Res<ButtonInput<KeyCode>>, mut windows: Query<&mut Window>) {
    if !keys.just_pressed(KeyCode::F11) {
        return;
    }
    for mut window in &mut windows {
        window.mode = match window.mode {
            bevy::window::WindowMode::Windowed => bevy::window::WindowMode::BorderlessFullscreen(
                bevy::window::MonitorSelection::Current,
            ),
            _ => bevy::window::WindowMode::Windowed,
        };
    }
}

/// The world without anything to see or hear, which the server has too: the map with its
/// houses and trees to hit, the simulation, and the traffic.
pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            terrain::TerrainPlugin,
            obstacles::ObstaclesPlugin,
            sim::SimPlugin,
            traffic::TrafficPlugin,
        ));
    }
}

/// What every driving app has: the world, its scenery and traffic to see, wheel and
/// keyboard input, driving a car and its looks, the camera, the HUD, the map view, sound,
/// and the egui setup the windows share. Which car there is, is up to the app.
pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(GlobalAmbientLight {
            brightness: 250.0,
            ..default()
        })
        .add_plugins((
            (
                WorldPlugin,
                scenery::SceneryPlugin,
                traffic::TrafficLooksPlugin,
            ),
            input::InputPlugin,
            wheel_base::WheelBasePlugin,
            car::CarPlugin,
            looks::LooksPlugin,
            net::ClientPlugin,
            visuals::VisualsPlugin,
            cockpit::CockpitPlugin,
            camera::CameraPlugin,
            ui::UiPlugin,
            hud::HudPlugin,
            map::MapPlugin,
            audio::AudioPlugin,
            sky::SkyPlugin,
        ))
        .add_systems(Startup, setup);
    }
}

fn setup(mut commands: Commands) {
    spawn_sun(&mut commands);
    commands.spawn((Camera3d::default(), camera::ChaseCamera::default(), fog()));
}

/// The sun, with shadows near the camera.
pub fn spawn_sun(commands: &mut Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: 5_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_translation(sky::SUN).looking_at(Vec3::ZERO, Vec3::Y),
        CascadeShadowConfigBuilder {
            maximum_distance: 120.0,
            ..default()
        }
        .build(),
    ));
}

/// The haze the land fades into, ending before the scenery stops being loaded.
pub fn fog() -> DistanceFog {
    DistanceFog {
        color: sky::HAZE,
        falloff: FogFalloff::Linear {
            start: 180.0,
            end: 480.0,
        },
        ..default()
    }
}
