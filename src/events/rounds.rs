//! The server's side of the events: the rounds of car hunts, races and time trials, from
//! the lobby at a marker to the end (see `hunt`, `race`). Each running round has a realm
//! of its own: its players' entities get its `Realm` (so they see only each other and its
//! traffic), and the traffic has the round's density there (none without traffic).

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use super::hunt::{self, CAUGHT_SPEED, CAUGHT_TIME, Catch, Ending, TAG_RANGE};
use super::race::{self, BEST_KEPT, Best, COUNTDOWN, Course, FINISH_GRACE, MAX_RACERS, Racer};
use super::round::{Action, Game, Outcome, Phase, Play, Round};
use super::{Event, EventKind, marker_spot, start_spot};
use crate::car::ResetRule;
use crate::heli::{Chase, HeliPose};
use crate::net::protocol::CarId;
use crate::net::protocol::{CarState, PlayerId, ToClient};
use crate::net::server::{Peer, Server};
use crate::terrain::Terrain;
use crate::traffic::network::Network;
use crate::traffic::{Realm, Traffic};

/// How often the hunt's helicopter is sent to the players in the round (per second).
const HELI_RATE: f64 = 15.0;

/// A player asks to enter only this near the event's marker (m): stopped on it, as their
/// game sees it, give or take what their state is behind.
const ENTER_RANGE: f32 = 15.0;

/// A player asked something of the events.
#[derive(Message, Clone, Debug)]
pub struct EventAction {
    pub player: PlayerId,
    pub action: Action,
}

/// The best times' lists: per event, route and traffic.
type BestKey = (u16, u8, bool);

/// Every round there is, lobbies and running ones.
#[derive(Resource, Default)]
pub struct Rounds {
    list: Vec<Live>,
    /// The last realm given to a round.
    realm: u32,
    /// Who's in which event, as everyone was last told, and who was told.
    busy: Vec<(PlayerId, u16)>,
    told_busy: HashSet<PlayerId>,
    /// The players given a round's realm (only theirs are taken away again).
    in_realms: HashSet<PlayerId>,
    /// The time trials' best times, this session.
    best: HashMap<BestKey, Vec<Best>>,
    /// Each player in a running round: its event, when it started and their slot in it
    /// (their car was put on its start spot then).
    started: HashMap<PlayerId, (u16, f64, usize)>,
}

impl Rounds {
    /// Whether players' cars collide in `realm` (not in a race set so they don't).
    pub fn players_collide(&self, realm: Realm) -> bool {
        (self.list.iter())
            .filter(|live| live.realm == realm && realm != Realm::NORMAL)
            .all(|live| live.round.game.race().is_none_or(|race| race.collisions))
    }

    /// The rounds there are.
    pub fn iter(&self) -> impl Iterator<Item = &Round> {
        self.list.iter().map(|live| &live.round)
    }
}

/// A round on the server.
struct Live {
    round: Round,
    /// Its realm while it runs.
    realm: Realm,
    /// What the host (in a hunt, the hunted) is called (kept for the outcome should they
    /// leave).
    host_name: String,
    /// Its players need telling how it stands.
    changed: bool,
    /// The helicopter over the hunted while a hunt runs, and when it was last sent (s).
    heli: Option<Chase>,
    heli_sent: f64,
    /// A race's course while it runs, and where each racer's car was last seen (map x, z).
    course: Option<Course>,
    seen: HashMap<PlayerId, Vec2>,
}

impl Live {
    fn new(event: u16, player: PlayerId, game: Game, name: String) -> Self {
        Self {
            round: Round {
                event,
                players: vec![player],
                host: player,
                game,
                phase: Phase::Lobby { last: None },
                best: Vec::new(),
            },
            realm: Realm::NORMAL,
            host_name: name,
            changed: true,
            heli: None,
            heli_sent: f64::NEG_INFINITY,
            course: None,
            seen: HashMap::new(),
        }
    }

    fn has(&self, player: PlayerId) -> bool {
        self.round.players.contains(&player)
    }

    /// Whether others can join it in its lobby (a time trial is private).
    fn open(&self) -> bool {
        !matches!(self.round.game, Game::Trial(_))
    }

