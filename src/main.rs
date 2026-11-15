mod audio;
mod buildings;
mod camera;
mod car;
mod debug;
mod physics;
mod scenery;
mod sim;
mod terrain;
mod trees;
mod ui;
mod visuals;

use bevy::light::CascadeShadowConfigBuilder;
use bevy::prelude::*;

const SKY: Color = Color::srgb(0.62, 0.76, 0.90);

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Car Sim".into(),
                canvas: Some("#bevy".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(SKY))
        .insert_resource(GlobalAmbientLight {
            brightness: 250.0,
            ..default()
        })
        .add_plugins((
            terrain::TerrainPlugin,
            scenery::SceneryPlugin,
            sim::SimPlugin,
            car::CarPlugin,
            visuals::VisualsPlugin,
            camera::CameraPlugin,
            debug::DebugPlugin,
            ui::UiPlugin,
            audio::AudioPlugin,
        ))
        .add_systems(Startup, setup)
        .run();
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
