//! A player's side of the events: stopping on a hunt's marker goes into its lobby (a
//! window: who's in, the hunted's settings, start); stopping on a race's first asks whether
//! to drive a time trial or race (`racing`); driving off or Esc leaves. When the round
//! starts, the car goes to its spot (by a hunt's marker, a lent one if the round says so;
//! on a race's grid), held for the head start or the countdown, R as the round allows; a
//! bar at the top says what's what. Once it's over (or left), the car is back by the
//! marker (our own, as it was).

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::egui::{self, Align2, Color32, RichText};
use bevy_egui::{EguiContexts, EguiPrimaryContextPass};

use super::hunt::{CAUGHT_TIME, Cars, Catch};
use super::race::{COUNTDOWN, Course};
use super::round::{Action, Game, Phase, Play, Round};
use super::{EventKind, Events};
use crate::car::{ActiveCar, Car, CarSetup, Held, ResetRule};
use crate::chassis::CHASSIS;
use crate::net::protocol::PlayerId;
use crate::net::{Connection, GhostPlayers, Status, Touching, WantsGhost};
use crate::online::lane_spots;
use crate::shops::{MARKER_RADIUS, kerb};
use crate::terrain::Terrain;
use crate::traffic::network::Network;

/// A car slower than this on a marker goes in (m/s); in a lobby, driving this far from
/// where the car stood leaves it (m).
const STOPPED: f32 = 1.5;
const LEAVE: f32 = 8.0;
/// Asking to go in again after this long without an answer (s).
const ASK_AGAIN: f64 = 1.0;
/// The cars ours touched go to the server at most this often (s).
const TOUCHES_EVERY: f64 = 0.2;
/// The nearest hunter's distance is shown red and yellow within these (m).
const HUNTER_CLOSE: f32 = 50.0;
const HUNTER_NEAR: f32 = 200.0;
/// The players' spots by the marker: within this far of it (m), at most this many.
const SPOT_RANGE: f32 = 300.0;
const SPOTS: usize = 32;
/// The cars lent for "muscle vs rally": the hunters', the hunted's (their base engines:
/// the muscle car's smaller V8).
const MUSCLE: &str = "Harrow";
const RALLY: &str = "Skarn";

pub struct LobbyPlugin;

impl Plugin for LobbyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EventState>()
            .init_resource::<GhostPlayers>()
            .add_systems(
                Update,
                (follow, arrive, report_touches)
                    .chain()
                    .after(crate::shops::arrive)
                    .run_if(resource_exists::<Connection>),
            )
            .add_systems(EguiPrimaryContextPass, window);
    }
}

/// Which course a race is on: event, route, the other way round, laps.
type CourseKey = (u16, u8, bool, u8);

/// Where we stand with the events.
#[derive(Resource)]
pub struct EventState {
    /// The round as last seen, to notice it starting and ending.
    seen: Option<Round>,
    /// Our own car while a lent one is driven.
    lent: Option<Car>,
    /// An event left while on its marker: not gone into again until the car is off it.
    pub(super) left: Option<usize>,
    /// When we last asked to go in (s, `Time<Real>`).
    pub(super) asked: f64,
    /// Where the car stood when the lobby (or the choice at a race's marker) began:
    /// driving off from there leaves it.
    stood: Option<Vec2>,
    /// When the round under way started on our clock (s, `Time<Real>`), should the
    /// server's clock not be known yet.
    started: f64,
    /// When we last told the server which cars ours touched (s, `Time<Real>`).
    reported: f64,
    /// The race event whose marker we stopped on, asked whether to race or drive a time
    /// trial.
    pub(crate) choosing: Option<usize>,
    /// The course of the race we're in, made from the event's route.
    course: Option<(CourseKey, Course)>,
    /// When each racer passed each of their gates (s, the server's clock), as the round
    /// told: the gaps between them.
    pub(super) gate_times: HashMap<(PlayerId, u16), f64>,
    /// Since when we've been going the wrong way round (s, `Time<Real>`).
    pub(super) wrong_way: Option<f64>,
}

impl Default for EventState {
    fn default() -> Self {
        Self {
            seen: None,
            lent: None,
            left: None,
            asked: f64::NEG_INFINITY,
            stood: None,
            started: 0.0,
            reported: f64::NEG_INFINITY,
            choosing: None,
            course: None,
            gate_times: HashMap::new(),
            wrong_way: None,
        }
    }
}

