//! Shops on the map: places to drive to for what changes a car (a showroom, parts, the
//! alignment, paint, fuel, a dyno). The server says where they are (its own list, `--shops`,
//! separate from the map; `assets/shops.cfg` by default) when a player joins; playing alone
//! there are none.
//!
//! Each has a glowing marker on the road in front of it, at the edge on its side, and its
//! name floats above it when near.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, FontId};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::camera::ChaseCamera;
use crate::car::{ActiveCar, Car};
use crate::map::{MapLegend, MapMark, MapMarks};
use crate::net::wire::{Reader, Wire};
use crate::net::{Connection, Status, WantsGhost};
use crate::terrain::Terrain;

/// The list the server has unless told otherwise (`--shops`).
pub const DEFAULT: &str = include_str!("../assets/shops.cfg");

/// A car within this distance of a marker's middle is on it (m).
pub const MARKER_RADIUS: f32 = 3.5;
/// A car going slower than this (m/s) on a marker goes in; once in, it has left when this
/// far from the marker's middle (m).
const STOPPED: f32 = 1.5;
const LEAVE: f32 = 6.0;
/// Names float above markers nearer than this (m).
const LABEL_RANGE: f32 = 500.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ShopKind {
    /// Another car (the showroom window for now).
    Showroom,
    /// Engine and car parts, repairs, tires.
    Parts,
    /// Camber, toe, tire pressures and the brakes.
    Alignment,
    Paint,
    /// Fuel of three octane ratings.
    Gas,
    /// A chassis dyno, with the ECU's tuning.
    Dyno,
}

impl ShopKind {
    pub const ALL: [ShopKind; 6] = [
        Self::Showroom,
        Self::Parts,
        Self::Alignment,
        Self::Paint,
        Self::Gas,
        Self::Dyno,
    ];

    /// Its word in a list of shops.
    pub fn word(self) -> &'static str {
        match self {
            Self::Showroom => "showroom",
            Self::Parts => "parts",
            Self::Alignment => "alignment",
            Self::Paint => "paint",
            Self::Gas => "gas",
            Self::Dyno => "dyno",
        }
    }

    /// What it is, for people.
    pub fn title(self) -> &'static str {
        match self {
            Self::Showroom => "Car dealer",
            Self::Parts => "Parts shop",
            Self::Alignment => "Alignment shop",
            Self::Paint => "Paint shop",
            Self::Gas => "Gas station",
            Self::Dyno => "Dyno",
        }
    }

    pub fn color(self) -> [u8; 3] {
        match self {
            Self::Showroom => [80, 230, 120],
            Self::Parts => [200, 120, 255],
            Self::Alignment => [90, 200, 255],
            Self::Paint => [255, 110, 190],
            Self::Gas => [255, 200, 40],
            Self::Dyno => [255, 120, 30],
        }
    }
}

/// A shop as the server lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Shop {
    pub kind: ShopKind,
    pub name: String,
    /// The building (map x and z); the marker goes on the road next to it.
    pub at: Vec2,
}

/// Reads a list of shops: one per line, its kind, x, z and name, with `#` comments and
/// blank lines skipped.
pub fn parse(text: &str) -> Result<Vec<Shop>, String> {
    let mut shops = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let wrong = |what: &str| format!("line {}: {what}: {line}", n + 1);
        let mut words = line.split_whitespace();
        let word = words.next().unwrap_or("");
        let kind = (ShopKind::ALL.into_iter())
            .find(|kind| kind.word() == word)
            .ok_or_else(|| {
                let kinds: Vec<_> = ShopKind::ALL.iter().map(|k| k.word()).collect();
                wrong(&format!("not a kind of shop ({})", kinds.join(", ")))
            })?;
        let mut number = || {
            (words.next())
                .and_then(|w| w.parse::<f32>().ok())
                .filter(|x| x.is_finite())
                .ok_or_else(|| wrong("needs x and z (m) after the kind"))
        };
        let at = Vec2::new(number()?, number()?);
        let name = words.collect::<Vec<_>>().join(" ");
        if name.is_empty() {
            return Err(wrong("needs a name after x and z"));
        }
        shops.push(Shop { kind, name, at });
    }
    if shops.len() > usize::from(u16::MAX) {
        return Err("too many shops".into());
    }
    Ok(shops)
}

impl Wire for Shop {
    fn put(&self, out: &mut Vec<u8>) {
        let kind = ShopKind::ALL.iter().position(|&k| k == self.kind);
        (kind.unwrap_or(0) as u8).put(out);
        self.name.put(out);
        self.at.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            kind: *ShopKind::ALL.get(usize::from(r.get::<u8>()?))?,
            name: r.get()?,
            at: r.get()?,
        })
    }
}

/// A shop where the client put it.
pub struct Placed {
    pub shop: Shop,
    /// The marker's middle (map x and z): on the nearest paved road, at the edge on the
    /// building's side.
    pub marker: Vec2,
    /// Along the road there (yaw), the way a car leaving it is put down.
    pub yaw: f32,
}

