//! Map of the whole area (K, or a wheel button bound to "Map"): the ground in its land use
//! colours as one image, with roads, houses and the cars drawn on top (the app's own,
//! and whatever others it lists in `MapCars`, e.g. other players). Drag to move,
//! scroll to zoom. A click is a `MapClicked` message for whatever the app does with it
//! (the sandbox's `TeleportPlugin` puts the car on the nearest road there). Driving goes
//! on underneath. The ground image is built a few rows per frame from startup, so the
//! map opens at once.

use std::time::Duration;

use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId, Pos2, Rect, Shape, Stroke, pos2, vec2};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::buildings::Footprint;
use crate::car::{ActiveCar, Car};
use crate::input::WheelInput;
use crate::terrain::{self, RoadKind, Terrain};

/// Metres per pixel of the ground image: 1920 pixels across this map, within the 2048
/// that every GPU (and WebGL2) supports as a texture size.
const PIXEL: f32 = 5.0;
/// Houses are drawn when zoomed in to fewer metres per screen point than this.
const HOUSES_BELOW: f32 = 3.0;
/// Closest zoom (metres per screen point).
const MIN_SCALE: f32 = 0.2;
/// A click puts the car on the nearest road within this distance (m) of where it was.
const PLACE_WITHIN: f32 = 250.0;
/// Narrowest a road is drawn, in screen points, however far out the map is zoomed.
const MIN_ROAD_WIDTH: f32 = 1.5;
/// Time per frame spent building the ground image until it's done (it takes about 0.1 s
/// natively).
const BUILD_BUDGET: Duration = Duration::from_millis(3);

const BACKGROUND: Color32 = Color32::from_rgb(28, 30, 34);
const PANEL: Color32 = Color32::from_rgba_premultiplied(10, 12, 16, 190);
const HOUSE: Color32 = Color32::from_rgb(150, 82, 66);
const TEXT: Color32 = Color32::from_rgb(235, 235, 230);

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapView>()
            .init_resource::<MapMarks>()
            .init_resource::<MapCars>()
            .add_message::<MapClicked>()
            .add_systems(Update, toggle)
            .add_systems(
                EguiPrimaryContextPass,
                (
                    prepare.run_if(|view: Res<MapView>| view.layers.is_none()),
                    draw.run_if(|view: Res<MapView>| view.open),
                )
                    .chain(),
            );
    }
}

/// Clicking puts the car being driven on the nearest road there.
pub struct TeleportPlugin;

impl Plugin for TeleportPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, |mut view: ResMut<MapView>| {
            view.click = Some("drive from there");
        })
        .add_systems(Update, teleport);
    }
}

/// Spots an app wants on the map (places, destinations): a dot and a label each.
#[derive(Resource, Default)]
pub struct MapMarks(pub Vec<MapMark>);

pub struct MapMark {
    pub at: Vec2,
    pub color: [u8; 3],
    pub label: String,
}

/// Cars other than the app's own to show (other players, as a server tells): an arrow in
/// their paint and a label each.
#[derive(Resource, Default)]
pub struct MapCars(pub Vec<MapCar>);

pub struct MapCar {
    pub at: Vec2,
    /// Which way it points (a unit vector, x east, z south).
    pub forward: Vec2,
    pub color: Color,
    pub label: String,
}

/// A click on the map, at this map point (x east, z south).
#[derive(Message)]
pub struct MapClicked(pub Vec2);

#[derive(Resource)]
pub struct MapView {
    pub open: bool,
    /// What a click does, for the help line; clicks only count with one.
    pub click: Option<&'static str>,
    /// The key that opens and closes it (K in the driving apps).
    pub key: KeyCode,
    /// Map point in the middle of the screen (x east, z south).
    center: Vec2,
    /// Metres per screen point; 0 until the map is first drawn, which fits the whole map.
    scale: f32,
    /// The ground image while it's being built.
    ground: Option<GroundImage>,
    /// Built from startup (see `prepare`).
    layers: Option<Layers>,
}

impl Default for MapView {
    fn default() -> Self {
        Self {
            open: false,
            click: None,
            key: KeyCode::KeyK,
            center: Vec2::ZERO,
            scale: 0.0,
            ground: None,
            layers: None,
        }
    }
}

impl MapView {
    /// Forgets the drawn layers, so they're built again from the map (after a re-bake).
    pub fn rebuild(&mut self) {
        self.ground = None;
        self.layers = None;
    }
}