    /// The best times' list it shows.
    fn best_key(&self) -> Option<BestKey> {
        let settings = self.round.game.race()?;
        Some((self.round.event, settings.route, settings.traffic))
    }
}

/// Takes in what players asked, lets go of the ones who left, runs the rounds under way,
/// and tells everyone in a round how it stands.
#[allow(clippy::too_many_arguments)]
pub fn run_rounds(
    mut commands: Commands,
    mut rounds: ResMut<Rounds>,
    server: Res<Server>,
    mut traffic: ResMut<Traffic>,
    (time, fixed, terrain): (Res<Time<Real>>, Res<Time<Fixed>>, Res<Terrain>),
    network: Option<Res<Network>>,
    mut actions: MessageReader<EventAction>,
    mut peers: Query<(Entity, &mut Peer, Option<&Realm>)>,
) {
    let now = time.elapsed_secs_f64();
    let rounds = &mut *rounds;
    let events = server.events();
    let present: HashMap<PlayerId, (String, Option<Vec3>, Option<f32>)> = (peers.iter())
        .filter_map(|(_, peer, _)| {
            let state = peer.state;
            Some((
                peer.id,
                (
                    peer.name.clone()?,
                    state.map(|s| s.position),
                    state.map(|s| s.velocity.length()),
                ),
            ))
        })
        .collect();
    // Whose car is whose.
    let owners: HashMap<CarId, PlayerId> = (peers.iter())
        .map(|(_, peer, _)| (peer.car, peer.id))
        .collect();
    let name = |player: PlayerId| {
        present
            .get(&player)
            .map(|p| p.0.clone())
            .unwrap_or_default()
    };
    // Who's out of their round now, to be told.
    let mut out = Vec::new();

    // The ones gone from the server leave their rounds.
    for live in &mut rounds.list {
        for player in live.round.players.clone() {
            if !present.contains_key(&player) {
                leave(live, player, &name, events);
            }
        }
    }
    for EventAction { player, action } in actions.read().cloned() {
        let mine = rounds.list.iter().position(|live| live.has(player));
        // The event at `event`, if the player's car is at its marker.
        let near = |event: u16| {
            (events.get(usize::from(event)))
                .zip(present.get(&player).and_then(|p| p.1))
                .filter(|(event, at)| event.at.distance(at.xz()) < ENTER_RANGE)
                .map(|(event, _)| event)
        };
        match action {
            Action::Enter(event) => {
                let Some(kind) = near(event).map(|e| e.kind) else {
                    continue;
                };
                // The most racers the lobby's route has grid spots for.
                let room = |live: &Live| {
                    let route = (live.round.game.race())
                        .and_then(|settings| route_of(events, live.round.event, settings));
                    route.map_or(MAX_RACERS, |route| route.spots())
                };
                if mine.is_some() {
                    continue;
                }
                let lobby = (rounds.list.iter_mut())
                    .find(|live| live.round.event == event && !live.round.running() && live.open());
                match lobby {
                    Some(live)
                        if kind == EventKind::Hunt || live.round.players.len() < room(live) =>
                    {
                        live.round.players.push(player);
                        live.changed = true;
                    }
                    Some(_) => {}
                    None => {
                        let game = match kind {
                            EventKind::Hunt => Game::Hunt(default()),
                            EventKind::Race => Game::Race(default()),
                        };
                        rounds
                            .list
                            .push(Live::new(event, player, game, name(player)));
                    }
                }
            }
            Action::Trial(event) => {
                if mine.is_none() && near(event).is_some_and(|e| e.kind == EventKind::Race) {
                    let game = Game::Trial(default());
                    rounds
                        .list
                        .push(Live::new(event, player, game, name(player)));
                }
            }
            Action::Leave => {
                if let Some(i) = mine {
                    leave(&mut rounds.list[i], player, &name, events);
                    out.push(player);
                }
            }
            Action::Game(game) => {
                if let Some(live) = mine.map(|i| &mut rounds.list[i])
                    && live.round.host == player
                    && !live.round.running()
                    && let Some(game) = live.round.game.with(&game)
                {
                    let routes = (events.get(usize::from(live.round.event)))
                        .map_or(0, |event| event.routes.len());
                    live.round.game = match game {
                        Game::Hunt(settings) => Game::Hunt(settings.clamped()),
                        Game::Race(settings) => Game::Race(settings.clamped(routes)),
                        Game::Trial(settings) => Game::Trial(settings.clamped(routes)),
                    };
                    live.changed = true;
                }
            }
            Action::Host(other) => {
                if let Some(live) = mine.map(|i| &mut rounds.list[i])
                    && live.round.host == player
                    && !live.round.running()
                    && live.has(other)
                {
                    live.round.host = other;
                    live.host_name = name(other);
                    live.changed = true;
                }
            }
            Action::Touched(car) => {
                let (Some(&other), Some(live)) =
                    (owners.get(&car), mine.map(|i| &mut rounds.list[i]))
                else {
                    continue;
                };
                let (Phase::Running { start, .. }, Some(&settings)) =
                    (&live.round.phase, live.round.hunt())
                else {
                    continue;
                };
                let start = *start;
                let hunted = live.round.host;
                let hunter = match () {
                    _ if player == hunted => other,
                    _ if other == hunted => player,
                    _ => continue,
                };
                let hunting = start + f64::from(settings.head_start);
                let at = |p: PlayerId| present.get(&p).and_then(|p| p.1);
                let near =
                    (at(player).zip(at(other))).is_some_and(|(a, b)| a.distance(b) < TAG_RANGE);
                if settings.catch == Catch::Tag && live.has(hunter) && now >= hunting && near {
                    let by = name(hunter);
                    end_hunt(
                        live,
                        Ending::Tagged {
                            by,
                            after: (now - hunting) as f32,
                        },
                    );
                }
            }
            Action::Start => {
                let Some(live) = mine.map(|i| &mut rounds.list[i]) else {
                    continue;
                };
                let fewest = if matches!(live.round.game, Game::Trial(_)) {
                    1
                } else {
                    2
                };
                if live.round.host != player
                    || live.round.running()
                    || live.round.players.len() < fewest
                {
                    continue;
                }
                let event = events.get(usize::from(live.round.event));
                let play = match &live.round.game {
                    Game::Hunt(_) => Play::Hunt { slow: None },
                    Game::Race(settings) | Game::Trial(settings) => {
                        let Some(route) =
                            event.and_then(|event| event.routes.get(usize::from(settings.route)))
                        else {
                            continue;
                        };
                        if live.round.players.len() > route.spots() {
                            continue;
                        }
                        live.course = Some(Course::new(route, settings.laps));
                        live.seen.clear();
                        Play::Race(standing(&live.round.players, &name, now))
                    }
                };
                rounds.realm += 1;
                live.realm = Realm(rounds.realm);
                let traffic_on = match &live.round.game {
                    Game::Hunt(settings) => settings.traffic,
                    Game::Race(settings) | Game::Trial(settings) => settings.traffic,
                };
                let cars = if traffic_on { traffic.cars } else { 0 };
                traffic.realms.insert(live.realm, cars);
                live.round.phase = Phase::Running { start: now, play };
                live.changed = true;
                info!(
                    "A {} started in realm {} with {} players",
                    live.round.game.title().to_lowercase(),
                    live.realm.0,
                    live.round.players.len()
                );
            }
            Action::Restart => {
                if let Some(live) = mine.map(|i| &mut rounds.list[i])
                    && matches!(live.round.game, Game::Trial(_))
                    && live.round.running()
                {
                    live.seen.clear();
                    live.round.phase = Phase::Running {
                        start: now,
                        play: Play::Race(standing(&live.round.players, &name, now)),
                    };
                    live.changed = true;
                }
            }
        }
    }

    // The hunts under way: is the hunted caught, or is the time up?
    for live in &mut rounds.list {
        let Game::Hunt(settings) = live.round.game else {
            continue;
        };
        let Phase::Running {
            start,
            play: Play::Hunt { slow },
        } = &mut live.round.phase
        else {
            continue;
        };
        let hunting = *start + f64::from(settings.head_start);
        if now >= hunting + f64::from(settings.duration) {
            end_hunt(live, Ending::Escaped);
            continue;
        }
        let speed = present.get(&live.round.host).and_then(|p| p.2);
        let Some(speed) = speed.filter(|_| now >= hunting) else {
            continue;
        };
        if speed >= CAUGHT_SPEED {
            live.changed |= slow.take().is_some();
        } else if let Some(since) = *slow {
            if now - since >= CAUGHT_TIME {
                end_hunt(live, Ending::Caught((now - hunting) as f32));
            }
        } else {
            *slow = Some(now);
            live.changed = true;
        }
    }
    // The races under way: who went through their next gate, who's through, is it over?
    let mut new_best: Vec<(BestKey, Best)> = Vec::new();
    for live in &mut rounds.list {
        let positions: Vec<(PlayerId, Vec2)> = (live.round.players.iter())
            .filter_map(|&p| Some((p, present.get(&p)?.1?.xz())))
            .collect();
        let key = live.best_key();
        let Phase::Running {
            start,
            play: Play::Race(standing),
        } = &mut live.round.phase
        else {
            continue;
        };
        let Some(course) = &live.course else {
            continue;
        };
        let go = *start + COUNTDOWN;
        for (player, at) in positions {
            let before = live.seen.insert(player, at);
            let Some(racer) = (standing.racers.iter_mut()).find(|r| r.player == player) else {
                continue;
            };
            let Some(before) = before.filter(|_| now >= go && racer.finished.is_none()) else {
                continue;
            };
            let passed = usize::from(racer.gates);
            if !course.next(passed).crossed(before, at) {
                continue;
            }
            racer.gates += 1;
            racer.at = now;
            live.changed = true;
            if course.ends_lap(passed + 1) {
                let lap = (now - go) as f32 - racer.laps.iter().sum::<f32>();
                racer.laps.push(lap);
                if let Some(key) = key.filter(|_| matches!(live.round.game, Game::Trial(_))) {
                    new_best.push((
                        key,
                        Best {
                            name: racer.name.clone(),
                            time: lap,
                        },
                    ));
                }
            }
            if passed + 1 >= course.total() {
                racer.finished = Some((now - go) as f32);
                standing.closes.get_or_insert(now + FINISH_GRACE);
            }
        }
        let over = (standing.racers.iter()).all(|r| r.finished.is_some() || r.left)
            || standing.closes.is_some_and(|closes| now >= closes);
        if over {
            end_race(live, events);
        }
    }
    // A loop's laps and a sprint's whole way go on the list of best times.
    for (key, best) in new_best {
        let list = rounds.best.entry(key).or_default();
        match list.iter_mut().find(|b| b.name == best.name) {
            Some(theirs) if theirs.time <= best.time => continue,
            Some(theirs) => theirs.time = best.time,
            None => list.push(best),
        }
        list.sort_by(|a, b| a.time.total_cmp(&b.time));
        list.truncate(BEST_KEPT);
        for live in &mut rounds.list {
            if live.best_key() == Some(key) {
                live.changed = true;
            }
        }
    }
    // The helicopter over the hunted in every hunt under way, sent to its players wherever
    // they are.
    let dt = time.delta_secs();
    let stamp = crate::net::stamp(&time, &fixed);
    let hunted_states: HashMap<PlayerId, CarState> = (peers.iter())
        .filter_map(|(_, peer, _)| Some((peer.id, peer.state?)))
        .collect();
    let mut helis: HashMap<PlayerId, HeliPose> = HashMap::new();
    for live in &mut rounds.list {
        if !live.round.running() || live.round.hunt().is_none() {
            live.heli = None;
            continue;
        }
        let Some(state) = hunted_states.get(&live.round.host) else {
            continue;
        };
        let heli = live
            .heli
            .get_or_insert_with(|| Chase::new(state.position, crate::car::heading(state.rotation)));
        let ground = terrain.sample(heli.position.x, heli.position.z).0;
        heli.step(state.position, state.velocity, ground, dt);
        if now - live.heli_sent >= 1.0 / HELI_RATE - 0.004 {
            live.heli_sent = now;
            let pose = heli.pose(stamp);
            for &player in &live.round.players {
                helis.insert(player, pose);
            }
        }
    }
    // The realms of rounds over go, with their traffic.
    for live in &mut rounds.list {
        if !live.round.running() && live.realm != Realm::NORMAL {
            traffic.realms.remove(&live.realm);
            live.realm = Realm::NORMAL;
            live.course = None;
        }
    }
    // Players of a round just over join the lobby already waiting at its marker, if any
    // (keeping its host): one open lobby per event.
    let mut i = 0;
    while i < rounds.list.len() {
        let live = &rounds.list[i];
        let round = &live.round;
        let (event, ended) = (
            round.event,
            matches!(round.phase, Phase::Lobby { last: Some(_) }),
        );
        let waiting = (0..rounds.list.len()).find(|&j| {
            let other = &rounds.list[j];
            j != i
                && live.open()
                && other.open()
                && other.round.event == event
                && !other.round.running()
                && !round.running()
                && (ended
                    || (j < i && !matches!(other.round.phase, Phase::Lobby { last: Some(_) })))
        });
        if let Some(j) = waiting {
            let gone = rounds.list.remove(i);
            let lobby = &mut rounds.list[if j > i { j - 1 } else { j }];
            lobby.round.players.extend(gone.round.players);
            if ended {
                lobby.round.phase = gone.round.phase;
            }
            lobby.changed = true;
            i = 0;
            continue;
        }
        i += 1;
    }
    rounds.list.retain(|live| !live.round.players.is_empty());
    // Each lobby with its best times.
    for live in &mut rounds.list {
        if live.changed {
            live.round.best = (live.best_key())
                .and_then(|key| rounds.best.get(&key).cloned())
                .unwrap_or_default();
        }
    }

    // Each player in the realm of their round, if it runs.
    let realm_of: HashMap<PlayerId, Realm> = (rounds.list.iter())
        .filter(|live| live.round.running())
        .flat_map(|live| live.round.players.iter().map(|&p| (p, live.realm)))
        .collect();
    let busy: Vec<(PlayerId, u16)> = (rounds.list.iter())
        .flat_map(|live| live.round.players.iter().map(|&p| (p, live.round.event)))
        .collect();
    let busy_changed = busy != rounds.busy;
    if busy_changed {
        rounds.busy = busy;
    }
    rounds
        .told_busy
        .retain(|player| present.contains_key(player));
    rounds
        .started
        .retain(|player, _| present.contains_key(player));
    for (entity, mut peer, realm) in &mut peers {
        let wanted = realm_of.get(&peer.id).copied();
        let ours = wanted.is_some() || rounds.in_realms.contains(&peer.id);
        if ours && realm.copied() != wanted {
            match wanted {
                Some(realm) => commands.entity(entity).insert(realm),
                None => commands.entity(entity).remove::<Realm>(),
            };
        }
        // What R does with their car.
        let running = (rounds.list.iter()).find(|live| live.round.running() && live.has(peer.id));
        peer.shadow.rule = running.map_or(ResetRule::Road, |live| {
            live.round.reset_rule(live.course.as_ref(), peer.id)
        });
        // Their car onto its start spot when the round starts (or a time trial starts
        // again), back by the marker when it's over for them (by the marker of the lobby
        // they're in, else of the round they were in).
        let network = network.as_deref();
        let at = |event: u16| events.get(usize::from(event)).map(|event| event.at);
        let running = running.and_then(|live| match live.round.phase {
            Phase::Running { start, .. } => Some((live, start)),
            Phase::Lobby { .. } => None,
        });
        match running {
            Some((live, start)) => {
                let event = live.round.event;
                let key = (event, start, live.round.slot(peer.id));
                if rounds
                    .started
                    .get(&peer.id)
                    .is_none_or(|k| (k.0, k.1) != (event, start))
                {
                    rounds.started.insert(peer.id, key);
                    let course = live.course.as_ref();
                    let spot = (at(event)).and_then(|at| {
                        start_spot(&live.round, course, &terrain, network, at, peer.id)
                    });
                    if let Some((p, yaw)) = spot {
                        // On a race's grid, the countdown holds the car.
                        let parked = live.round.game.race().is_none();
                        peer.shadow.put(&terrain, p, yaw, parked);
                    }
                }
            }
            None => {
                if let Some((event, _, slot)) = rounds.started.remove(&peer.id) {
                    let lobby = (rounds.list.iter()).find(|live| live.has(peer.id));
                    let (event, slot) = lobby.map_or((event, slot), |live| {
                        (live.round.event, live.round.slot(peer.id))
                    });
                    if let Some(at) = at(event) {
                        let (p, yaw) = marker_spot(&terrain, network, at, slot);
                        peer.shadow.put(&terrain, p, yaw, true);
                    }
                }
            }
        }
        if peer.name.is_none() {
            continue;
        }
        if out.contains(&peer.id) {
            peer.tell(&ToClient::Round(None));
        }
        let round = (rounds.list.iter()).find(|live| live.changed && live.has(peer.id));
        if let Some(live) = round {
            peer.tell(&ToClient::Round(Some(live.round.clone())));
        }
        if busy_changed || rounds.told_busy.insert(peer.id) {
            peer.tell(&ToClient::Busy(rounds.busy.clone()));
        }
        if let Some(pose) = helis.get(&peer.id) {
            peer.tell(&ToClient::Heli(*pose));
        }
    }
    for live in &mut rounds.list {
        live.changed = false;
    }
    rounds.in_realms = realm_of.into_keys().collect();
}

