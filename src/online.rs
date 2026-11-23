//! Playing alone or on a server: the start menu (name, server address, "Play alone",
//! "Join server"; on the web only the name and "Join", for the server the page came
//! from), what's shown while joining or when it fails (the players and leaving are in
//! the central menu), name tags (`name_tags`), and the players the
//! server puts on the map (`MapCars`), and the chat (`chat`). Joined, the car is put in a lane near the spawn
//! point, in a spot of its own (by player number).

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::car::{ActiveCar, Car, SETTINGS};
use crate::chassis::PAINTS;
use crate::help::Intro;
use crate::map::{MapCar, MapCars};
use crate::menu::kit::{self, Panel, rank};
use crate::name_tags::NameTagPlugin;
use crate::net::{self, Connection, Status};
use crate::terrain::Terrain;
use crate::traffic::network::{Leg, Network};

/// Spots for joining cars in the lanes near the spawn point: how many, at least how far
/// apart, and within how far of it (m).
const SPOTS: usize = 16;
const SPACING: f32 = 15.0;
const SPOT_RANGE: f32 = 400.0;

pub struct OnlinePlugin;

impl Plugin for OnlinePlugin {
    fn build(&self, app: &mut App) {
        let settings = Settings::load();
        app.add_plugins((crate::chat::ChatPlugin, NameTagPlugin))
            .insert_resource(Menu {
                open: true,
                settings,
            })
            .add_systems(Update, (place_on_joining, show_on_map))
            .add_systems(EguiPrimaryContextPass, menu);
    }
}

/// The start menu.
#[derive(Resource)]
pub struct Menu {
    pub open: bool,
    settings: Settings,
}

impl Menu {
    /// The player's name.
    pub fn name(&self) -> &str {
        &self.settings.name
    }
}

/// The player's name and the server last joined, kept between runs
/// (`~/.config/car-sim/settings/online.cfg`).
#[derive(Clone, Debug, Default, PartialEq)]
struct Settings {
    name: String,
    address: String,
}

impl Settings {
    fn load() -> Self {
        let mut settings = Self::default();
        for line in SETTINGS.load("online").unwrap_or_default().lines() {
            match line.split_once(' ') {
                Some(("name", name)) => settings.name = name.trim().to_string(),
                Some(("address", address)) => settings.address = address.trim().to_string(),
                _ => {}
            }
        }
        // On the web, always the server the page came from.
        #[cfg(target_arch = "wasm32")]
        {
            settings.address = net::web::page_host().unwrap_or_default();
        }
        settings
    }

    fn save(&self) {
        let text = format!("name {}\naddress {}\n", self.name, self.address);
        if let Err(e) = SETTINGS.save("online", &text) {
            warn!("Couldn't save the online settings: {e}");
        }
    }
}

