//! The game's library, shared by its two apps: the sandbox (`main.rs`: every car and part
//! free, with the developer tools) and the career (`bin/career.rs`). Each app is the
//! plugins it adds; `CorePlugin` is what both have: the world, driving a car, and seeing
//! and hearing it.

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
pub mod garage;
pub mod hud;
pub mod input;
pub mod map;
pub mod obstacles;
pub mod physics;
pub mod scenery;
pub mod sim;
pub mod storage;
pub mod terrain;
pub mod traffic;
pub mod trees;
pub mod tunes;
pub mod ui;
pub mod visuals;
pub mod wheel_base;

use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;

const SKY: Color = Color::srgb(0.62, 0.76, 0.90);

/// The engine's own plugins, with the game's window titled `title`.
pub fn default_plugins(title: &str) -> impl PluginGroup {
    DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: title.into(),
            canvas: Some("#bevy".into()),
            fit_canvas_to_parent: true,
            ..default()
        }),
        ..default()
    })
}

/// What every app has: the map (terrain, scenery, houses and trees to hit, traffic), the
/// simulation, wheel and keyboard input, the cars and their looks, the camera, the HUD, the
/// map view, sound, and the egui setup the windows share.
pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(SKY))
            .insert_resource(GlobalAmbientLight {
                brightness: 250.0,
                ..default()
            })
            .add_plugins((
                (
                    terrain::TerrainPlugin,
                    scenery::SceneryPlugin,
                    obstacles::ObstaclesPlugin,
                    traffic::TrafficPlugin,
                ),
                sim::SimPlugin,
                input::InputPlugin,
                wheel_base::WheelBasePlugin,
                car::CarPlugin,
                visuals::VisualsPlugin,
                cockpit::CockpitPlugin,
                camera::CameraPlugin,
                debug::DebugPlugin,
                ui::UiPlugin,
                ecu_ui::EcuUiPlugin,
                hud::HudPlugin,
                map::MapPlugin,
                audio::AudioPlugin,
            ))
            .add_plugins((garage::GaragePlugin, dyno::DynoPlugin))
            .add_systems(Startup, setup);
    }
}

fn setup(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: 5_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(30.0, 60.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
        CascadeShadowConfigBuilder {
            maximum_distance: 120.0,
            ..default()
        }
        .build(),
    ));
    commands.spawn((
        Camera3d::default(),
        camera::ChaseCamera::default(),
        DistanceFog {
            color: SKY,
            falloff: FogFalloff::Linear {
                start: 180.0,
                end: 480.0,
            },
            ..default()
        },
    ));
}