/// The ground in its land use colours, a pixel per `PIXEL` metres, built row by row.
struct GroundImage {
    size: [usize; 2],
    pixels: Vec<Color32>,
}

impl GroundImage {
    fn new(terrain: &Terrain) -> Self {
        let size = terrain.half_size() * 2.0 / PIXEL;
        let size = [size.x as usize, size.y as usize];
        Self {
            size,
            pixels: Vec::with_capacity(size[0] * size[1]),
        }
    }

    /// Builds rows for up to `budget`; whether the image is complete.
    fn advance(&mut self, terrain: &Terrain, budget: Duration) -> bool {
        let start = Instant::now();
        let [w, h] = self.size;
        while self.pixels.len() < w * h {
            let py = self.pixels.len() / w;
            for px in 0..w {
                let p = (Vec2::new(px as f32, py as f32) + 0.5) * PIXEL - terrain.half_size();
                // The bare terrain's slope: the roads are drawn over it anyway, and their
                // lookup made this three times slower.
                let up = terrain.land(p.x, p.y).1.y;
                let class = terrain.land_class(p.x, p.y);
                let color = terrain::ground_color(class, up, py * w + px);
                self.pixels.push(color32(color));
            }
            if start.elapsed() >= budget {
                break;
            }
        }
        self.pixels.len() == w * h
    }
}

/// What the map draws, prepared once.
struct Layers {
    ground: egui::TextureHandle,
    /// Each road's bounding box, to skip roads off screen.
    road_bounds: Vec<(Vec2, Vec2)>,
    houses: Vec<[Vec2; 4]>,
}

impl Layers {
    fn new(ctx: &egui::Context, terrain: &Terrain, image: GroundImage) -> Self {
        let ground = ctx.load_texture(
            "map ground",
            egui::ColorImage::new(image.size, image.pixels),
            egui::TextureOptions::NEAREST,
        );
        let road_bounds = terrain
            .roads
            .iter()
            .map(|road| {
                let lo = road
                    .points
                    .iter()
                    .copied()
                    .reduce(Vec2::min)
                    .unwrap_or_default();
                let hi = road
                    .points
                    .iter()
                    .copied()
                    .reduce(Vec2::max)
                    .unwrap_or_default();
                (lo - road.width, hi + road.width)
            })
            .collect();
        let houses = terrain
            .buildings
            .iter()
            .filter_map(|building| Footprint::fit(&building.outline))
            .map(|footprint| footprint.corners())
            .collect();
        Self {
            ground,
            road_bounds,
            houses,
        }
    }
}

/// K (the view's key), the wheel's map button or (to close) Escape. Opening centres the
/// map on the car being driven (without one, where the camera is), at the last zoom.
fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    wheel: Option<Res<WheelInput>>,
    active: Option<Res<ActiveCar>>,
    cars: Query<&Car>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
    egui: Option<Res<bevy_egui::input::EguiWantsInput>>,
    mut view: ResMut<MapView>,
) {
    // Not while typing into a field.
    if egui.is_some_and(|e| e.wants_keyboard_input()) {
        return;
    }
    let close = view.open && keys.just_pressed(KeyCode::Escape);
    if !(keys.just_pressed(view.key) || wheel.is_some_and(|w| w.map) || close) {
        return;
    }
    view.open = !view.open;
    if !view.open || view.scale <= 0.0 {
        return;
    }
    let active = active.map_or(usize::MAX, |a| a.0);
    if let Some(car) = cars.iter().find(|car| car.index == active) {
        view.center = car.body.position.xz();
    } else if let Ok(camera) = camera.single() {
        view.center = camera.translation().xz();
    }
}