impl EventState {
    /// The course of `round`, a race's or time trial's (made once).
    pub(super) fn course(&mut self, connection: &Connection, round: &Round) -> Option<&Course> {
        let settings = round.game.race()?;
        let key = (round.event, settings.route, settings.reverse, settings.laps);
        if self.course.as_ref().is_none_or(|(k, _)| *k != key) {
            let event = connection.events.get(usize::from(round.event))?;
            let route = event.routes.get(usize::from(settings.route))?;
            self.course = Some((key, Course::new(route, settings.reverse, settings.laps)));
        }
        self.course.as_ref().map(|(_, course)| course)
    }
}

/// The player we are, once joined.
pub(super) fn you(connection: &Connection) -> Option<u32> {
    match connection.status {
        Status::Joined { you } => Some(you),
        _ => None,
    }
}

/// How long the round has been under way (s).
pub(super) fn elapsed(connection: &Connection, state: &EventState, start: f64, now: f64) -> f64 {
    (connection.server_time(now)).map_or(now - state.started, |server| server - start)
}

/// Out of the round we're in (told the server; we don't wait for its answer).
pub(super) fn leave(connection: &mut Connection, state: &mut EventState) {
    if let Some(round) = connection.round.take() {
        connection.act(Action::Leave);
        state.left = Some(usize::from(round.event));
    }
}

/// When the round under way started (s, the server's clock).
fn start_of(round: Option<&Round>) -> Option<f64> {
    match round?.phase {
        Phase::Running { start, .. } => Some(start),
        Phase::Lobby { .. } => None,
    }
}

/// Stopping on a hunt's marker asks to go into its lobby, on a race's asks what to do
/// there; in the lobby (or asked), driving off or Esc leaves. Stopped on a marker or in a
/// lobby, the car is a ghost (several wait there at once).
#[allow(clippy::too_many_arguments)]
fn arrive(
    mut connection: ResMut<Connection>,
    events: Res<Events>,
    mut state: ResMut<EventState>,
    mut ghost: ResMut<WantsGhost>,
    active: Res<ActiveCar>,
    mut cars: Query<&mut Car>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time<Real>>,
) {
    if you(&connection).is_none() {
        return;
    }
    let Some(mut car) = cars.iter_mut().find(|car| car.index == active.0) else {
        return;
    };
    let now = time.elapsed_secs_f64();
    let p = car.body.position.xz();
    let marker = |i: usize| events.list.get(i).map(|placed| placed.marker);
    if (state.left).is_some_and(|i| marker(i).is_none_or(|m| m.distance(p) > LEAVE)) {
        state.left = None;
    }
    let stopped = car.body.velocity.length() < STOPPED;
    match connection.round.as_ref().map(Round::running) {
        None => {
            if let Some(i) = state.choosing {
                // Asked what to do at a race's marker.
                ghost.0 = true;
                let stood = *state.stood.get_or_insert(p);
                if p.distance(stood) > LEAVE || keys.just_pressed(KeyCode::Escape) {
                    state.choosing = None;
                    state.left = Some(i);
                }
                return;
            }
            state.stood = None;
            let on = (events.list.iter())
                .position(|placed| placed.marker.distance(p) < MARKER_RADIUS)
                .filter(|&i| stopped && state.left != Some(i));
            if let Some(i) = on {
                ghost.0 = true;
                if events.list[i].event.kind == EventKind::Race {
                    state.choosing = Some(i);
                    car.parking_brake = true;
                } else if now - state.asked > ASK_AGAIN {
                    state.asked = now;
                    car.parking_brake = true;
                    connection.act(Action::Enter(i as u16));
                }
            }
        }
        Some(false) => {
            state.choosing = None;
            ghost.0 = true;
            let stood = *state.stood.get_or_insert(p);
            if p.distance(stood) > LEAVE || keys.just_pressed(KeyCode::Escape) {
                leave(&mut connection, &mut state);
            }
        }
        Some(true) => {
            state.choosing = None;
            state.stood = None;
        }
    }
}

/// In a round under way that tagging ends, tells the server which cars ours touched (it
/// checks whose they are and when), at most every `TOUCHES_EVERY`.
fn report_touches(
    mut connection: ResMut<Connection>,
    mut touching: ResMut<Touching>,
    mut state: ResMut<EventState>,
    time: Res<Time<Real>>,
) {
    let tagging = (connection.round.as_ref()).is_some_and(|round| {
        round.running() && round.hunt().is_some_and(|hunt| hunt.catch == Catch::Tag)
    });
    let now = time.elapsed_secs_f64();
    if !tagging {
        touching.0.clear();
    } else if !touching.0.is_empty() && now - state.reported >= TOUCHES_EVERY {
        state.reported = now;
        for car in std::mem::take(&mut touching.0) {
            connection.act(Action::Touched(car));
        }
    }
}

