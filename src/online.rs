//! Playing alone or on a server: the start menu (name, server address, "Play alone",
//! "Join server"; on the web only the name and "Join", for the server the page came
//! from), what's shown while joining or when it fails, the players' list with
//! "Leave" while on a server, name tags (`name_tags`), and the players the
//! server puts on the map (`MapCars`), and the chat (`chat`). Joined, the car is put in a lane near the spawn
//! point, in a spot of its own (by player number).

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use crate::car::{ActiveCar, Car, SETTINGS};
use crate::chassis::PAINTS;
use crate::help::Intro;
use crate::map::{MapCar, MapCars};
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
            .add_systems(EguiPrimaryContextPass, (menu, players));
    }
}

/// The start menu.
#[derive(Resource)]
pub struct Menu {
    pub open: bool,
    settings: Settings,
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
) -> Result {
    // After the controls screen.
    if intro.is_some_and(|intro| intro.open) {
        return Ok(());
    }
    let ctx = contexts.ctx_mut()?;
    let window = |title: &str| {
        egui::Window::new(title.to_string())
            .anchor(Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .order(egui::Order::Foreground)
            .collapsible(false)
            .resizable(false)
    };
    if let Some(mut connection) = connection {
        match connection.status.clone() {
            Status::Joined { .. } => {}
            Status::Joining => {
                window("Joining").show(ctx, |ui| {
                    ui.label(format!("Joining {} …", menu.settings.address));
                    if ui.button("Cancel").clicked() {
                        connection.close();
                        commands.remove_resource::<Connection>();
                        menu.open = true;
                    }
                });
            }
            Status::Refused(why) | Status::Lost(why) => {
                window("Not on the server").show(ctx, |ui| {
                    ui.label(why);
                    if ui.button("Back").clicked() {
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
    window("Car Sim").show(ctx, |ui| {
        let settings = &mut menu.settings;
        egui::Grid::new("online").num_columns(2).show(ui, |ui| {
            ui.label("Your name");
            ui.text_edit_singleline(&mut settings.name);
            ui.end_row();
            // On the web, the page's server is joined: no address to give.
            if !cfg!(target_arch = "wasm32") {
                ui.label("Server");
                ui.add(
                    egui::TextEdit::singleline(&mut settings.address).hint_text("cars.example.org"),
                );
                ui.end_row();
            }
        });
        ui.horizontal(|ui| {
            // On the web, the page came from the server: always online.
            if !cfg!(target_arch = "wasm32") && ui.button("Play alone").clicked() {
                menu.open = false;
            }
            let can_join = !settings.address.trim().is_empty() && !settings.name.trim().is_empty();
            let join = if cfg!(target_arch = "wasm32") {
                "Join"
            } else {
                "Join server"
            };
            if ui.add_enabled(can_join, egui::Button::new(join)).clicked() {
                settings.save();
                let link = net::connect(&settings.address);
                commands.insert_resource(Connection::new(link, settings.name.trim(), &terrain));
                menu.open = false;
            }
        });
    });
    Ok(())
}

/// On a server: who's there with their pings (ours too), and leaving it.
fn players(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut menu: ResMut<Menu>,
    connection: Option<ResMut<Connection>>,
) -> Result {
    let Some(mut connection) = connection else {
        return Ok(());
    };
    let Status::Joined { you } = connection.status else {
        return Ok(());
    };
    egui::Window::new(format!("Online ({} more)", connection.players.len()))
        .id(egui::Id::new("players"))
        .anchor(Align2::LEFT_TOP, egui::vec2(8.0, 8.0))
        .default_open(false)
        .resizable(false)
        .show(contexts.ctx_mut()?, |ui| {
            let ping =
                |id| (connection.pings.get(&id)).map_or(String::new(), |ms| format!("{ms} ms"));
            egui::Grid::new("players").num_columns(3).show(ui, |ui| {
                // And the event each is in, if any.
                let event = |id| {
                    (connection.busy.get(&id))
                        .and_then(|&e| connection.events.get(usize::from(e)))
                        .map_or(String::new(), |event| event.name.clone())
                };
                ui.label(format!("You: {}", menu.settings.name));
                ui.label(ping(you));
                ui.label(event(you));
                ui.end_row();
                for (&id, name) in &connection.players {
                    ui.label(name);
                    ui.label(ping(id));
                    ui.label(event(id));
                    ui.end_row();
                }
            });
            ui.separator();
            if ui.button("Leave the server").clicked() {
                connection.close();
                commands.remove_resource::<Connection>();
                menu.open = true;
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