/// The route a race's (or time trial's) settings pick at `event`.
fn route_of<'a>(
    events: &'a [Event],
    event: u16,
    settings: &race::Settings,
) -> Option<&'a race::Route> {
    (events.get(usize::from(event)))?
        .routes
        .get(usize::from(settings.route))
}

/// A race's standing at its start: everyone on the grid, the clock running from the green
/// light.
fn standing(players: &[PlayerId], name: &impl Fn(PlayerId) -> String, now: f64) -> race::Standing {
    race::Standing {
        racers: (players.iter())
            .map(|&player| Racer {
                player,
                name: name(player),
                gates: 0,
                at: now + COUNTDOWN,
                laps: Vec::new(),
                finished: None,
                left: false,
            })
            .collect(),
        closes: None,
    }
}

/// `player` leaves the round: in the lobby someone else hosts it if they did; a hunt under
/// way ends if they were hunted, or if they were the last hunter; a race goes on without
/// them (they're listed as having left), unless nobody's left racing.
fn leave(live: &mut Live, player: PlayerId, name: &impl Fn(PlayerId) -> String, events: &[Event]) {
    live.round.players.retain(|&p| p != player);
    live.seen.remove(&player);
    live.changed = true;
    let was_host = live.round.host == player;
    if live.round.running() {
        match &mut live.round.phase {
            Phase::Running {
                play: Play::Race(standing),
                ..
            } => {
                if let Some(racer) = standing.racers.iter_mut().find(|r| r.player == player) {
                    racer.left = true;
                }
                if (standing.racers.iter()).all(|r| r.finished.is_some() || r.left) {
                    end_race(live, events);
                }
            }
            _ => {
                if was_host {
                    end_hunt(live, Ending::HuntedLeft);
                } else if live.round.players.len() < 2 {
                    end_hunt(live, Ending::NoHunters);
                }
            }
        }
    }
    if was_host && let Some(&next) = live.round.players.first() {
        live.round.host = next;
        live.host_name = name(next);
    }
}

/// The hunt is over: back to its lobby with how it ended (its realm goes afterwards).
fn end_hunt(live: &mut Live, ending: Ending) {
    let outcome = Outcome::Hunt(hunt::Outcome {
        hunted: live.host_name.clone(),
        ending,
    });
    info!("A car hunt ended: {}", outcome.text());
    live.round.phase = Phase::Lobby {
        last: Some(outcome),
    };
    live.changed = true;
}

/// The race (or time trial) is over: back to its lobby with the results.
fn end_race(live: &mut Live, events: &[Event]) {
    let (Some(standing), Some(settings)) = (live.round.standing(), live.round.game.race()) else {
        return;
    };
    let route = (events.get(usize::from(live.round.event)))
        .and_then(|event| event.routes.get(usize::from(settings.route)))
        .map(|route| route.name.clone())
        .unwrap_or_default();
    let results = Outcome::Race(race::Results::of(standing, route));
    info!(
        "A {} ended: {}",
        live.round.game.title().to_lowercase(),
        results.text()
    );
    live.round.phase = Phase::Lobby {
        last: Some(results),
    };
    live.changed = true;
}