/// Follows the round: when it starts (or a time trial starts again), the car goes to its
/// spot (lent, if so); hunters are held for the head start, racers for the countdown; R
/// follows the round's rules; in a race without contact the others are ghosts. When it
/// ends (or we left), our own car is back by the marker.
#[allow(clippy::too_many_arguments)]
pub(super) fn follow(
    mut commands: Commands,
    connection: Res<Connection>,
    mut state: ResMut<EventState>,
    terrain: Res<Terrain>,
    network: Option<Res<Network>>,
    active: Res<ActiveCar>,
    mut cars: Query<(Entity, &mut Car)>,
    (mut held, mut rule, mut ghost_players): (
        ResMut<Held>,
        ResMut<ResetRule>,
        ResMut<GhostPlayers>,
    ),
    time: Res<Time<Real>>,
) {
    let now = time.elapsed_secs_f64();
    let you = you(&connection);
    let round = connection.round.clone().filter(|_| you.is_some());
    let you = you.unwrap_or_default();
    let was_running = state.seen.as_ref().is_some_and(Round::running);
    let running = round.as_ref().is_some_and(Round::running);
    let start = start_of(round.as_ref());
    // Our spot by the round's marker: (where, yaw).
    let spot = |round: &Round| {
        let event = connection.events.get(usize::from(round.event))?;
        // Without the traffic network, by the nearest road.
        let (marker, yaw) = kerb(&terrain, event.at);
        let spots = network
            .as_deref()
            .map(|network| lane_spots(network, event.at, SPOT_RANGE, SPOTS))
            .unwrap_or_default();
        let slot = round.slot(you);
        Some(
            spots
                .get(slot % spots.len().max(1))
                .copied()
                .unwrap_or((marker, yaw)),
        )
    };
    if running && start != start_of(state.seen.as_ref()) {
        let round = round.as_ref().unwrap();
        state.started = now;
        state.gate_times.clear();
        state.wrong_way = None;
        let place = match &round.game {
            Game::Hunt(_) => spot(round),
            Game::Race(_) | Game::Trial(_) => {
                let (slot, count) = (round.slot(you), round.players.len());
                (state.course(&connection, round)).map(|course| course.grid(slot, count))
            }
        };
        if let (Some((p, yaw)), Some((entity, mut car))) = (
            place,
            cars.iter_mut().find(|(_, car)| car.index == active.0),
        ) {
            let lend = match round.game {
                Game::Hunt(settings) if settings.cars == Cars::MuscleVsRally => {
                    Some(if round.host == you { RALLY } else { MUSCLE })
                }
                _ => None,
            };
            if let Some(name) = lend {
                // A factory car for the round; ours waits.
                let chassis = CHASSIS.iter().position(|c| c.name == name).unwrap_or(0);
                let setup = CarSetup::factory(chassis, 0);
                let mut lent = Car::spawn_at(car.index, setup, &terrain, p.x, p.y, yaw);
                lent.keep_settings(&car);
                state.lent = Some(car.clone());
                commands.entity(entity).despawn();
                commands.spawn((Transform::from_translation(lent.body.position), lent));
            } else {
                car.place(&terrain, p, yaw);
                // On a grid, held for the countdown: off at the green light.
                if round.game.race().is_some() {
                    car.parking_brake = false;
                }
            }
        }
    }
    if was_running && !running {
        *rule = ResetRule::Road;
        // Back by the marker of the round we were in.
        let back = round.as_ref().or(state.seen.as_ref()).and_then(spot);
        let active_car = cars.iter_mut().find(|(_, car)| car.index == active.0);
        if let Some((entity, mut car)) = active_car {
            let (p, yaw) = back.unwrap_or((car.body.position.xz(), 0.0));
            if let Some(mut own) = state.lent.take() {
                own.place(&terrain, p, yaw);
                commands.entity(entity).despawn();
                commands.spawn((Transform::from_translation(own.body.position), own));
            } else {
                car.place(&terrain, p, yaw);
            }
        }
    }
    if running != was_running {
        state.stood = None;
    }
    // While it runs: who's held, what R does, whether the others are ghosts.
    held.0 = false;
    ghost_players.0 = false;
    if let Some(round) = round.as_ref().filter(|round| round.running()) {
        let elapsed = elapsed(&connection, &state, start.unwrap_or_default(), now);
        match &round.game {
            Game::Hunt(settings) => {
                let hunted = round.host == you;
                held.0 = !hunted && elapsed < f64::from(settings.head_start);
                *rule = if settings.hunters_reset && !hunted {
                    ResetRule::Road
                } else {
                    ResetRule::Never
                };
            }
            Game::Race(settings) | Game::Trial(settings) => {
                let settings = *settings;
                let trial = matches!(round.game, Game::Trial(_));
                held.0 = elapsed < COUNTDOWN;
                ghost_players.0 = !trial && !settings.collisions;
                let passed = (round.standing())
                    .and_then(|standing| standing.racers.iter().find(|r| r.player == you))
                    .map_or(0, |racer| usize::from(racer.gates));
                let (slot, count) = (round.slot(you), round.players.len());
                *rule = match state.course(&connection, round) {
                    Some(course) if settings.resets && !trial => {
                        let (p, yaw) = course.reset_spot(passed, slot, count);
                        ResetRule::Spot(p, yaw)
                    }
                    _ => ResetRule::Never,
                };
                // When each racer passed their gates, for the gaps.
                for racer in round.standing().iter().flat_map(|s| &s.racers) {
                    if racer.gates > 0 {
                        (state.gate_times)
                            .entry((racer.player, racer.gates))
                            .or_insert(racer.at);
                    }
                }
            }
        }
    }
    state.seen = round;
}

