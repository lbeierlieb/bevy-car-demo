//! Post-processing and the time of day: a colour grade over the 3D view (one full-screen
//! pass, `shaders/grade.wesl`) and the sun's and the sky light's colour and strength,
//! for day, dusk, night, or a cycle through them with dawn, blended all the way.
//!
//! The day is graded like a modern spy thriller: cool, teal shadows against warm
//! highlights, crushed blacks, a firm S-curve, a little less saturation, a teal sky.
//! Night is "day for night" as in film (darker, bluer, washed out, the sky held down),
//! with the sun turned into weak, cold moonlight so shadows fade. Off in the graphics
//! settings, there's no pass and the lights are the plain day's (the web starts with it
//! off); with "Day and night" off it's always day, graded. In a shop's scene
//! (`car::Away`) it's always day, so the rooms stay lit.
//!
//! With post-processing the camera renders in HDR with bloom (stronger at night, for
//! `night`'s lamps and windows), and the grade runs after tonemapping, on the picture
//! as shown.

use bevy::camera::Hdr;
use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::ecs::schedule::ScheduleConfigs;
use bevy::ecs::system::BoxedSystem;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::RenderApp;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::ShaderType;
use bevy::shader::ShaderRef;

use crate::graphics::{Graphics, TimeOfDay};

pub struct GradePlugin;

impl Plugin for GradePlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shaders/grade.wesl");
        app.add_plugins(FullscreenMaterialPlugin::<Grade>::default())
            .init_resource::<Clock>()
            .init_resource::<Darkness>()
            .add_systems(PostUpdate, apply);
    }
}

/// The sun's illuminance and the ambient light's brightness without a time of day (as
/// `lib.rs` sets them up).
pub const SUN: f32 = 5_000.0;
pub const AMBIENT: f32 = 250.0;

/// How far through the day the cycle is (0..1: 0 sunrise, 0.25 noon, 0.5 sunset, 0.75
/// midnight). Advanced by the frame's time over the day's length, so changing the length
/// doesn't jump.
#[derive(Resource)]
pub struct Clock(pub f32);

impl Default for Clock {
    /// Mid-morning.
    fn default() -> Self {
        Self(0.12)
    }
}

/// How dark it is (0 day, 1 night): how much `night`'s lamps and windows glow.
#[derive(Resource, Default)]
pub struct Darkness(pub f32);

/// The bloom's strength by day and added at night.
const BLOOM: (f32, f32) = (0.04, 0.2);

/// The grade's numbers, as the shader reads them (vectors of four: WebGL 2 wants 16 bytes).
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default, PartialEq, Debug)]
#[extract_app(RenderApp)]
pub struct Grade {
    /// Tint times exposure (rgb).
    tint: Vec4,
    /// What the bright upper part of the picture (the sky) is multiplied by (rgb), and
    /// how much (a).
    sky: Vec4,
    /// Added to the shadows in linear light (rgb); saturation (a).
    lift: Vec4,
    /// S-curve (0 none, 1 a full smoothstep), vignette, grain, seconds (for the grain).
    look: Vec4,
    /// Split toning (perceptual offsets) of the shadows (rgb); black crush (a).
    shadows: Vec4,
    /// Split toning of the highlights (rgb); colour fringes at the edges (a).
    highlights: Vec4,
}

impl FullscreenMaterial for Grade {
    fn fragment_shader() -> ShaderRef {
        "embedded://car_sim/shaders/grade.wesl".into()
    }

    /// After tonemapping (in HDR the picture is in range only then), before FXAA.
    fn schedule_configs(system: ScheduleConfigs<BoxedSystem>) -> ScheduleConfigs<BoxedSystem> {
        system
            .in_set(bevy::core_pipeline::Core3dSystems::PostProcess)
            .after(tonemapping)
            .before(bevy::anti_alias::fxaa::fxaa)
    }
}

/// A time of day: the grade and the lights.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    pub grade: Grade,
    /// The sun's colour (rgb) and its illuminance as a share of `SUN` (a).
    pub sun: Vec4,
    /// The ambient light's colour (rgb) and its brightness as a share of `AMBIENT` (a).
    pub ambient: Vec4,
    /// What the sky dome, the fog and the clear colour are multiplied by (rgb).
    pub sky: Vec4,
}