/// The shops on the map: the server's, once joined; none alone.
#[derive(Resource, Default)]
pub struct Shops {
    pub list: Vec<Placed>,
}

/// A glowing disc on the road and a beam above it.
#[derive(Component)]
pub struct Marker;

pub struct ShopsPlugin;

impl Plugin for ShopsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Shops>()
            .init_resource::<AtShop>()
            .add_systems(Update, (place, arrive).chain())
            .add_systems(EguiPrimaryContextPass, (labels, window));
    }
}

/// The shop the driven car is in, if any (its place in `Shops::list`).
#[derive(Resource, Default)]
pub struct AtShop {
    pub shop: Option<usize>,
    /// A shop left with Esc (or the button) while still on its marker: not gone into
    /// again until the car has moved off it.
    left: Option<usize>,
}

impl AtShop {
    fn leave(&mut self) {
        self.left = self.shop.take();
    }
}

/// Stopping on a marker is going into its shop (with the parking brake pulled); moving
/// off it, leaving. A car stopped on a marker, or in a shop, is a ghost (see
/// `WantsGhost`), so several can use a shop at once.
fn arrive(
    shops: Res<Shops>,
    mut at: ResMut<AtShop>,
    mut ghost: ResMut<WantsGhost>,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    ghost.0 = false;
    let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0) else {
        *at = AtShop::default();
        return;
    };
    let p = car.body.position.xz();
    let near = |i: Option<usize>| {
        i.and_then(|i| shops.list.get(i))
            .is_some_and(|placed| placed.marker.distance(p) < LEAVE)
    };
    if !near(at.left) {
        at.left = None;
    }
    if at.shop.is_some() && (!near(at.shop) || keys.just_pressed(KeyCode::Escape)) {
        at.leave();
    }
    let stopped = car.body.velocity.length() < STOPPED;
    let on = (shops.list.iter()).position(|placed| placed.marker.distance(p) < MARKER_RADIUS);
    if at.shop.is_none() && stopped && on.is_some() && on != at.left {
        at.shop = on;
        car.parking_brake = true;
    }
    ghost.0 = at.shop.is_some() || (stopped && on.is_some());
}

/// The window of the shop the car is in.
fn window(mut contexts: EguiContexts, shops: Res<Shops>, mut at: ResMut<AtShop>) -> Result {
    let Some(placed) = at.shop.and_then(|i| shops.list.get(i)) else {
        return Ok(());
    };
    let kind = placed.shop.kind;
    let ctx = contexts.ctx_mut()?;
    let mut leave = false;
    egui::Window::new(format!("{} ({})", placed.shop.name, kind.title()))
        .id(egui::Id::new("shop"))
        .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 40.0))
        .collapsible(false)
        .resizable(false)
        .show(ctx, |ui| {
            ui.label("Nothing here yet.");
            ui.separator();
            leave = ui
                .button("Leave (Esc)")
                .on_hover_text("Or drive off the marker (Space releases the parking brake)")
                .clicked();
        });
    if leave {
        at.leave();
    }
    Ok(())
}

/// Where a car stops for the building at `p`: on the nearest paved road, at the edge on the
/// building's side; and the yaw along the road there.
pub fn kerb(terrain: &Terrain, p: Vec2) -> (Vec2, f32) {
    let Some((point, along, width)) = terrain.paved_road_near(p) else {
        return (p, 0.0);
    };
    let side = (p - point).normalize_or_zero();
    let at = point + side * (width / 2.0 - 1.5).max(0.0);
    (at, (-along.x).atan2(-along.y))
}

/// Puts the server's shops on the map (and the map, M) when they come, and takes them off
/// when we leave.
fn place(
    mut commands: Commands,
    connection: Option<Res<Connection>>,
    (mut shops, mut at): (ResMut<Shops>, ResMut<AtShop>),
    markers: Query<Entity, With<Marker>>,
    terrain: Res<Terrain>,
    (mut meshes, mut materials): (ResMut<Assets<Mesh>>, ResMut<Assets<StandardMaterial>>),
    (mut marks, mut legend): (ResMut<MapMarks>, ResMut<MapLegend>),
) {
    let wanted: &[Shop] = match connection.as_deref() {
        Some(c) if matches!(c.status, Status::Joined { .. }) => &c.shops,
        _ => &[],
    };
    let current = shops.list.iter().map(|placed| &placed.shop);
    if current.eq(wanted.iter()) {
        return;
    }
    *at = AtShop::default();
    for marker in &markers {
        commands.entity(marker).despawn();
    }
    shops.list = (wanted.iter())
        .map(|shop| {
            let (marker, yaw) = kerb(&terrain, shop.at);
            spawn_marker(
                &mut commands,
                &mut meshes,
                &mut materials,
                &terrain,
                marker,
                shop.kind.color(),
            );
            Placed {
                shop: shop.clone(),
                marker,
                yaw,
            }
        })
        .collect();
    // On the map: each shop in its kind's colour, and a legend of the kinds there are.
    marks.0 = (shops.list.iter())
        .map(|placed| MapMark {
            at: placed.marker,
            color: placed.shop.kind.color(),
            label: placed.shop.name.clone(),
        })
        .collect();
    legend.0 = (ShopKind::ALL.into_iter())
        .filter(|&kind| shops.list.iter().any(|placed| placed.shop.kind == kind))
        .map(|kind| (kind.color(), kind.title().to_string()))
        .collect();
}