/// A hunt's lobby window, or the bar at the top while it runs.
fn window(
    mut contexts: EguiContexts,
    connection: Option<ResMut<Connection>>,
    mut state: ResMut<EventState>,
    time: Res<Time<Real>>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
) -> Result {
    let Some(mut connection) = connection else {
        return Ok(());
    };
    let (Some(you), Some(round)) = (you(&connection), connection.round.clone()) else {
        return Ok(());
    };
    let Some(event) = connection.events.get(usize::from(round.event)).cloned() else {
        return Ok(());
    };
    let ctx = contexts.ctx_mut()?;
    let now = time.elapsed_secs_f64();
    let name = |id: u32| {
        if id == you {
            format!("{} (you)", connection.name)
        } else {
            connection.players.get(&id).cloned().unwrap_or_default()
        }
    };
    let hunted_name = name(round.host);
    let we_are_hunted = round.host == you;
    // Races' windows are `racing`'s.
    let Game::Hunt(hunt) = round.game else {
        return Ok(());
    };
    match &round.phase {
        Phase::Lobby { last } => {
            let mut settings = hunt;
            let mut wanted = None;
            let mut leaving = false;
            egui::Window::new(format!("{} ({})", event.name, event.kind.title()))
                .id(egui::Id::new("event"))
                .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 40.0))
                .collapsible(false)
                .resizable(false)
                .show(ctx, |ui| {
                    if let Some(last) = last {
                        ui.label(RichText::new(last.text()).strong().size(16.0));
                        ui.separator();
                    }
                    ui.label(hunt.catch.rule());
                    ui.add_space(4.0);
                    egui::Grid::new("event players")
                        .num_columns(3)
                        .show(ui, |ui| {
                            for &id in &round.players {
                                ui.label(name(id));
                                ui.label(if id == round.host { "Hunted" } else { "Hunter" });
                                if we_are_hunted && id != you {
                                    if ui.button("Make hunted").clicked() {
                                        wanted = Some(Action::Host(id));
                                    }
                                } else {
                                    ui.label("");
                                }
                                ui.end_row();
                            }
                        });
                    ui.separator();
                    ui.add_enabled_ui(we_are_hunted, |ui| {
                        egui::Grid::new("event settings")
                            .num_columns(2)
                            .show(ui, |ui| settings_grid(ui, &mut settings));
                    });
                    if !we_are_hunted {
                        ui.small(format!("{hunted_name} sets these."));
                    }
                    ui.separator();
                    if !we_are_hunted {
                        ui.label(format!("Waiting for {hunted_name} to start the hunt."));
                    } else if round.players.len() < 2 {
                        ui.label("Waiting for a hunter: anyone stopping on the marker joins.");
                    } else if ui.button("Start the hunt").clicked() {
                        wanted = Some(Action::Start);
                    }
                    leaving = ui
                        .button("Leave (Esc)")
                        .on_hover_text("Or drive off (Space releases the parking brake)")
                        .clicked();
                });
            if settings != hunt {
                // Shown at once, not only once the server has it.
                if let Some(round) = connection.round.as_mut() {
                    round.game = Game::Hunt(settings);
                }
                connection.act(Action::Game(Game::Hunt(settings)));
            }
            if let Some(action) = wanted {
                connection.act(action);
            }
            if leaving {
                leave(&mut connection, &mut state);
            }
        }
        Phase::Running {
            start,
            play: Play::Hunt { slow },
        } => {
            let elapsed = elapsed(&connection, &state, *start, now);
            let head_start = f64::from(hunt.head_start);
            let left = head_start + f64::from(hunt.duration) - elapsed;
            let mut lines: Vec<(String, Color32)> = Vec::new();
            let white = Color32::WHITE;
            lines.push(if we_are_hunted {
                ("You're hunted: get away!".into(), white)
            } else {
                (format!("Hunt {hunted_name}!"), white)
            });
            if elapsed < head_start {
                let wait = (head_start - elapsed).ceil();
                lines.push(if we_are_hunted {
                    (format!("The hunters go in {wait} s"), white)
                } else {
                    (format!("You go in {wait} s"), Color32::YELLOW)
                });
            } else {
                let left = left.max(0.0) as u32;
                lines.push((format!("{}:{:02} left", left / 60, left % 60), white));
            }
            if we_are_hunted
                && let Some(distance) = nearest_hunter(&connection, &round, &active, &cars, now)
            {
                // Rounded: the map's positions are a second old, carried on.
                let step = if distance < 100.0 { 5.0 } else { 10.0 };
                let shown = (distance / step).round() * step;
                let color = match distance {
                    d if d < HUNTER_CLOSE => Color32::RED,
                    d if d < HUNTER_NEAR => Color32::YELLOW,
                    _ => white,
                };
                lines.push((format!("Nearest hunter: {shown:.0} m"), color));
            }
            if let Some(since) = slow {
                let server = connection.server_time(now).unwrap_or(*since);
                let caught_in = (CAUGHT_TIME - (server - since)).ceil().max(0.0);
                lines.push(if we_are_hunted {
                    (format!("Too slow! Caught in {caught_in} s"), Color32::RED)
                } else {
                    (
                        format!("{hunted_name} is slow: caught in {caught_in} s"),
                        Color32::LIGHT_GREEN,
                    )
                });
            }
            let mut leaving = false;
            egui::Area::new(egui::Id::new("hunt bar"))
                .anchor(Align2::CENTER_TOP, egui::vec2(0.0, 8.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .fill(Color32::from_black_alpha(180))
                        .show(ui, |ui| {
                            ui.vertical_centered(|ui| {
                                for (text, color) in &lines {
                                    ui.label(RichText::new(text).size(18.0).color(*color));
                                }
                                leaving = ui.small_button("Leave the hunt").clicked();
                            });
                        });
                });
            if leaving {
                leave(&mut connection, &mut state);
            }
        }
        Phase::Running { .. } => {}
    }
    Ok(())
}