impl Look {
    /// Midday: warm sun, cool sky light, teal and orange, hard contrast.
    pub const DAY: Self = Self {
        grade: Grade {
            tint: Vec4::new(1.04, 1.0, 0.95, 0.0),
            sky: Vec4::new(0.82, 0.97, 1.0, 0.45),
            lift: Vec4::new(0.0, 0.0, 0.0, 0.88),
            look: Vec4::new(0.55, 0.32, 0.004, 0.0),
            shadows: Vec4::new(-0.045, 0.01, 0.045, 0.035),
            highlights: Vec4::new(0.035, 0.01, -0.035, 0.0025),
        },
        sun: Vec4::new(1.0, 0.95, 0.86, 1.15),
        ambient: Vec4::new(0.8, 0.95, 1.1, 0.85),
        sky: Vec4::ONE,
    };
    /// Golden hour: a low orange sun, a darker, warmer picture, the sky going orange.
    pub const DUSK: Self = Self {
        grade: Grade {
            tint: Vec4::new(1.03, 0.9, 0.78, 0.0),
            sky: Vec4::new(1.2, 0.72, 0.55, 0.6),
            lift: Vec4::new(0.006, 0.002, 0.008, 0.92),
            look: Vec4::new(0.45, 0.36, 0.006, 0.0),
            shadows: Vec4::new(-0.02, 0.0, 0.04, 0.03),
            highlights: Vec4::new(0.05, 0.01, -0.04, 0.0025),
        },
        sun: Vec4::new(1.0, 0.72, 0.5, 0.8),
        ambient: Vec4::new(0.75, 0.68, 0.85, 0.7),
        sky: Vec4::new(0.95, 0.8, 0.72, 1.0),
    };
    /// Moonlight: weak, cold light, a deep blue sky and fog, faint shadows; the grade only
    /// cools and washes it out, so lamps and windows keep their colour.
    pub const NIGHT: Self = Self {
        grade: Grade {
            tint: Vec4::new(0.85, 0.92, 1.08, 0.0),
            sky: Vec4::new(1.0, 1.0, 1.0, 0.0),
            lift: Vec4::new(0.001, 0.003, 0.008, 0.6),
            look: Vec4::new(0.35, 0.5, 0.012, 0.0),
            shadows: Vec4::new(-0.01, 0.0, 0.025, 0.02),
            highlights: Vec4::new(0.0, 0.0, 0.0, 0.002),
        },
        sun: Vec4::new(0.55, 0.68, 1.0, 0.07),
        ambient: Vec4::new(0.5, 0.62, 1.0, 0.3),
        sky: Vec4::new(0.07, 0.1, 0.2, 1.0),
    };
    /// Before sunrise: cooler and pinker than dusk.
    pub const DAWN: Self = Self {
        grade: Grade {
            tint: Vec4::new(0.82, 0.74, 0.78, 0.0),
            sky: Vec4::new(1.05, 0.78, 0.85, 0.5),
            lift: Vec4::new(0.006, 0.005, 0.012, 0.85),
            look: Vec4::new(0.35, 0.36, 0.006, 0.0),
            shadows: Vec4::new(-0.015, 0.0, 0.035, 0.025),
            highlights: Vec4::new(0.035, 0.0, -0.01, 0.0025),
        },
        sun: Vec4::new(1.0, 0.7, 0.6, 0.6),
        ambient: Vec4::new(0.7, 0.72, 0.95, 0.75),
        sky: Vec4::new(0.7, 0.62, 0.72, 1.0),
    };

    fn mix(self, other: Self, t: f32) -> Self {
        let (a, b) = (self.grade, other.grade);
        Self {
            grade: Grade {
                tint: a.tint.lerp(b.tint, t),
                sky: a.sky.lerp(b.sky, t),
                lift: a.lift.lerp(b.lift, t),
                look: a.look.lerp(b.look, t),
                shadows: a.shadows.lerp(b.shadows, t),
                highlights: a.highlights.lerp(b.highlights, t),
            },
            sun: self.sun.lerp(other.sun, t),
            ambient: self.ambient.lerp(other.ambient, t),
            sky: self.sky.lerp(other.sky, t),
        }
    }

    /// The look that far through the day (`Clock`), blended between the times of day.
    pub fn cycle(phase: f32) -> Self {
        const KEYS: [(f32, Look); 8] = [
            (0.0, Look::DAWN),
            (0.1, Look::DAY),
            (0.38, Look::DAY),
            (0.5, Look::DUSK),
            (0.58, Look::NIGHT),
            (0.9, Look::NIGHT),
            (0.97, Look::DAWN),
            (1.0, Look::DAWN),
        ];
        let phase = phase.rem_euclid(1.0);
        let next = KEYS
            .iter()
            .position(|&(at, _)| at > phase)
            .unwrap_or(KEYS.len() - 1);
        let (a, from) = KEYS[next - 1];
        let (b, to) = KEYS[next];
        let t = ((phase - a) / (b - a)).clamp(0.0, 1.0);
        from.mix(to, t * t * (3.0 - 2.0 * t))
    }

    /// How dark it is with this look (0 by day, 1 at night), from the sun's strength.
    pub fn darkness(&self) -> f32 {
        let (day, night) = (Self::DUSK.sun.w + 0.1, Self::NIGHT.sun.w);
        ((day - self.sun.w) / (day - night)).clamp(0.0, 1.0)
    }

