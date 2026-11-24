//! Post-processing: a colour grade over the 3D view for the time of day (day, dusk,
//! night, or a cycle through them), one full-screen pass (`shaders/grade.wesl`). It
//! only changes the picture, not the lights: night is "day for night", as in film,
//! darker, bluer and less saturated, the sky held down most. Off in the graphics
//! settings, there's no pass at all (the web starts with it off).

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
            .add_systems(PostUpdate, apply);
    }
}

/// A full day in the cycle (s): an hour a minute.
pub const CYCLE: f32 = 24.0 * 60.0;
/// Where in the cycle the game starts (0 is the start of the day).
const CYCLE_START: f32 = 0.05;

/// The grade's numbers, as the shader reads them (vectors of four: WebGL 2 wants 16 bytes).
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType, Default, PartialEq, Debug)]
#[extract_app(RenderApp)]
pub struct Grade {
    /// Tint times exposure (rgb).
    tint: Vec4,
    /// What the bright upper part of the picture (the sky) is multiplied by (rgb), and
    /// how much (a).
    sky: Vec4,
    /// Added to the shadows (rgb); saturation (a).
    lift: Vec4,
    /// Contrast, vignette, grain, seconds (for the grain).
    look: Vec4,
}

impl FullscreenMaterial for Grade {
    fn fragment_shader() -> ShaderRef {
        "embedded://car_sim/shaders/grade.wesl".into()
    }
}

impl Grade {
    /// A clear day: a little warmer and punchier than the raw picture.
    pub const DAY: Self = Self {
        tint: Vec4::new(1.03, 1.0, 0.96, 0.0),
        sky: Vec4::new(1.0, 1.0, 1.0, 0.0),
        lift: Vec4::new(0.0, 0.0, 0.0, 1.15),
        look: Vec4::new(1.08, 0.18, 0.0, 0.0),
    };
    /// Golden hour: warm, a bit darker, the sky going orange.
    pub const DUSK: Self = Self {
        tint: Vec4::new(1.05, 0.8, 0.6, 0.0),
        sky: Vec4::new(1.15, 0.75, 0.6, 0.6),
        lift: Vec4::new(0.012, 0.004, 0.012, 1.05),
        look: Vec4::new(1.1, 0.3, 0.005, 0.0),
    };
    /// Moonlight: dark, blue, washed out, the sky deep blue.
    pub const NIGHT: Self = Self {
        tint: Vec4::new(0.13, 0.19, 0.32, 0.0),
        sky: Vec4::new(0.3, 0.38, 0.6, 0.85),
        lift: Vec4::new(0.002, 0.005, 0.014, 0.35),
        look: Vec4::new(1.15, 0.5, 0.012, 0.0),
    };
    /// Before sunrise: cooler and pinker than dusk.
    pub const DAWN: Self = Self {
        tint: Vec4::new(0.78, 0.68, 0.72, 0.0),
        sky: Vec4::new(1.05, 0.78, 0.85, 0.5),
        lift: Vec4::new(0.01, 0.008, 0.02, 0.85),
        look: Vec4::new(1.08, 0.32, 0.006, 0.0),
    };

    fn mix(self, other: Self, t: f32) -> Self {
        Self {
            tint: self.tint.lerp(other.tint, t),
            sky: self.sky.lerp(other.sky, t),
            lift: self.lift.lerp(other.lift, t),
            look: self.look.lerp(other.look, t),
        }
    }

    /// The grade that far through the cycle (0..1, from the start of the day).
    pub fn cycle(phase: f32) -> Self {
        const KEYS: [(f32, Grade); 8] = [
            (0.0, Grade::DAY),
            (0.42, Grade::DAY),
            (0.5, Grade::DUSK),
            (0.58, Grade::NIGHT),
            (0.88, Grade::NIGHT),
            (0.94, Grade::DAWN),
            (0.98, Grade::DAY),
            (1.0, Grade::DAY),
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

    /// The grade for the setting, `seconds` into the game.
    pub fn of(time: TimeOfDay, seconds: f32) -> Self {
        let mut grade = match time {
            TimeOfDay::Day => Self::DAY,
            TimeOfDay::Dusk => Self::DUSK,
            TimeOfDay::Night => Self::NIGHT,
            TimeOfDay::Cycle => Self::cycle(CYCLE_START + seconds / CYCLE),
        };
        grade.look.w = seconds % 1000.0;
        grade
    }
}

/// Puts the grade on the 3D camera, or takes it off with post-processing off.
fn apply(
    mut commands: Commands,
    graphics: Res<Graphics>,
    time: Res<Time>,
    mut cameras: Query<(Entity, Option<&mut Grade>), With<crate::camera::ChaseCamera>>,
) {
    for (entity, grade) in &mut cameras {
        match (graphics.post, grade) {
            (true, Some(mut grade)) => {
                *grade = Grade::of(graphics.time_of_day, time.elapsed_secs())
            }
            (true, None) => {
                commands
                    .entity(entity)
                    .insert(Grade::of(graphics.time_of_day, time.elapsed_secs()));
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

    fn differ(a: Grade, b: Grade) -> f32 {
        [
            a.tint - b.tint,
            a.sky - b.sky,
            a.lift - b.lift,
            a.look - b.look,
        ]
        .iter()
        .map(|d| d.abs().max_element())
        .fold(0.0, f32::max)
    }

    #[test]
    fn the_cycle_runs_through_day_dusk_night_and_dawn_and_back() {
        for (phase, grade) in [
            (0.2, Grade::DAY),
            (0.5, Grade::DUSK),
            (0.7, Grade::NIGHT),
            (0.94, Grade::DAWN),
            (1.0, Grade::DAY),
        ] {
            assert!(differ(Grade::cycle(phase), grade) < 1e-5, "at {phase}");
        }
        // No jumps: a minute of game time changes nothing much.
        for i in 0..1440 {
            let (a, b) = (
                Grade::cycle(i as f32 / 1440.0),
                Grade::cycle((i + 1) as f32 / 1440.0),
            );
            assert!(differ(a, b) < 0.05, "at {i}");
        }
    }
}
