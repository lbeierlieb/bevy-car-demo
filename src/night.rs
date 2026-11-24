//! Lights at night, as dark as the time of day makes it (`grade::Darkness`): every car's
//! head and tail lamps glow, some of the houses' windows are lit, and our own car's head
//! lamps light the road. Bloom (on the camera with post-processing, `grade`) makes them
//! shine. Purely local and cosmetic: nothing is sent, every car has its lamps on at night.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::grade::Darkness;
use crate::looks::Looks;
use crate::scenery::WindowGlow;

/// How the lamps and windows glow at full dark (emissive, linear).
const HEAD_GLOW: LinearRgba = LinearRgba::rgb(16.0, 15.0, 12.0);
const TAIL_GLOW: LinearRgba = LinearRgba::rgb(3.0, 0.06, 0.03);
const WINDOW_GLOW: LinearRgba = LinearRgba::rgb(2.2, 1.1, 0.35);
/// Our head lamps' light at full dark (lumens each), its reach (m), the beam's half
/// angles (rad) and how far it's aimed down.
const HEADLIGHT: f32 = 4_000_000.0;
const HEADLIGHT_RANGE: f32 = 90.0;
const BEAM: (f32, f32) = (0.22, 0.45);
const AIM_DOWN: f32 = 0.05;

pub struct NightPlugin;

impl Plugin for NightPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup)
            .add_systems(PostUpdate, (glow, headlights));
    }
}

/// The lamps' lit materials, shared by every car (`visuals` puts them on).
#[derive(Resource)]
pub struct Glow {
    pub head: Handle<StandardMaterial>,
    pub tail: Handle<StandardMaterial>,
}

/// One of our car's head lamps' lights.
#[derive(Component)]
struct Headlight;

fn setup(mut commands: Commands, mut materials: ResMut<Assets<StandardMaterial>>) {
    let mut lens = |color: Color| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: 0.2,
            ..default()
        })
    };
    commands.insert_resource(Glow {
        head: lens(Color::srgb(0.95, 0.93, 0.8)),
        tail: lens(Color::srgb(0.75, 0.04, 0.03)),
    });
}

/// The lamps' and windows' glow, when the darkness changes.
fn glow(
    darkness: Res<Darkness>,
    glow: Option<Res<Glow>>,
    windows: Option<Res<WindowGlow>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut last: Local<Option<f32>>,
) {
    let dark = darkness.0;
    if last.is_some_and(|last| (last - dark).abs() < 0.005) {
        return;
    }
    *last = Some(dark);
    let mut set = |handle: &Handle<StandardMaterial>, full: LinearRgba| {
        if let Some(mut material) = materials.get_mut(handle) {
            material.emissive = full * dark;
        }
    };
    if let Some(glow) = glow {
        set(&glow.head, HEAD_GLOW);
        set(&glow.tail, TAIL_GLOW);
    }
    if let Some(windows) = windows {
        set(&windows.0, WINDOW_GLOW);
    }
}

/// Spot lights at our car's head lamps (made afresh when its looks change), as bright as
/// it's dark, and hidden by day.
#[allow(clippy::type_complexity)]
fn headlights(
    mut commands: Commands,
    darkness: Res<Darkness>,
    cars: Query<(Entity, Ref<Looks>, Option<&Children>), With<crate::car::Car>>,
    mut lights: Query<(&mut SpotLight, &mut Visibility), With<Headlight>>,
) {
    for (car, looks, children) in &cars {
        if !looks.is_changed() {
            continue;
        }
        for &child in children.into_iter().flatten() {
            if lights.contains(child) {
                commands.entity(child).despawn();
            }
        }
        let chassis = looks.build.chassis();
        let (_, _, centres) = chassis.body.lamps(&chassis.wheels());
        commands.entity(car).with_children(|parent| {
            for centre in centres.into_iter().take(2) {
                parent.spawn((
                    Headlight,
                    SpotLight {
                        intensity: 0.0,
                        range: HEADLIGHT_RANGE,
                        inner_angle: BEAM.0,
                        outer_angle: BEAM.1,
                        shadow_maps_enabled: false,
                        color: Color::srgb(1.0, 0.95, 0.85),
                        ..default()
                    },
                    Transform::from_translation(looks.body_offset + centre)
                        .with_rotation(Quat::from_rotation_x(-AIM_DOWN)),
                    Visibility::Hidden,
                    NotShadowCaster,
                ));
            }
        });
    }
    let dark = darkness.0;
    let shown = if dark > 0.02 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for (mut light, mut visibility) in &mut lights {
        light.intensity = HEADLIGHT * dark;
        visibility.set_if_neq(shown);
    }
}