    /// The look for the setting, at that point of the cycle.
    pub fn of(time: TimeOfDay, phase: f32) -> Self {
        match time {
            TimeOfDay::Day => Self::DAY,
            TimeOfDay::Dusk => Self::DUSK,
            TimeOfDay::Night => Self::NIGHT,
            TimeOfDay::Cycle => Self::cycle(phase),
        }
    }
}

/// Runs the clock, puts the grade on the 3D camera (or takes it off with
/// post-processing off) and sets the lights.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn apply(
    mut commands: Commands,
    graphics: Res<Graphics>,
    time: Res<Time>,
    away: Option<Res<crate::car::Away>>,
    mut clock: ResMut<Clock>,
    mut darkness: ResMut<Darkness>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut clear: ResMut<ClearColor>,
    dome: Option<Res<crate::sky::SkyMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut suns: Query<&mut DirectionalLight>,
    mut fogs: Query<&mut DistanceFog>,
    mut cameras: Query<
        (Entity, Option<&mut Grade>, Option<&mut Bloom>),
        With<crate::camera::ChaseCamera>,
    >,
) {
    clock.0 = (clock.0 + time.delta_secs() / (graphics.day_minutes as f32 * 60.0)).fract();
    let away = away.is_some_and(|away| away.0);
    let look = if graphics.post && graphics.day_night && !away {
        Look::of(graphics.time_of_day, clock.0)
    } else {
        Look::DAY
    };
    let (sun, light, sky) = if graphics.post {
        (look.sun, look.ambient, look.sky)
    } else {
        (Vec4::ONE, Vec4::ONE, Vec4::ONE)
    };
    let color = |v: Vec4| Color::linear_rgb(v.x, v.y, v.z);
    let haze = Color::from(LinearRgba::from_vec4(
        crate::sky::HAZE.to_linear().to_vec4() * sky.truncate().extend(1.0),
    ));
    if clear.0 != haze {
        clear.0 = haze;
    }
    for mut fog in &mut fogs {
        if fog.color != haze {
            fog.color = haze;
        }
    }
    if let Some(dome) = dome {
        let tint = color(sky);
        if materials.get(&dome.0).is_some_and(|m| m.base_color != tint)
            && let Some(mut material) = materials.get_mut(&dome.0)
        {
            material.base_color = tint;
        }
    }
    for mut light in &mut suns {
        light.color = color(sun);
        light.illuminance = SUN * sun.w;
    }
    ambient.color = color(light);
    ambient.brightness = AMBIENT * light.w;
    let dark = if graphics.post { look.darkness() } else { 0.0 };
    if darkness.0 != dark {
        darkness.0 = dark;
    }
    let mut grade = look.grade;
    grade.look.w = time.elapsed_secs_wrapped();
    let bloom = BLOOM.0 + BLOOM.1 * dark;
    for (entity, current, glow) in &mut cameras {
        match (graphics.post, current, glow) {
            (true, Some(mut current), Some(mut glow)) => {
                *current = grade;
                if glow.intensity != bloom {
                    glow.intensity = bloom;
                }
            }
            (true, _, _) => {
                commands.entity(entity).insert((
                    grade,
                    Bloom {
                        intensity: bloom,
                        ..Bloom::NATURAL
                    },
                ));
            }
            (false, None, None) => {}
            (false, _, _) => {
                commands.entity(entity).remove::<(Grade, Bloom, Hdr)>();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn differ(a: Look, b: Look) -> f32 {
        let (g, h) = (a.grade, b.grade);
        [
            g.tint - h.tint,
            g.sky - h.sky,
            g.lift - h.lift,
            g.look - h.look,
            g.shadows - h.shadows,
            g.highlights - h.highlights,
            a.sun - b.sun,
            a.ambient - b.ambient,
            a.sky - b.sky,
        ]
        .iter()
        .map(|d| d.abs().max_element())
        .fold(0.0, f32::max)
    }

    #[test]
    fn the_cycle_runs_through_dawn_day_dusk_and_night_and_back() {
        for (phase, look) in [
            (0.0, Look::DAWN),
            (0.25, Look::DAY),
            (0.5, Look::DUSK),
            (0.75, Look::NIGHT),
            (1.0, Look::DAWN),
        ] {
            assert!(differ(Look::cycle(phase), look) < 1e-5, "at {phase}");
        }
        // No jumps: a minute of a 24-minute day changes nothing much.
        for i in 0..1440 {
            let (a, b) = (
                Look::cycle(i as f32 / 1440.0),
                Look::cycle((i + 1) as f32 / 1440.0),
            );
            assert!(differ(a, b) < 0.05, "at {i}");
        }
    }
}