/// Builds the ground image a little every frame from startup (all at once if the map is
/// already open), then the layers.
fn prepare(mut contexts: EguiContexts, terrain: Res<Terrain>, mut view: ResMut<MapView>) -> Result {
    let budget = if view.open {
        Duration::MAX
    } else {
        BUILD_BUDGET
    };
    let ground = view
        .ground
        .get_or_insert_with(|| GroundImage::new(&terrain));
    if !ground.advance(&terrain, budget) {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let image = view.ground.take().expect("built above");
    view.layers = Some(Layers::new(ctx, &terrain, image));
    Ok(())
}

fn draw(
    mut contexts: EguiContexts,
    terrain: Res<Terrain>,
    active: Option<Res<ActiveCar>>,
    cars: Query<&Car>,
    (marks, others): (Res<MapMarks>, Res<MapCars>),
    mut view: ResMut<MapView>,
    mut clicks: MessageWriter<MapClicked>,
) -> Result {
    let view = &mut *view;
    let active = active.map_or(usize::MAX, |a| a.0);
    let ctx = contexts.ctx_mut()?;
    let screen = ctx.content_rect();
    let half = terrain.half_size();
    let fit = (2.0 * half.x / screen.width()).max(2.0 * half.y / screen.height()) * 1.05;
    if view.scale <= 0.0 {
        view.scale = fit;
    }
    let clicked = egui::Area::new(egui::Id::new("map"))
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            let response = ui.allocate_rect(screen, egui::Sense::click_and_drag());
            // Drag to move; scroll (or pinch) to zoom about the pointer.
            if response.dragged() {
                let drag = response.drag_delta();
                view.center -= Vec2::new(drag.x, drag.y) * view.scale;
            }
            if let Some(pointer) = response.hover_pos() {
                let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
                let zoom = (-scroll * 0.003).exp() / pinch;
                if zoom != 1.0 {
                    let offset =
                        Vec2::new(pointer.x - screen.center().x, pointer.y - screen.center().y);
                    let under = view.center + offset * view.scale;
                    view.scale = (view.scale * zoom).clamp(MIN_SCALE, fit * 1.5);
                    view.center = under - offset * view.scale;
                }
            }
            view.center = view.center.clamp(-half, half);

            let (center, scale) = (view.center, view.scale);
            let clicked = (response.clicked() && view.click.is_some())
                .then(|| response.interact_pointer_pos())
                .flatten()
                .map(|p| {
                    center + Vec2::new(p.x - screen.center().x, p.y - screen.center().y) * scale
                });
            let to_screen =
                |p: Vec2| screen.center() + vec2(p.x - center.x, p.y - center.y) / scale;
            let visible = |lo: Vec2, hi: Vec2| {
                let (a, b) = (to_screen(lo), to_screen(hi));
                Rect::from_min_max(a, b).intersects(screen)
            };
            let painter = ui.painter_at(screen);
            let layers = view.layers.as_ref().expect("map layers built by `prepare`");
            painter.rect_filled(screen, 0.0, BACKGROUND);
            painter.image(
                layers.ground.id(),
                Rect::from_min_max(to_screen(-half), to_screen(half)),
                Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
                Color32::WHITE,
            );

            // Streams first, then tracks, then paved roads on top.
            for kind in RoadKind::ALL {
                let color = color32(kind.color());
                for (road, &(lo, hi)) in terrain.roads.iter().zip(&layers.road_bounds) {
                    if road.kind != kind || !visible(lo, hi) {
                        continue;
                    }
                    let width = (road.width / scale).max(MIN_ROAD_WIDTH);
                    let points = road.points.iter().map(|p| to_screen(*p)).collect();
                    painter.line(points, Stroke::new(width, color));
                }
            }

            if scale < HOUSES_BELOW {
                for house in &layers.houses {
                    if visible(house[0], house[0]) || visible(house[2], house[2]) {
                        let points = house.iter().map(|p| to_screen(*p)).collect();
                        painter.add(Shape::convex_polygon(points, HOUSE, Stroke::NONE));
                    }
                }
            }

            for mark in &marks.0 {
                let at = to_screen(mark.at);
                let [r, g, b] = mark.color;
                let color = Color32::from_rgb(r, g, b);
                painter.circle(at, 6.0, color, Stroke::new(1.5, Color32::BLACK));
                label(
                    &painter,
                    at + vec2(12.0, 0.0),
                    Align2::LEFT_CENTER,
                    &mark.label,
                );
            }

            // The cars as arrows, the one being driven last, larger and outlined in white.
            for other in &others.0 {
                let at = to_screen(other.at);
                arrow(
                    &painter,
                    at,
                    other.forward,
                    8.0,
                    other.color,
                    Color32::BLACK,
                );
                label(
                    &painter,
                    at + vec2(12.0, 0.0),
                    Align2::LEFT_CENTER,
                    &other.label,
                );
            }
            let mut cars: Vec<&Car> = cars.iter().collect();
            cars.sort_by_key(|car| car.index == active);
            for car in cars {
                let driven = car.index == active;
                let forward = (car.body.rotation * Vec3::NEG_Z)
                    .xz()
                    .normalize_or(Vec2::NEG_Y);
                let (size, outline) = if driven {
                    (11.0, Color32::WHITE)
                } else {
                    (8.0, Color32::BLACK)
                };
                let at = to_screen(car.body.position.xz());
                arrow(&painter, at, forward, size, car.setup.color, outline);
            }

            scale_bar(&painter, screen, scale);
            let click = view
                .click
                .map_or(String::new(), |c| format!("Click: {c}    "));
            let key = format!("{:?}", view.key);
            let key = key.trim_start_matches("Key");
            let help = format!("Drag: move    Scroll: zoom    {click}{key} / Esc: close");
            label(
                &painter,
                screen.left_bottom() + vec2(12.0, -12.0),
                Align2::LEFT_BOTTOM,
                &help,
            );
            label(
                &painter,
                screen.right_top() + vec2(-12.0, 12.0),
                Align2::RIGHT_TOP,
                "Map data © OpenStreetMap contributors",
            );
            clicked
        })
        .inner;
    if let Some(target) = clicked {
        clicks.write(MapClicked(target));
    }
    Ok(())
}

