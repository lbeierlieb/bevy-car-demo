//! The graphics settings (central menu, Settings, Graphics), kept in
//! `~/.config/car-sim/settings/graphics.cfg`: shadows, antialiasing and how far the land
//! is seen; natively also vsync and a frame rate limit. The web starts on the quick settings, native on the nice ones. Only the
//! driving apps have them; the editors always draw at the nicest.

use bevy::anti_alias::fxaa::Fxaa;
use bevy::light::{CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::prelude::*;

use crate::car::SETTINGS;

pub struct GraphicsPlugin;

impl Plugin for GraphicsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Graphics::load())
            .add_systems(PostUpdate, (apply, save));
        #[cfg(not(target_arch = "wasm32"))]
        app.add_systems(PostUpdate, present)
            .add_systems(Last, limit_frame_rate);
    }
}

/// The sun's shadows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shadows {
    Off,
    /// One cascade of 1024² to 60 m.
    Low,
    /// Four cascades of 2048² to 120 m (WebGL 2 has only the first cascade).
    High,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Antialiasing {
    Off,
    /// A cheap pass over the finished picture.
    Fxaa,
    /// 4× multisampling: smoother edges, much more memory traffic.
    Msaa,
}

/// How far the land is seen (the fog's end) and loaded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ViewDistance {
    Near,
    Far,
}

impl ViewDistance {
    /// Where the fog starts and ends (m).
    pub fn fog(self) -> (f32, f32) {
        match self {
            Self::Near => (120.0, 320.0),
            Self::Far => (180.0, 480.0),
        }
    }

    /// Chunks whose centre is within the first distance of the camera are loaded (the
    /// fog's end plus half a chunk diagonal), dropped beyond the second (a little
    /// further out, so they don't flicker in and out at the edge).
    pub fn reach(self) -> (f32, f32) {
        let end = self.fog().1;
        (end + 80.0, end + 180.0)
    }
}

/// The frame rate limits offered (frames per second; none: as fast as it goes).
pub const FRAME_CAPS: [Option<u32>; 6] = [None, Some(30), Some(60), Some(90), Some(120), Some(144)];

/// The graphics settings.
#[derive(Resource, Clone, Copy, PartialEq, Debug)]
pub struct Graphics {
    pub shadows: Shadows,
    pub antialiasing: Antialiasing,
    pub view: ViewDistance,
    /// Native only (the browser paces the page itself): wait for the display's refresh.
    pub vsync: bool,
    /// Native only: at most this many frames per second.
    pub frame_cap: Option<u32>,
}

impl Default for Graphics {
    /// Quick on the web, nice natively.
    fn default() -> Self {
        if cfg!(target_arch = "wasm32") {
            Self {
                shadows: Shadows::Low,
                antialiasing: Antialiasing::Fxaa,
                view: ViewDistance::Near,
                ..Self::NICEST
            }
        } else {
            Self::NICEST
        }
    }
}

impl Graphics {
    /// What the editors draw with, and native's default.
    pub const NICEST: Self = Self {
        shadows: Shadows::High,
        antialiasing: Antialiasing::Msaa,
        view: ViewDistance::Far,
        vsync: true,
        frame_cap: None,
    };

    fn load() -> Self {
        Self::parse(&SETTINGS.load("graphics").unwrap_or_default())
    }

    /// Reads the settings file over the defaults, skipping what it doesn't know.
    fn parse(text: &str) -> Self {
        let mut graphics = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once(' ') else {
                continue;
            };
            match (key, value.trim()) {
                ("shadows", "off") => graphics.shadows = Shadows::Off,
                ("shadows", "low") => graphics.shadows = Shadows::Low,
                ("shadows", "high") => graphics.shadows = Shadows::High,
                ("antialiasing", "off") => graphics.antialiasing = Antialiasing::Off,
                ("antialiasing", "fxaa") => graphics.antialiasing = Antialiasing::Fxaa,
                ("antialiasing", "msaa") => graphics.antialiasing = Antialiasing::Msaa,
                ("view", "near") => graphics.view = ViewDistance::Near,
                ("view", "far") => graphics.view = ViewDistance::Far,
                ("vsync", "on") => graphics.vsync = true,
                ("vsync", "off") => graphics.vsync = false,
                ("frame_cap", "off") => graphics.frame_cap = None,
                ("frame_cap", fps) => {
                    if let Ok(fps) = fps.parse()
                        && FRAME_CAPS.contains(&Some(fps))
                    {
                        graphics.frame_cap = Some(fps);
                    }
                }
                _ => {}
            }
        }
        graphics
    }

    fn text(&self) -> String {
        let shadows = match self.shadows {
            Shadows::Off => "off",
            Shadows::Low => "low",
            Shadows::High => "high",
        };
        let antialiasing = match self.antialiasing {
            Antialiasing::Off => "off",
            Antialiasing::Fxaa => "fxaa",
            Antialiasing::Msaa => "msaa",
        };
        let view = match self.view {
            ViewDistance::Near => "near",
            ViewDistance::Far => "far",
        };
        let vsync = if self.vsync { "on" } else { "off" };
        let cap = self.frame_cap.map_or("off".into(), |fps| fps.to_string());
        format!(
            "shadows {shadows}\nantialiasing {antialiasing}\nview {view}\nvsync {vsync}\n\
             frame_cap {cap}\n"
        )
    }
}

