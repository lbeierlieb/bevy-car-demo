//! The graphics settings (central menu, Settings, Graphics), kept in
//! `~/.config/car-sim/settings/graphics.cfg`: shadows, antialiasing and how far the land
//! is seen, the 3D view's resolution; natively also vsync and a frame rate limit.
//!
//! The 3D camera draws into an image of the window's size times the render resolution,
//! and egui (on a camera of its own) paints it over the whole window, under everything
//! else (`Backdrop`), so the menus stay sharp. The image's scale factor keeps the 3D
//! camera's logical size the window's, so `world_to_viewport` gives window points.
//!
//! Post-processing (`grade`): a colour grade for the time of day, which can be switched
//! off (one full-screen pass, for slow GPUs and some browsers).
//!
//! The web starts on the quick settings, native on the nice ones. Only the
//! driving apps have them; the editors always draw at the nicest.

use std::sync::{Arc, Mutex};

use bevy::anti_alias::fxaa::Fxaa;
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::light::{CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureFormat};
use bevy::window::PrimaryWindow;
use bevy_egui::{EguiContexts, EguiGlobalSettings, EguiTextureHandle, PrimaryEguiContext, egui};

use crate::car::SETTINGS;

pub struct GraphicsPlugin;

impl Plugin for GraphicsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Graphics::load())
            // egui goes on the camera `setup` makes, not on the first one there is.
            .insert_resource(EguiGlobalSettings {
                auto_create_primary_context: false,
                ..default()
            })
            .add_plugins(crate::grade::GradePlugin)
            .add_systems(Startup, setup)
            .add_systems(Update, (settle_resolution, size_view).chain())
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

/// The colour grade's time of day (only with post-processing on).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimeOfDay {
    Day,
    Dusk,
    Night,
    /// Dawn, day, dusk and night in turn, blended, a day in `Graphics::day_minutes`.
    Cycle,
}

impl TimeOfDay {
    pub const ALL: [Self; 4] = [Self::Day, Self::Dusk, Self::Night, Self::Cycle];

    pub fn name(self) -> &'static str {
        match self {
            Self::Day => "Day",
            Self::Dusk => "Dusk",
            Self::Night => "Night",
            Self::Cycle => "Cycle",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Dusk => "dusk",
            Self::Night => "night",
            Self::Cycle => "cycle",
        }
    }
}

/// How much the tires throw up: smoke, dust, mud (`spray`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Particles {
    Off,
    Low,
    High,
}

impl Particles {
    pub const ALL: [Self; 3] = [Self::Off, Self::Low, Self::High];

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Low => "Low",
            Self::High => "High",
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Low => "low",
            Self::High => "high",
        }
    }
}

/// The lengths of a day in the cycle offered (minutes).
pub const DAY_LENGTHS: [u32; 5] = [4, 12, 24, 48, 96];

/// The frame rate limits offered (frames per second; none: as fast as it goes).
pub const FRAME_CAPS: [Option<u32>; 6] = [None, Some(30), Some(60), Some(90), Some(120), Some(144)];

/// The render resolutions offered (% of the window's pixels each way).
pub const RESOLUTIONS: [u32; 3] = [50, 75, 100];

/// The graphics settings.
#[derive(Resource, Clone, Copy, PartialEq, Debug)]
pub struct Graphics {
    pub shadows: Shadows,
    pub antialiasing: Antialiasing,
    pub view: ViewDistance,
    /// The 3D view's resolution (% of the window's pixels each way); none until the window
    /// is known on the web (75 on a high-density display, else 100).
    pub resolution: Option<u32>,
    /// Native only (the browser paces the page itself): wait for the display's refresh.
    pub vsync: bool,
    /// Native only: at most this many frames per second.
    pub frame_cap: Option<u32>,
    /// The colour grade pass (`grade`).
    pub post: bool,
    /// Whether the time of day changes the picture and the lights (off: always day).
    pub day_night: bool,
    pub time_of_day: TimeOfDay,
    /// How long a day of the cycle takes (minutes).
    pub day_minutes: u32,
    /// Tire smoke, dust and mud.
    pub particles: Particles,
}