/// How far the nearest hunter is from our car (m), as the map has them (it shows everyone
/// in a round).
fn nearest_hunter(
    connection: &Connection,
    round: &Round,
    active: &ActiveCar,
    cars: &Query<&Car>,
    now: f64,
) -> Option<f32> {
    let ours = cars
        .iter()
        .find(|car| car.index == active.0)?
        .body
        .position
        .xz();
    (connection.on_map(now))
        .filter(|player| player.id != round.host && round.players.contains(&player.id))
        .map(|player| player.position.distance(ours))
        .min_by(f32::total_cmp)
}

/// The hunted's settings, a row each.
fn settings_grid(ui: &mut egui::Ui, settings: &mut super::hunt::Settings) {
    use super::hunt::{DURATION, HEAD_START};
    ui.label("Head start");
    ui.add(egui::Slider::new(&mut settings.head_start, HEAD_START.0..=HEAD_START.1).suffix(" s"));
    ui.end_row();
    ui.label("Time to get away");
    let mut minutes = settings.duration / 60;
    ui.add(egui::Slider::new(&mut minutes, DURATION.0 / 60..=DURATION.1 / 60).suffix(" min"));
    settings.duration = minutes * 60;
    ui.end_row();
    ui.label("Traffic");
    ui.checkbox(&mut settings.traffic, "");
    ui.end_row();
    ui.label("Hunters can reset (R)");
    ui.checkbox(&mut settings.hunters_reset, "")
        .on_hover_text("A helicopter carries a hunter's car back to the road: it takes a while");
    ui.end_row();
    ui.label("The hunters");
    egui::ComboBox::from_id_salt("hunt catch")
        .selected_text(settings.catch.title())
        .show_ui(ui, |ui| {
            for catch in Catch::ALL {
                ui.selectable_value(&mut settings.catch, catch, catch.title());
            }
        });
    ui.end_row();
    ui.label("Cars");
    egui::ComboBox::from_id_salt("hunt cars")
        .selected_text(settings.cars.title())
        .show_ui(ui, |ui| {
            for cars in Cars::ALL {
                ui.selectable_value(&mut settings.cars, cars, cars.title());
            }
        });
    ui.end_row();
}