fn menu(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut menu: ResMut<Menu>,
    terrain: Res<Terrain>,
    connection: Option<ResMut<Connection>>,
    intro: Option<Res<Intro>>,
    mut exit: MessageWriter<AppExit>,
) -> Result {
    // After the quick start.
    if intro.is_some_and(|intro| intro.open) {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let panel = |title| {
        Panel::new("start menu", title)
            .rank(rank::START)
            .width(460.0)
            .back(None)
    };
    if let Some(mut connection) = connection {
        match connection.status.clone() {
            Status::Joined { .. } => {}
            Status::Joining => {
                panel("Joining").show(ctx, |ui| {
                    kit::note(ui, &format!("Joining {} …", menu.settings.address));
                    if kit::button(ui, "Cancel").clicked() {
                        connection.close();
                        commands.remove_resource::<Connection>();
                        menu.open = true;
                    }
                });
            }
            Status::Refused(why) | Status::Lost(why) => {
                panel("Not on the server").show(ctx, |ui| {
                    kit::note(ui, &why);
                    if kit::button(ui, "Back").clicked() {
                        commands.remove_resource::<Connection>();
                        menu.open = true;
                    }
                });
            }
        }
        return Ok(());
    }
    if !menu.open {
        return Ok(());
    }
    let menu = &mut *menu;
    panel("Car Sim").show(ctx, |ui| {
        let settings = &mut menu.settings;
        let can_join = !settings.address.trim().is_empty() && !settings.name.trim().is_empty();
        // On the web, the page came from the server: always online.
        let web = cfg!(target_arch = "wasm32");
        let join = if web { "Join" } else { "Join server" };
        // The join button first, so choose alone joins the last server.
        let joined = ui
            .add_enabled_ui(can_join, |ui| kit::button(ui, join))
            .inner
            .clicked();
        if !web && kit::button(ui, "Play alone").clicked() {
            menu.open = false;
        }
        kit::section(ui, "Your name");
        kit::text_field(ui, &mut settings.name, "", 1);
        // On the web, the page's server is joined: no address to give.
        if !web {
            kit::section(ui, "Server");
            kit::text_field(ui, &mut settings.address, "cars.example.org", 1);
        }
        if !can_join {
            kit::note(ui, "Give your name and a server's address to join it.");
        }
        if !web && kit::button(ui, "Quit").clicked() {
            exit.write(AppExit::Success);
        }
        if joined {
            settings.save();
            let link = net::connect(&settings.address);
            commands.insert_resource(Connection::new(link, settings.name.trim(), &terrain));
            menu.open = false;
        }
    });
    Ok(())
}

/// Once joined, the car goes to a spot of its own on the road by the spawn point.
fn place_on_joining(
    connection: Option<Res<Connection>>,
    mut placed: Local<Option<u32>>,
    terrain: Res<Terrain>,
    network: Res<Network>,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
) {
    let Some(connection) = connection else {
        *placed = None;
        return;
    };
    let you = match connection.status {
        Status::Joined { you } => you,
        _ => {
            *placed = None;
            return;
        }
    };
    if *placed == Some(you) {
        return;
    }
    *placed = Some(you);
    let (start, _) = terrain.road_spawns(&connection.spawn, 1, 0.0)[0];
    let spots = lane_spots(&network, start, SPOT_RANGE, SPOTS);
    let Some(&(p, yaw)) = spots.get(you as usize % spots.len().max(1)) else {
        return;
    };
    for mut car in cars.iter_mut().filter(|car| car.index == active.0) {
        car.place(&terrain, p, yaw);
    }
}

/// The other players the server tells us of, on the map.
fn show_on_map(
    connection: Option<Res<Connection>>,
    time: Res<Time<Real>>,
    mut map: ResMut<MapCars>,
) {
    let Some(connection) = connection else {
        if !map.0.is_empty() {
            map.0.clear();
        }
        return;
    };
    let now = time.elapsed_secs_f64();
    map.0 = connection
        .on_map(now)
        .map(|player| {
            let [r, g, b] = PAINTS[player.paint].1;
            MapCar {
                at: player.position,
                forward: player.forward,
                color: Color::srgb(r, g, b),
                label: (connection.players.get(&player.id).cloned()).unwrap_or_default(),
            }
        })
        .collect();
}

/// Up to `count` places in the lanes (not on bridges) within `range` of `start`, nearest
/// first, at least `SPACING` apart, as (position in xz, yaw facing along the lane): for
/// joining cars by the spawn point, and for an event's players by its marker.
pub fn lane_spots(network: &Network, start: Vec2, range: f32, count: usize) -> Vec<(Vec2, f32)> {
    let mut candidates = Vec::new();
    for (e, edge) in network.edges.iter().enumerate() {
        for forward in [true, false] {
            let lane = network.lane(Leg {
                edge: e as u32,
                forward,
            });
            let mut s = SPACING / 2.0;
            while s < lane.length() && !edge.bridge {
                let p = lane.point(s);
                if p.distance(start) < range {
                    let d = lane.direction(s);
                    candidates.push((p, (-d.x).atan2(-d.y)));
                }
                s += SPACING;
            }
        }
    }
    candidates.sort_by(|a, b| a.0.distance(start).total_cmp(&b.0.distance(start)));
    let mut spots: Vec<(Vec2, f32)> = Vec::new();
    for spot in candidates {
        if spots.len() < count && spots.iter().all(|s| s.0.distance(spot.0) >= SPACING) {
            spots.push(spot);
        }
    }
    spots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terrain::Spawn;

    #[test]
    fn joining_cars_get_spots_of_their_own() {
        let terrain = Terrain::baked();
        let (start, _) = terrain.road_spawns(&Spawn::DEFAULT, 1, 0.0)[0];
        let spots = lane_spots(&Network::new(&terrain), start, SPOT_RANGE, SPOTS);
        assert_eq!(spots.len(), SPOTS);
    }
}