/// A marker of `color` on the ground at `at`.
fn spawn_marker(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    terrain: &Terrain,
    at: Vec2,
    [r, g, b]: [u8; 3],
) {
    let color = Color::srgb_u8(r, g, b);
    let mut glow = |alpha: f32| {
        materials.add(StandardMaterial {
            base_color: color.with_alpha(alpha),
            emissive: color.to_linear() * 2.0,
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        })
    };
    let (disc, beam) = (glow(0.45), glow(0.18));
    let ground = terrain.sample(at.x, at.y).0;
    commands
        .spawn((
            Marker,
            Transform::from_xyz(at.x, ground, at.y),
            Visibility::default(),
        ))
        .with_children(|parent| {
            parent.spawn((
                Mesh3d(meshes.add(Cylinder::new(MARKER_RADIUS, 0.05))),
                MeshMaterial3d(disc),
                Transform::from_xyz(0.0, 0.12, 0.0),
                NotShadowCaster,
            ));
            parent.spawn((
                Mesh3d(meshes.add(Cylinder::new(0.35, 40.0))),
                MeshMaterial3d(beam),
                Transform::from_xyz(0.0, 20.0, 0.0),
                NotShadowCaster,
            ));
        });
}

/// The shops' names above their markers, when they're near and in view.
fn labels(
    mut contexts: EguiContexts,
    shops: Res<Shops>,
    terrain: Res<Terrain>,
    cameras: Query<(&Camera, &GlobalTransform), With<ChaseCamera>>,
) -> Result {
    if shops.list.is_empty() {
        return Ok(());
    }
    let Ok((camera, view)) = cameras.single() else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let painter = ctx.layer_painter(egui::LayerId::background());
    for placed in &shops.list {
        let at = placed.marker;
        let ground = terrain.sample(at.x, at.y).0;
        let top = Vec3::new(at.x, ground + 3.5, at.y);
        let distance = top.distance(view.translation());
        if distance > LABEL_RANGE {
            continue;
        }
        let Ok(screen) = camera.world_to_viewport(view, top) else {
            continue;
        };
        let kind = placed.shop.kind;
        let [r, g, b] = kind.color();
        let text = format!("{}\n{}  {:.0} m", placed.shop.name, kind.title(), distance);
        let galley = painter.layout_no_wrap(text, FontId::proportional(14.0), Color32::WHITE);
        let rect = Align2::CENTER_BOTTOM
            .anchor_size(egui::pos2(screen.x, screen.y), galley.size())
            .expand(5.0);
        painter.rect_filled(rect, 4.0, Color32::from_black_alpha(170));
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.5, Color32::from_rgb(r, g, b)),
            egui::StrokeKind::Inside,
        );
        painter.galley(rect.min + egui::vec2(5.0, 5.0), galley, Color32::WHITE);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_list_reads() {
        let shops = parse(DEFAULT).unwrap();
        for kind in ShopKind::ALL {
            assert!(shops.iter().any(|shop| shop.kind == kind), "no {kind:?}");
        }
    }

    #[test]
    fn a_wrong_line_says_where() {
        assert_eq!(
            parse("gas 1 2 Pumps\n# a comment\n\nbakery 3 4 Bread").unwrap_err(),
            "line 4: not a kind of shop (showroom, parts, alignment, paint, gas, dyno): \
             bakery 3 4 Bread"
        );
        assert!(parse("gas 1 Pumps").is_err());
        assert!(parse("gas 1 2").is_err());
        let shops = parse("dyno -2601.5 -3784  Rollwerk  Dyno # under the shop").unwrap();
        assert_eq!(
            shops,
            [Shop {
                kind: ShopKind::Dyno,
                name: "Rollwerk Dyno".into(),
                at: Vec2::new(-2601.5, -3784.0),
            }]
        );
    }

    #[test]
    fn every_default_shop_is_by_a_road() {
        let terrain = Terrain::baked();
        for shop in parse(DEFAULT).unwrap() {
            let (marker, _) = kerb(&terrain, shop.at);
            assert!(
                marker.distance(shop.at) < 60.0,
                "{} is {:.0} m from a paved road",
                shop.name,
                marker.distance(shop.at)
            );
        }
    }
}
