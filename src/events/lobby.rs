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
use super::{EventKind, Events, marker_spot, start_spot};
use crate::car::{ActiveCar, Car, CarSetup, Held};
use crate::menu::kit::{self, Panel, Place};
use crate::menu::nav::MenuInput;
use crate::net::protocol::PlayerId;
use crate::net::{Connection, GhostPlayers, Status, Touching, WantsGhost};
use crate::shops::MARKER_RADIUS;
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
                    .in_set(crate::menu::nav::MenuSet::Windows)
                    .run_if(resource_exists::<Connection>),
            )
            .add_systems(EguiPrimaryContextPass, window);
    }
}

/// Which course a race is on: event, route, laps.
type CourseKey = (u16, u8, u8);

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
        let key = (round.event, settings.route, settings.laps);
        if self.course.as_ref().is_none_or(|(k, _)| *k != key) {
            let event = connection.events.get(usize::from(round.event))?;
            let route = event.routes.get(usize::from(settings.route))?;
            self.course = Some((key, Course::new(route, settings.laps)));
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
pub fn leave(connection: &mut Connection, state: &mut EventState) {
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
    mut input: Option<ResMut<MenuInput>>,
    time: Res<Time<Real>>,
) {
    let mut back = || input.as_mut().is_some_and(|input| input.take_back());
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
                if p.distance(stood) > LEAVE || back() {
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
            if p.distance(stood) > LEAVE || back() {
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
    (mut held, mut ghost_players): (ResMut<Held>, ResMut<GhostPlayers>),
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
        Some(marker_spot(
            &terrain,
            network.as_deref(),
            event.at,
            round.slot(you),
        ))
    };
    if running && start != start_of(state.seen.as_ref()) {
        let round = round.as_ref().unwrap();
        state.started = now;
        state.gate_times.clear();
        state.wrong_way = None;
        let at = (connection.events.get(usize::from(round.event))).map(|event| event.at);
        let course = state.course(&connection, round).cloned();
        let place = at.and_then(|at| {
            start_spot(
                round,
                course.as_ref(),
                &terrain,
                network.as_deref(),
                at,
                you,
            )
        });
        if let (Some((p, yaw)), Some((entity, mut car))) = (
            place,
            cars.iter_mut().find(|(_, car)| car.index == active.0),
        ) {
            if let Some(chassis) = round.lent_chassis(you) {
                // A factory car for the round; ours waits (the server builds the same).
                let setup = CarSetup::factory(chassis, 0);
                let mut lent = Car::spawn_at(car.index, setup, &terrain, p.x, p.y, yaw);
                lent.drivetrain.neutral();
                lent.keep_settings(&car);
                lent.hold = round.hold(you);
                state.lent = Some(car.clone());
                commands.entity(entity).despawn();
                commands.spawn((Transform::from_translation(lent.body.position), lent));
            } else {
                car.place(&terrain, p, yaw);
                car.hold = round.hold(you);
                // On a grid, held for the countdown: off at the green light.
                if round.game.race().is_some() {
                    car.parking_brake = false;
                }
            }
        }
    }
    if was_running && !running {
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
            }
            Game::Race(settings) | Game::Trial(settings) => {
                let settings = *settings;
                let trial = matches!(round.game, Game::Trial(_));
                held.0 = elapsed < COUNTDOWN;
                ghost_players.0 = !trial && !settings.collisions;
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
            let title = format!("{} ({})", event.name, event.kind.title());
            Panel::new("event", &title)
                .place(Place::Right)
                .width(440.0)
                .back(Some("Leave"))
                .show(ctx, |ui| {
                    if let Some(last) = last {
                        kit::section(ui, &last.text());
                    }
                    if !we_are_hunted {
                        kit::note(ui, &format!("Waiting for {hunted_name} to start the hunt."));
                    } else if round.players.len() < 2 {
                        kit::note(
                            ui,
                            "Waiting for a hunter: anyone stopping on the marker joins.",
                        );
                    } else if kit::button(ui, "Start the hunt").clicked() {
                        wanted = Some(Action::Start);
                    }
                    leaving = kit::button(ui, "Leave").clicked();
                    kit::note(ui, hunt.catch.rule());
                    kit::section(ui, "Players");
                    for &id in &round.players {
                        let role = if id == round.host { "hunted" } else { "hunter" };
                        if we_are_hunted && id != you {
                            let label = format!("{}: make them the hunted", name(id));
                            if kit::button_with(ui, &label, role).clicked() {
                                wanted = Some(Action::Host(id));
                            }
                        } else {
                            kit::info(ui, &name(id), role);
                        }
                    }
                    kit::section(ui, "Rules");
                    ui.add_enabled_ui(we_are_hunted, |ui| hunt_settings(ui, &mut settings));
                    if !we_are_hunted {
                        kit::note(ui, &format!("{hunted_name} sets these."));
                    }
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
fn hunt_settings(ui: &mut egui::Ui, settings: &mut super::hunt::Settings) {
    use super::hunt::{DURATION, HEAD_START};
    let mut head_start = f32::from(settings.head_start);
    let range = f32::from(HEAD_START.0)..=f32::from(HEAD_START.1);
    kit::slider(ui, "Head start", &mut head_start, range, 1.0, |v| {
        format!("{v:.0} s")
    });
    settings.head_start = head_start as _;
    let mut minutes = f32::from(settings.duration / 60);
    let range = f32::from(DURATION.0 / 60)..=f32::from(DURATION.1 / 60);
    kit::slider(ui, "Time to get away", &mut minutes, range, 1.0, |v| {
        format!("{v:.0} min")
    });
    settings.duration = minutes as u16 * 60;
    kit::toggle(ui, "Traffic", &mut settings.traffic);
    kit::toggle(ui, "Hunters can reset (R)", &mut settings.hunters_reset);
    kit::note(
        ui,
        "A helicopter carries a hunter's car back to the road: it takes a while.",
    );
    kit::pick(
        ui,
        "The hunters",
        &mut settings.catch,
        &Catch::ALL,
        |catch| catch.title().to_string(),
    );
    kit::pick(ui, "Cars", &mut settings.cars, &Cars::ALL, |cars| {
        cars.title().to_string()
    });
}