impl Default for Graphics {
    /// Quick on the web, nice natively.
    fn default() -> Self {
        if cfg!(target_arch = "wasm32") {
            Self {
                shadows: Shadows::Low,
                antialiasing: Antialiasing::Fxaa,
                view: ViewDistance::Near,
                resolution: None,
                post: false,
                particles: Particles::Low,
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
        resolution: Some(100),
        vsync: true,
        frame_cap: None,
        post: true,
        day_night: true,
        time_of_day: TimeOfDay::Cycle,
        day_minutes: 24,
        particles: Particles::High,
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
                ("resolution", percent) => {
                    if let Ok(percent) = percent.parse()
                        && RESOLUTIONS.contains(&percent)
                    {
                        graphics.resolution = Some(percent);
                    }
                }
                ("post", "on") => graphics.post = true,
                ("post", "off") => graphics.post = false,
                ("day_night", "on") => graphics.day_night = true,
                ("day_night", "off") => graphics.day_night = false,
                ("time", key) => {
                    if let Some(&time) = TimeOfDay::ALL.iter().find(|t| t.key() == key) {
                        graphics.time_of_day = time;
                    }
                }
                ("day_length", minutes) => {
                    if let Ok(minutes) = minutes.parse()
                        && DAY_LENGTHS.contains(&minutes)
                    {
                        graphics.day_minutes = minutes;
                    }
                }
                ("particles", key) => {
                    if let Some(&p) = Particles::ALL.iter().find(|p| p.key() == key) {
                        graphics.particles = p;
                    }
                }
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
        let post = if self.post { "on" } else { "off" };
        let time = self.time_of_day.key();
        let day_night = if self.day_night { "on" } else { "off" };
        let day = self.day_minutes;
        let particles = self.particles.key();
        let cap = self.frame_cap.map_or("off".into(), |fps| fps.to_string());
        let resolution = self
            .resolution
            .map_or(String::new(), |p| format!("resolution {p}\n"));
        format!(
            "shadows {shadows}\nantialiasing {antialiasing}\nview {view}\n{resolution}\
             vsync {vsync}\nframe_cap {cap}\npost {post}\nday_night {day_night}\n\
             time {time}\nday_length {day}\nparticles {particles}\n"
        )
    }
}

/// The image the 3D camera draws into, and its egui texture for `Backdrop`.
#[derive(Resource)]
struct View {
    image: Handle<Image>,
    texture: Arc<Mutex<Option<egui::TextureId>>>,
}

/// Paints the 3D view over the whole window at the start of every egui pass, so it's
/// under whatever else goes into the background layer.
struct Backdrop(Arc<Mutex<Option<egui::TextureId>>>);

impl egui::Plugin for Backdrop {
    fn debug_name(&self) -> &'static str {
        "3D view"
    }

    fn on_begin_pass(&mut self, ui: &mut egui::Ui) {
        let Some(texture) = *self.0.lock().unwrap() else {
            return;
        };
        let rect = ui.ctx().viewport_rect();
        ui.ctx().layer_painter(egui::LayerId::background()).image(
            texture,
            rect,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
    }
}

/// The camera egui draws with (the window's only one), and the image for the 3D view.
fn setup(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            ..default()
        },
        Msaa::Off,
        PrimaryEguiContext,
    ));
    let image = Image::new_target_texture(1, 1, TextureFormat::Rgba8UnormSrgb, None);
    commands.insert_resource(View {
        image: images.add(image),
        texture: default(),
    });
}

/// The web's default resolution, once the window's pixel density is known.
fn settle_resolution(mut graphics: ResMut<Graphics>, windows: Query<&Window, With<PrimaryWindow>>) {
    if graphics.resolution.is_some() {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    if window.resolution.physical_width() > 1 {
        graphics.resolution = Some(if window.scale_factor() >= 1.5 {
            75
        } else {
            100
        });
    }
}

/// Keeps the 3D view's image the window's size times the resolution, the 3D camera
/// drawing into it, and egui knowing it.
#[allow(clippy::too_many_arguments)]
fn size_view(
    mut commands: Commands,
    graphics: Res<Graphics>,
    view: Res<View>,
    mut images: ResMut<Assets<Image>>,
    mut contexts: EguiContexts,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(Entity, &RenderTarget), With<crate::camera::ChaseCamera>>,
    mut added: Local<bool>,
) -> Result {
    let Ok(window) = windows.single() else {
        return Ok(());
    };
    if !*added {
        let Ok(ctx) = contexts.ctx_mut() else {
            return Ok(());
        };
        ctx.add_plugin(Backdrop(view.texture.clone()));
        let id = contexts.add_image(EguiTextureHandle::Weak(view.image.id()));
        *view.texture.lock().unwrap() = Some(id);
        *added = true;
    }
    let scale = graphics.resolution.unwrap_or(100) as f32 / 100.0;
    let size = UVec2::new(
        window.resolution.physical_width(),
        window.resolution.physical_height(),
    );
    let size = (size.as_vec2() * scale).round().as_uvec2().max(UVec2::ONE);
    let Some(image) = images.get(&view.image) else {
        return Ok(());
    };
    if image.size() != size {
        images.get_mut(&view.image).unwrap().resize(Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        });
    }
    let target = RenderTarget::Image(ImageRenderTarget {
        handle: view.image.clone(),
        scale_factor: window.scale_factor() * scale,
    });
    for (camera, current) in &cameras {
        let same = matches!(
            (current, &target),
            (RenderTarget::Image(a), RenderTarget::Image(b))
                if a.handle == b.handle && a.scale_factor == b.scale_factor
        );
        if !same {
            commands.entity(camera).insert(target.clone());
        }
    }
    Ok(())
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
            resolution: Some(75),
            frame_cap: Some(60),
            post: false,
            day_night: false,
            time_of_day: TimeOfDay::Night,
            day_minutes: 48,
            particles: Particles::Low,
        };
        assert_eq!(Graphics::parse(&graphics.text()), graphics);
        assert_eq!(Graphics::parse(&Graphics::NICEST.text()), Graphics::NICEST);
        assert_eq!(
            Graphics::parse("shadows purple\nnonsense"),
            Graphics::default()
        );
    }
}