/// A click puts the car being driven on the nearest road, facing as close to its current
/// heading as the road allows, and closes the map.
fn teleport(
    mut clicks: MessageReader<MapClicked>,
    terrain: Res<Terrain>,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
    mut view: ResMut<MapView>,
) {
    for &MapClicked(target) in clicks.read() {
        let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0) else {
            continue;
        };
        let heading = (car.body.rotation * Vec3::NEG_Z).xz();
        if let Some((spot, yaw)) = terrain.drive_spot(target, heading)
            && spot.distance(target) < PLACE_WITHIN
        {
            car.place(&terrain, spot, yaw);
            view.open = false;
        }
    }
}

/// A bar of a round length (1, 2 or 5 times a power of ten metres) about 120 points long.
/// A car pointing along `forward`, `size` points from its middle to its nose.
fn arrow(
    painter: &egui::Painter,
    at: Pos2,
    forward: Vec2,
    size: f32,
    color: Color,
    outline: Color32,
) {
    let (ahead, side) = (vec2(forward.x, forward.y), vec2(-forward.y, forward.x));
    let points = vec![
        at + ahead * size,
        at - ahead * size * 0.7 + side * size * 0.7,
        at - ahead * size * 0.3,
        at - ahead * size * 0.7 - side * size * 0.7,
    ];
    // Not convex (the notch at the back), so filled as two triangles.
    let fill = color32(color.to_linear().to_f32_array());
    for tri in [[0, 1, 2], [0, 2, 3]] {
        let tri = tri.iter().map(|&i| points[i]).collect();
        painter.add(Shape::convex_polygon(tri, fill, Stroke::NONE));
    }
    painter.add(Shape::closed_line(points, Stroke::new(1.5, outline)));
}

fn scale_bar(painter: &egui::Painter, screen: Rect, scale: f32) {
    let target = 120.0 * scale;
    let power = 10f32.powf(target.log10().floor());
    let metres = [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|f| f * power)
        .rfind(|&m| m <= target)
        .unwrap_or(power);
    let length = metres / scale;
    let right = screen.right_bottom() + vec2(-16.0, -16.0);
    let left = right - vec2(length, 0.0);
    let text = if metres >= 1000.0 {
        format!("{} km", metres / 1000.0)
    } else {
        format!("{metres} m")
    };
    let background = Rect::from_min_max(left - vec2(8.0, 26.0), right + vec2(8.0, 6.0));
    painter.rect_filled(background, 4.0, PANEL);
    painter.line(
        vec![left - vec2(0.0, 5.0), left, right, right - vec2(0.0, 5.0)],
        Stroke::new(2.0, TEXT),
    );
    painter.text(
        pos2((left.x + right.x) / 2.0, left.y - 6.0),
        Align2::CENTER_BOTTOM,
        text,
        FontId::proportional(13.0),
        TEXT,
    );
}

/// Text on a dark box, anchored at `at`.
fn label(painter: &egui::Painter, at: Pos2, anchor: Align2, text: &str) {
    let galley = painter.layout_no_wrap(text.into(), FontId::proportional(13.0), TEXT);
    let rect = anchor.anchor_size(at, galley.size());
    painter.rect_filled(rect.expand(6.0), 4.0, PANEL);
    painter.galley(rect.min, galley, TEXT);
}

/// A linear colour (as the meshes use) for egui, which works in sRGB.
fn color32(linear: [f32; 4]) -> Color32 {
    let [r, g, b, _] = Srgba::from(LinearRgba::from_f32_array(linear)).to_u8_array();
    Color32::from_rgb(r, g, b)
}
