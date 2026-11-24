//! Post-processing and the time of day: a colour grade over the 3D view (one full-screen
//! pass, `shaders/grade.wesl`) and the sun's and the sky light's colour and strength,
//! for day, dusk, night, or a cycle through them with dawn, blended all the way.
//!
//! The day is graded like a modern spy thriller: cool, teal shadows against warm
//! highlights, crushed blacks, a firm S-curve, a little less saturation, a teal sky.
//! Night is "day for night" as in film (darker, bluer, washed out, the sky held down),
//! with the sun turned into weak, cold moonlight so shadows fade. Off in the graphics
//! settings, there's no pass and the lights are the plain day's (the web starts with it
//! off). In a shop's scene (`car::Away`) it's always day, so the rooms stay lit.

use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
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
}

/// A time of day: the grade and the lights.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    pub grade: Grade,
    /// The sun's colour (rgb) and its illuminance as a share of `SUN` (a).
    pub sun: Vec4,
    /// The ambient light's colour (rgb) and its brightness as a share of `AMBIENT` (a).
    pub ambient: Vec4,
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
    };
    /// Moonlight: dark, blue, washed out, the sky deep blue, faint shadows.
    pub const NIGHT: Self = Self {
        grade: Grade {
            tint: Vec4::new(0.22, 0.3, 0.48, 0.0),
            sky: Vec4::new(0.3, 0.38, 0.6, 0.85),
            lift: Vec4::new(0.002, 0.005, 0.014, 0.35),
            look: Vec4::new(0.35, 0.5, 0.012, 0.0),
            shadows: Vec4::new(-0.01, 0.0, 0.025, 0.02),
            highlights: Vec4::new(0.0, 0.0, 0.0, 0.002),
        },
        sun: Vec4::new(0.62, 0.72, 1.0, 0.35),
        ambient: Vec4::new(0.6, 0.7, 1.0, 1.0),
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
#[allow(clippy::too_many_arguments)]
fn apply(
    mut commands: Commands,
    graphics: Res<Graphics>,
    time: Res<Time>,
    away: Option<Res<crate::car::Away>>,
    mut clock: ResMut<Clock>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut suns: Query<&mut DirectionalLight>,
    mut cameras: Query<(Entity, Option<&mut Grade>), With<crate::camera::ChaseCamera>>,
) {
    clock.0 = (clock.0 + time.delta_secs() / (graphics.day_minutes as f32 * 60.0)).fract();
    let away = away.is_some_and(|away| away.0);
    let look = if graphics.post && !away {
        Look::of(graphics.time_of_day, clock.0)
    } else {
        Look::DAY
    };
    let (sun, light) = if graphics.post {
        (look.sun, look.ambient)
    } else {
        (Vec4::ONE, Vec4::ONE)
    };
    let color = |v: Vec4| Color::linear_rgb(v.x, v.y, v.z);
    for mut light in &mut suns {
        light.color = color(sun);
        light.illuminance = SUN * sun.w;
    }
    ambient.color = color(light);
    ambient.brightness = AMBIENT * light.w;
    let mut grade = look.grade;
    grade.look.w = time.elapsed_secs_wrapped();
    for (entity, current) in &mut cameras {
        match (graphics.post, current) {
            (true, Some(mut current)) => *current = grade,
            (true, None) => {
                commands.entity(entity).insert(grade);
            }
            (false, Some(_)) => {
                commands.entity(entity).remove::<Grade>();
            }
            (false, None) => {}
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
