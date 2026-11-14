mod terrain;

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
            brightness: 600.0,
            ..default()
        })
        .add_plugins(terrain::TerrainPlugin)
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: 9_000.0,
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
        Transform::from_xyz(0.0, 30.0, 60.0).looking_at(Vec3::ZERO, Vec3::Y),
        DistanceFog {
            color: SKY,
            falloff: FogFalloff::Linear {
                start: 150.0,
                end: 400.0,
            },
            ..default()
        },
    ));
}