/// The sun's shadow settings for that quality (the shadow map's size goes in
/// `DirectionalLightShadowMap`).
pub fn cascades(shadows: Shadows) -> CascadeShadowConfig {
    match shadows {
        Shadows::Low => CascadeShadowConfigBuilder {
            num_cascades: 1,
            maximum_distance: 60.0,
            ..default()
        },
        Shadows::Off | Shadows::High => CascadeShadowConfigBuilder {
            maximum_distance: 120.0,
            ..default()
        },
    }
    .build()
}

fn shadow_map_size(shadows: Shadows) -> usize {
    match shadows {
        Shadows::Low => 1024,
        Shadows::Off | Shadows::High => 2048,
    }
}

/// Puts the settings on the sun and the camera: when they change, and on whatever sun or
/// camera is new.
#[allow(clippy::type_complexity)]
fn apply(
    mut commands: Commands,
    graphics: Res<Graphics>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
    mut suns: Query<(&mut DirectionalLight, &mut CascadeShadowConfig)>,
    cameras: Query<(Entity, Ref<Camera3d>, Option<&Fxaa>), With<crate::camera::ChaseCamera>>,
    mut fogs: Query<&mut DistanceFog, With<crate::camera::ChaseCamera>>,
) {
    let changed = graphics.is_changed();
    let size = shadow_map_size(graphics.shadows);
    if shadow_map.size != size {
        shadow_map.size = size;
    }
    for (mut light, mut config) in &mut suns {
        if changed || light.is_added() {
            light.shadow_maps_enabled = graphics.shadows != Shadows::Off;
            *config = cascades(graphics.shadows);
        }
    }
    for (entity, added, fxaa) in &cameras {
        if !(changed || added.is_added()) {
            continue;
        }
        let msaa = match graphics.antialiasing {
            Antialiasing::Msaa => Msaa::Sample4,
            Antialiasing::Off | Antialiasing::Fxaa => Msaa::Off,
        };
        let mut camera = commands.entity(entity);
        camera.insert(msaa);
        match (graphics.antialiasing, fxaa.is_some()) {
            (Antialiasing::Fxaa, false) => {
                camera.insert(Fxaa::default());
            }
            (Antialiasing::Off | Antialiasing::Msaa, true) => {
                camera.remove::<Fxaa>();
            }
            _ => {}
        }
    }
    for mut fog in &mut fogs {
        if changed || fog.is_added() {
            let (start, end) = graphics.view.fog();
            fog.falloff = FogFalloff::Linear { start, end };
        }
    }
}

/// Vsync on the window, when it changes.
#[cfg(not(target_arch = "wasm32"))]
fn present(
    graphics: Res<Graphics>,
    mut windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
) {
    use bevy::window::PresentMode;
    let mode = if graphics.vsync {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    };
    for mut window in &mut windows {
        if window.present_mode != mode {
            window.present_mode = mode;
        }
    }
}

/// Holds each frame back until the limit's interval since the last one is over.
#[cfg(not(target_arch = "wasm32"))]
fn limit_frame_rate(graphics: Res<Graphics>, mut next: Local<Option<std::time::Instant>>) {
    use std::time::{Duration, Instant};
    let Some(fps) = graphics.frame_cap else {
        *next = None;
        return;
    };
    let interval = Duration::from_secs_f64(1.0 / f64::from(fps));
    let now = Instant::now();
    if let Some(due) = *next
        && due > now
    {
        std::thread::sleep(due - now);
    }
    // From when this frame was due, so the rate holds on average, but never more than a
    // frame behind (after a slow frame it doesn't race to catch up).
    let now = Instant::now();
    *next =
        Some(next.map_or(now, |due| due.max(now.checked_sub(interval).unwrap_or(now))) + interval);
}

fn save(graphics: Res<Graphics>) {
    if graphics.is_changed()
        && !graphics.is_added()
        && let Err(e) = SETTINGS.save("graphics", &graphics.text())
    {
        warn!("Couldn't save the graphics settings: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_read_back_what_was_written() {
        let graphics = Graphics {
            shadows: Shadows::Off,
            antialiasing: Antialiasing::Fxaa,
            view: ViewDistance::Near,
            vsync: false,
            frame_cap: Some(60),
        };
        assert_eq!(Graphics::parse(&graphics.text()), graphics);
        assert_eq!(Graphics::parse(&Graphics::NICEST.text()), Graphics::NICEST);
        assert_eq!(
            Graphics::parse("shadows purple\nnonsense"),
            Graphics::default()
        );
    }
}
