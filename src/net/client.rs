//! A player's side: say hello, send the car being driven (its looks when they change, its
//! state `SEND_RATE` times a second), and show the cars the server sends, the traffic's
//! and the other players' alike, as `RemoteCar`s: entities with their `Looks` and
//! `Motion` (so they're drawn and heard like any car) and the name they show, if any
//! (`Tag`), where they are by now: carried on from their latest state (dead reckoning),
//! turning as they turned and speeding up or slowing down as they did between their last
//! two states (`predict`). What's drawn closes on that smoothly (`Drawn`), so a new state
//! that disagrees doesn't make a car jump. A car only pushes itself off a remote car; the
//! other player's game (or the server, for traffic) does the same on their side. On a
//! server there's no traffic of our own: the server's comes as remote cars.
//!
//! Our own car runs ahead of the server (our inputs reach it just in time for each step,
//! see `predict`), so the remote cars are carried on to when the server drives the step
//! our car is at, not just to the latest the server's word could have come by (a round
//! trip earlier): that's where our car meets them on the server (`Connection::step_time`).
//! A car rear-ending a steadily driving one so meets it here where it does there.
//! The other players the server puts on our map are kept apart from all that
//! (`Connection::on_map`), and so is the chat (`Connection::chat`).

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use bevy::prelude::*;

use super::link::Link;
use super::predict::Prediction;
use super::protocol::{
    CarId, CarState, MapPlayer, PROTOCOL, PlayerId, SERVER, ToClient, ToServer, clean_chat,
};
use super::wire::{decode_with, encode};
use super::{ClockOffset, SEND_RATE};
use crate::car::{ActiveCar, Car, DriverInput, ServerDriven, Spectating, StepInput, heading};
use crate::catalogue::{Current, Shared};
use crate::events::Event;
use crate::events::round::{Action, Round};
use crate::heli::HeliPose;
use crate::looks::{Ghost, Looks, Motion};
use crate::physics::collision::Obb;
use crate::physics::collision::{self, OBSTACLE};
use crate::physics::engine::maps::EngineMaps;
use crate::physics::rigid_body::RigidBody;
use crate::shops::Shop;
use crate::sim;
use crate::spec::CarSpec;
use crate::terrain::{Spawn, Terrain};

/// How long a car is carried on past its latest state; then it's held there (s). More
/// than the time between states far away (3 a second), and a lost one, plus a round trip
/// (cars are carried on to when the server drives our latest step).
const PREDICT: f64 = 0.8;
/// How many steps from the latest the server told us of when it drove it we work out
/// when it drives others (two seconds; it tells ten times a second).
const TIMELINE_REACH: i32 = 240;
/// How far ahead of the latest the server's word could have come by our steps run, by the
/// server's clock (s): a round trip and a margin, give or take the clocks' jitter.
const AHEAD: std::ops::RangeInclusive<f64> = -0.1..=1.0;
/// How quickly what's drawn of a car closes on where it's predicted to be (s, the time
/// constant).
const SMOOTHING: f32 = 0.1;
/// Further than this from where it's predicted to be, a car is drawn there at once (m).
const SNAP: f32 = 5.0;
/// The step a car is carried on in (s).
const STEP: f32 = 1.0 / 120.0;
/// The most acceleration believed from two states (m/s²): more is a crash, which doesn't
/// go on.
const MOST_ACCELERATION: f32 = 15.0;
/// A car not heard of for this long is gone (should the server's word be lost) (s).
const GONE: f64 = 2.0;
/// What we've been told of a car (its looks and name) is kept this long after it was
/// last heard of (s): the server tells it again when the car comes back into range.
const FORGET: f64 = 30.0;
/// States kept per car: the last two give the acceleration, a scripted car is shown
/// between them (a third of a second near, at 30 a second).
const KEPT: usize = 12;
/// How long before the latest the server's word could have come a scripted car is shown
/// (`CarState::scripted`): three states near, so it's always between two.
const SCRIPTED_DELAY: f64 = 0.1;
/// How far past its latest state a scripted car is carried on, should none come (s).
const SCRIPTED_AHEAD: f64 = 0.1;
/// A player on the map is carried on along their way for this long past where they were
/// last sent (once a second), and gone when not sent for this long (s).
const MAP_PREDICT: f64 = 1.5;
const MAP_GONE: f64 = 3.0;
/// Chat lines kept.
const CHAT_KEPT: usize = 50;
/// How often what `Report` gathered is logged (s).
const REPORT: f64 = 10.0;
/// A state this much past the quickest counts as late (s).
const LATE: f64 = 0.03;
/// Each message sends the inputs of the steps since the last one and this many before
/// them again (a few frames' worth, so a lost message or two loses none), at most
/// `MOST_INPUTS` (a slow frame runs many steps; a datagram holds about 80).
const INPUTS_AGAIN: u32 = 12;
const MOST_INPUTS: usize = 64;
/// Our car's whole state goes to the server after every step whose number is a multiple
/// of this (once a second), for checking against the server's own simulation of it.
pub const CHECK_STEPS: u32 = 120;
/// How early (in steps, at the least) we keep our inputs coming to the server: our clock
/// runs a little faster or slower until they do (see `pace`).
const EARLY: f32 = 2.0;
/// How much faster or slower our clock runs per step our inputs come too late or too
/// early, and at most.
const PACE_GAIN: f64 = 0.004;
const MOST_PACE: f64 = 0.03;
/// Without word from the server for this long, our clock runs at its own pace (s).
const PACE_GONE: f64 = 1.0;
/// The most game time a frame may move on by (s). Bevy's own limit (0.25 s) let a game
/// at a few frames a second fall behind the server's clock for good; a step costs a
/// fraction of a millisecond, so catching up a second is cheap.
const MOST_FRAME: f64 = 1.0;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                receive,
                take_your_car,
                super::predict::reconcile,
                pace,
                dress_cars,
                send,
                (move_cars, remove_cars),
                spectate,
            )
                .chain()
                .after(crate::looks::show_cars)
                .run_if(resource_exists::<Connection>),
        )
        .init_resource::<StepInput>()
        .init_resource::<Prediction>()
        .init_resource::<WantsGhost>()
        .init_resource::<GhostPlayers>()
        .init_resource::<Touching>()
        .add_systems(Update, haunt.before(send))
        .add_systems(PreUpdate, forget_cars)
        .add_systems(
            FixedUpdate,
            (
                (record_input, super::predict::notice_placing)
                    .after(crate::car::take_input)
                    .before(crate::car::drive),
                (place_bodies, push_off_remote_cars)
                    .chain()
                    .after(crate::sim::simulate),
            )
                .run_if(resource_exists::<Connection>),
        )
        .add_systems(
            FixedLast,
            (super::predict::note_step, take_check).run_if(resource_exists::<Connection>),
        );
    }
}

/// The most changes of our car kept waiting for the server's answer.
const MOST_UNANSWERED: usize = 16;

/// Builds the car the server said ours is now: in place of the active car (where it is,
/// keeping the driver's settings), or the first one, by the spawn. The server's state of
/// it follows (`Yours`).
fn take_your_car(
    mut commands: Commands,
    mut connection: ResMut<Connection>,
    terrain: Res<Terrain>,
    active: Res<ActiveCar>,
    mut cars: Query<(Entity, &mut Car)>,
) {
    let Some(spec) = connection.your_car.take() else {
        return;
    };
    let ours = cars.iter_mut().find(|(_, car)| car.index == active.0);
    if ours.is_some_and(|(_, car)| CarSpec::of(&car.setup) == spec) {
        return;
    }
    let hardware = spec.parts.hardware();
    let print = crate::physics::engine::maps::fingerprint(&hardware);
    let maps = match crate::workshop::at_hand(&hardware) {
        Some(maps) => maps,
        None if connection.lacks_maps(print) => {
            // Not the server's to give: baked here (a car built in a shop here before
            // would have them kept already).
            let maps = EngineMaps::bake(&hardware);
            crate::workshop::keep(&hardware, &maps);
            maps
        }
        None => {
            // From the server; till then the car waits.
            connection.ask_maps(print);
            connection.your_car = Some(spec);
            return;
        }
    };
    let setup = spec.setup_with(maps);
    match cars.iter_mut().find(|(_, car)| car.index == active.0) {
        Some((_, mut car)) if car.setup.build == setup.build => {
            let rebuilt = car.rebuilt(setup);
            *car = rebuilt;
        }
        Some((entity, car)) => {
            // The body and wheels are the old entity's children: a new car takes its place.
            let (at, yaw) = (car.body.position, heading(car.body.rotation));
            let mut new = Car::spawn_at(car.index, setup, &terrain, at.x, at.z, yaw);
            new.keep_settings(&car);
            new.drivetrain.neutral();
            commands.entity(entity).despawn();
            commands.spawn((Transform::from_translation(new.body.position), new));
        }
        None => {
            let (p, yaw) = terrain.road_spawns(&connection.spawn, 1, 0.0)[0];
            let mut car = Car::spawn_at(active.0, setup, &terrain, p.x, p.y, yaw);
            car.drivetrain.neutral();
            commands.spawn((Transform::from_translation(car.body.position), car));
        }
    }
}

/// Where the connection to the server stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// Said hello, waiting for an answer.
    Joining,
    Joined {
        you: PlayerId,
    },
    Refused(String),
    /// Couldn't join, or the server went away: why.
    Lost(String),
}

/// A line in the chat.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatLine {
    /// Who wrote it (as they were called then), or nobody for the server's own lines.
    pub from: Option<String>,
    pub text: String,
    /// When it came (on `Time<Real>`, s).
    pub at: f64,
}

/// The connection to a server, while playing on one.
#[derive(Resource)]
pub struct Connection {
    link: Box<dyn Link>,
    pub status: Status,
    /// Everyone on the server but us, by number.
    pub players: BTreeMap<PlayerId, String>,
    /// Everyone's round trip to the server (ms), ours too, as last sent.
    pub pings: HashMap<PlayerId, u16>,
    /// Our name.
    pub name: String,
    /// Whether the server has chat on, and the latest lines, oldest first.
    pub chat_on: bool,
    pub chat: VecDeque<ChatLine>,
    /// The shops on the map, as the server told.
    pub shops: Vec<Shop>,
    /// Where players start, as the server told.
    pub spawn: Spawn,
    /// Our car's number, as the server told.
    pub car: Option<CarId>,
    /// The events on the map, likewise.
    pub events: Vec<Event>,
    /// The event round we're in (its lobby or under way), as the server last told.
    pub round: Option<Round>,
    /// Who's in which event's round (its place in `events`).
    pub busy: HashMap<PlayerId, u16>,
    /// The car hunt's helicopter, as the server last told, and when that came (s, on
    /// `Time<Real>`).
    pub hunt_heli: Option<(HeliPose, f64)>,
    /// The server's clock against ours (every state comes on it).
    clock: ClockOffset,
    /// Our latest step the server's word came for, and when the server drove it (its
    /// clock, s).
    timeline: Option<(u32, f64)>,
    /// What we've been told of each car (a state may overtake it, or come later).
    known: HashMap<CarId, Known>,
    /// The other players the server shows on our map, and when each was heard of (s).
    map: HashMap<PlayerId, (MapPlayer, f64)>,
    /// When our car's state was last sent (s).
    sent: f64,
    /// What our car was built of when last sent (or as the server built it).
    sent_spec: Option<CarSpec>,
    /// What our car was built of in messages the server hasn't answered yet (it answers
    /// each with the car it has), the latest last.
    unanswered: VecDeque<CarSpec>,
    /// What cars and parts there are, as the server told us (none before it has).
    pub catalogue: Option<Shared>,
    /// The car the server said ours is now, until it's built (`take_your_car`).
    pub your_car: Option<CarSpec>,
    /// Engine maps asked the server for (by fingerprint), and those it hasn't got.
    maps_asked: HashSet<u64>,
    maps_missing: HashSet<u64>,
    /// The driver's latest inputs by step, the latest last, and the last step sent.
    inputs: VecDeque<(u32, DriverInput)>,
    inputs_sent: Option<u32>,
    /// Our car's whole state after the latest step to check, to send.
    check: Option<(u32, Vec<u8>)>,
    /// How early our inputs came to the server lately (steps), and when it said (s).
    early: Option<(i16, f64)>,
    /// How the cars' states came lately, logged every `REPORT` seconds.
    report: Report,
}

/// How far new states landed from where the ones before predicted the cars, and how late
/// they came (for telling whether cars jump because of the server or the way here).
#[derive(Default)]
struct Report {
    since: f64,
    states: usize,
    /// The farthest a new state of a car near us (sent 10 times a second or more) was from
    /// its prediction (m), and whose.
    jump: (f32, CarId),
    /// The latest a state came, past the quickest (s).
    late: f64,
    /// States more than `LATE` past the quickest.
    late_count: usize,
}

impl Connection {
    /// Asks the server for the maps of the engine whose hardware has `fingerprint` (once);
    /// they're kept with the maps baked here when they come (`workshop::at_hand`).
    pub fn ask_maps(&mut self, fingerprint: u64) {
        if self.maps_asked.insert(fingerprint) {
            self.say(&ToServer::Maps { fingerprint });
        }
    }

    /// Whether the server said it hasn't got the maps of the engine with `fingerprint`.
    pub fn lacks_maps(&self, fingerprint: u64) -> bool {
        self.maps_missing.contains(&fingerprint)
    }

    /// Joins a server over `link` as `name` (a guest, or playing alone), playing on
    /// `terrain`'s map.
    pub fn new(link: impl Link, name: &str, terrain: &Terrain) -> Self {
        Self::login(link, name, "", terrain)
    }

    /// Joins a server over `link` as `name` with `password` (an account's; empty: a guest),
    /// playing on `terrain`'s map.
    pub fn login(link: impl Link, name: &str, password: &str, terrain: &Terrain) -> Self {
        let mut connection = Self {
            link: Box::new(link),
            status: Status::Joining,
            players: BTreeMap::new(),
            pings: HashMap::new(),
            name: name.to_string(),
            chat_on: false,
            shops: Vec::new(),
            spawn: Spawn::DEFAULT,
            car: None,
            events: Vec::new(),
            round: None,
            busy: HashMap::new(),
            hunt_heli: None,
            chat: VecDeque::new(),
            clock: ClockOffset::default(),
            timeline: None,
            known: HashMap::new(),
            map: HashMap::new(),
            sent: f64::NEG_INFINITY,
            sent_spec: None,
            unanswered: VecDeque::new(),
            catalogue: None,
            your_car: None,
            maps_asked: HashSet::new(),
            maps_missing: HashSet::new(),
            inputs: VecDeque::new(),
            inputs_sent: None,
            check: None,
            early: None,
            report: Report::default(),
        };
        connection.say(&ToServer::Hello {
            protocol: PROTOCOL,
            map: terrain.fingerprint,
            name: name.to_string(),
            password: password.to_string(),
        });
        connection
    }

    pub fn say(&mut self, message: &ToServer) {
        self.link.send(message.reliable(), encode(message));
    }

    /// The other players the server puts on our map, where they are by `now` (on
    /// `Time<Real>`): carried on at their speed for a little while.
    pub fn on_map(&self, now: f64) -> impl Iterator<Item = MapPlayer> + '_ {
        self.map.values().map(move |&(player, heard)| {
            let ahead = (now - heard).clamp(0.0, MAP_PREDICT) as f32;
            MapPlayer {
                position: player.position + player.velocity * ahead,
                ..player
            }
        })
    }

    /// Writes `text` in the chat (as one line, cut to `CHAT_LENGTH`); it shows once it's
    /// back from the server.
    pub fn write(&mut self, text: &str) {
        if let Some(text) = clean_chat(text).filter(|_| self.chat_on) {
            self.say(&ToServer::Chat(text));
        }
    }

    /// Whether we're in an event's round that's under way (in a world of its own).
    pub fn in_running_round(&self) -> bool {
        self.round.as_ref().is_some_and(Round::running)
    }

    /// Asks something of the events (into a lobby, out, the hunted's choices).
    pub fn act(&mut self, action: Action) {
        self.say(&ToServer::Event(action));
    }

    /// When the server drives (drove) our step `step`, by its clock (s): from the latest
    /// step it told us of (none without word lately).
    pub fn step_time(&self, step: u32) -> Option<f64> {
        let (told, time) = self.timeline?;
        let since = step.wrapping_sub(told) as i32;
        (since.abs() <= TIMELINE_REACH).then(|| time + f64::from(since) * f64::from(sim::STEP))
    }

    /// Our latest step (the one being simulated, in a step), once they run (a game that
    /// doesn't simulate its car, a test's, keeps the one input it began with).
    pub(super) fn last_step(&self) -> Option<u32> {
        let ran = self.inputs.len() > 1;
        self.inputs.back().filter(|_| ran).map(|&(step, _)| step)
    }

    /// When (the server's clock) a remote car is met and shown as of in our step `step`,
    /// `heard` being the latest the server's word could have come by now: when the server
    /// drives that step of ours. Not so a scripted car (`CarState::scripted`, nothing we do
    /// changes where it goes): that's shown as it went, `SCRIPTED_DELAY` before what's
    /// heard. Our own car shown as the server drives it, and any while our steps don't run
    /// ahead of what's heard as they should (a game not simulating its car), as heard (as
    /// long before as the step is).
    fn meet_time(&self, step: u32, heard: f64, car: CarId, latest: &CarState) -> f64 {
        let back = self.last_step().map_or(0.0, |last| {
            f64::from(last.wrapping_sub(step) as i32) * f64::from(sim::STEP)
        });
        let heard = heard - back;
        if latest.scripted {
            return heard - SCRIPTED_DELAY;
        }
        match self.step_time(step) {
            Some(time) if AHEAD.contains(&(time - heard)) && Some(car) != self.car => time,
            _ => heard,
        }
    }

    /// The time on the server's clock that it is `now` (on `Time<Real>`), as near as the
    /// states that came tell (none before the first).
    pub fn server_time(&self, now: f64) -> Option<f64> {
        self.clock.their(now)
    }

    /// Leaves the server.
    pub fn close(&mut self) {
        self.link.close();
    }
}

/// A car's looks and name as we were told, and when it was last heard of (s).
#[derive(Default)]
struct Known {
    looks: Option<Looks>,
    tag: Option<String>,
    heard: f64,
}

/// A car the server sends: another player's or a traffic car (nothing tells which).
#[derive(Component)]
pub struct RemoteCar {
    pub id: CarId,
    /// The latest states, oldest first.
    states: VecDeque<CarState>,
    /// When it was last heard of (our clock, s).
    heard: f64,
    /// Where it is by now, as a body to run into (moving, but never pushed), and as of
    /// when (the server's clock).
    pub body: Option<RigidBody>,
    met: f64,
    /// How our car pushed it, which the server's word doesn't show yet.
    kicks: Vec<Kick>,
    drawn: Drawn,
}

/// A push our car gave a remote car (as our game worked it out) at `time` on the server's
/// clock: it's taken to move so until a state from then on comes, which shows how it
/// really did (the server's traffic, or the other player's car on the server).
#[derive(Clone, Copy, Debug)]
struct Kick {
    time: f64,
    velocity: Vec3,
    angular_velocity: Vec3,
}

impl RemoteCar {
    /// Its latest state.
    pub fn latest(&self) -> Option<&CarState> {
        self.states.back()
    }

    /// As of when (the server's clock) it was last drawn.
    pub fn shown_at(&self) -> f64 {
        self.met
    }

    /// Where its rescue helicopter is at `time` (the server's clock), as `at` has the car:
    /// between the poses around it if the car is scripted (each pose's time is its
    /// state's), else carried on from the latest.
    pub fn heli_at(&self, time: f64) -> Option<HeliPose> {
        let latest = self.states.back()?.heli?;
        let poses = self.states.iter().filter_map(|state| state.heli);
        let after = poses.clone().position(|pose| pose.time > time);
        let pose = match after {
            Some(0) => poses.clone().next()?,
            Some(after) if self.states.back()?.scripted => {
                let (a, b) = (poses.clone().nth(after - 1)?, poses.clone().nth(after)?);
                let u = ((time - a.time) / (b.time - a.time)) as f32;
                let h = (b.time - a.time) as f32;
                let (position, velocity) =
                    hermite(a.position, a.velocity, b.position, b.velocity, h, u);
                HeliPose {
                    time,
                    position,
                    velocity,
                    rotation: a.rotation.slerp(b.rotation, u).normalize(),
                    cable: a.cable + (b.cable - a.cable) * u,
                    hooked: a.hooked,
                }
            }
            _ => {
                let limit = if self.states.back()?.scripted {
                    SCRIPTED_AHEAD
                } else {
                    PREDICT
                };
                let ahead = (time - latest.time).clamp(0.0, limit) as f32;
                HeliPose {
                    time,
                    position: latest.position + latest.velocity * ahead,
                    ..latest
                }
            }
        };
        Some(pose)
    }

    /// Where it is (to run into) as of our step `step`, by `connection`'s word, `heard`
    /// being the latest the server's word could have come by now (its clock).
    fn body_at(
        &self,
        connection: &Connection,
        looks: &Looks,
        step: u32,
        heard: f64,
    ) -> Option<RigidBody> {
        let time = connection.meet_time(step, heard, self.id, self.states.back()?);
        Some(body_of(&self.at(time).0, looks))
    }

    /// Where it is at `time` (the server's clock), the state that's from and how long after
    /// it: carried on from its latest state (a scripted car between its states, as long
    /// as there are some after `time`), and pushed as our car pushed it since (`Kick`).
    fn at(&self, time: f64) -> (Kinematics, usize, f32) {
        let last = self.states.len() - 1;
        let (mut at, from, ahead) = match self.states[last] {
            latest if latest.scripted && time < latest.time => interpolate(&self.states, time),
            latest if latest.scripted => {
                let time = time.min(latest.time + SCRIPTED_AHEAD);
                let (at, ahead) = predict(&self.states, time);
                (at, last, ahead)
            }
            _ => {
                let (at, ahead) = predict(&self.states, time);
                (at, last, ahead)
            }
        };
        let latest = self.states[last].time;
        for kick in &self.kicks {
            if kick.time > latest && kick.time <= time {
                at.velocity += kick.velocity;
                at.angular_velocity += kick.angular_velocity;
                at.position += kick.velocity * (time - kick.time) as f32;
            }
        }
        (at, from, ahead)
    }
}

/// A body moving as `at`.
fn body_of(at: &Kinematics, looks: &Looks) -> RigidBody {
    let mut body = RigidBody::new(at.position, at.rotation, looks.mass, looks.inertia);
    body.velocity = at.velocity;
    body.angular_velocity = at.angular_velocity;
    body
}

/// The name a remote car shows over it.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Tag(pub String);

fn now(time: &Time<Real>) -> f64 {
    time.elapsed_secs_f64()
}

fn receive(
    mut commands: Commands,
    mut connection: ResMut<Connection>,
    mut prediction: ResMut<Prediction>,
    time: Res<Time<Real>>,
    mut cars: Query<(Entity, &mut RemoteCar, Option<&Looks>, Option<&Tag>)>,
) {
    let now = now(&time);
    let connection = &mut *connection;
    // Cars new to us, with their states, spawned once everything's read (several states
    // of one can come in a frame).
    let mut fresh: Vec<(CarId, VecDeque<CarState>)> = Vec::new();
    while let Some(bytes) = connection.link.receive() {
        let Some(message) = decode_with::<ToClient>(&bytes, connection.catalogue.as_ref()) else {
            warn!("The server sent something unreadable");
            continue;
        };
        match message {
            ToClient::Welcome {
                you,
                car,
                chat,
                spawn,
            } => {
                info!("Joined the server as player {you}");
                connection.status = Status::Joined { you };
                connection.car = Some(car);
                connection.chat_on = chat;
                connection.spawn = spawn;
            }
            ToClient::Refused(why) => {
                warn!("The server refused: {why}");
                connection.status = Status::Refused(why);
            }
            ToClient::Joined { id, name } => {
                connection.players.insert(id, name);
            }
            ToClient::Left { id } => {
                connection.players.remove(&id);
                connection.pings.remove(&id);
                connection.map.remove(&id);
            }
            ToClient::Looks { car, looks } => {
                for (entity, remote, shown, _) in &cars {
                    if remote.id == car && shown != Some(&*looks) {
                        commands.entity(entity).insert((*looks).clone());
                    }
                }
                let known = connection.known.entry(car).or_default();
                known.looks = Some(*looks);
                known.heard = now;
            }
            ToClient::Tag { car, name } => {
                for (entity, remote, _, shown) in &cars {
                    if remote.id != car || shown.map(|tag| &tag.0) == name.as_ref() {
                        continue;
                    }
                    match &name {
                        Some(name) => commands.entity(entity).insert(Tag(name.clone())),
                        None => commands.entity(entity).remove::<Tag>(),
                    };
                }
                let known = connection.known.entry(car).or_default();
                known.tag = name;
                known.heard = now;
            }
            ToClient::Cars(states) => {
                for (id, state) in states {
                    connection.clock.heard(state.time, now);
                    let report = &mut connection.report;
                    report.states += 1;
                    let late = connection.clock.ours(state.time).map_or(0.0, |t| now - t);
                    report.late = report.late.max(late);
                    report.late_count += usize::from(late > LATE);
                    if let Some(known) = connection.known.get_mut(&id) {
                        known.heard = now;
                    }
                    let found = cars.iter_mut().find(|(_, car, ..)| car.id == id);
                    if let Some((_, mut car, ..)) = found {
                        car.heard = now;
                        if car.states.back().is_none_or(|last| state.time > last.time) {
                            // Only cars sent often (near): far ones go further between
                            // states, but nobody sees them jump.
                            let last = car.states.back().unwrap().time;
                            if state.time - last <= 0.11 {
                                let (predicted, _) = predict(&car.states, state.time);
                                let jump = predicted.position.distance(state.position);
                                if jump > report.jump.0 {
                                    report.jump = (jump, id);
                                }
                            }
                            car.states.push_back(state);
                            if car.states.len() > KEPT {
                                car.states.pop_front();
                            }
                        }
                        continue;
                    }
                    // A car new to us (or back in range).
                    match fresh.iter_mut().find(|(fresh, _)| *fresh == id) {
                        Some((_, states)) => {
                            if states.back().is_none_or(|last| state.time > last.time) {
                                states.push_back(state);
                            }
                        }
                        None => fresh.push((id, VecDeque::from([state]))),
                    }
                }
            }
            ToClient::Gone { car } => {
                fresh.retain(|(id, _)| *id != car);
                for (entity, remote, ..) in &cars {
                    if remote.id == car {
                        commands.entity(entity).despawn();
                    }
                }
                // A state still on its way mustn't bring it back with its looks.
                connection.known.remove(&car);
            }
            ToClient::Map(players) => {
                for player in players {
                    connection.map.insert(player.id, (player, now));
                }
            }
            ToClient::Catalogue(catalogue) => {
                commands.insert_resource(Current(catalogue.clone()));
                connection.catalogue = Some(catalogue);
            }
            ToClient::YourCar(spec) => {
                match connection.unanswered.iter().position(|sent| *sent == *spec) {
                    // The server took a change of ours: nothing to do.
                    Some(answered) => drop(connection.unanswered.drain(..=answered)),
                    // Its own (or ours refused): built as told, nothing to tell it.
                    None => {
                        connection.unanswered.clear();
                        connection.sent_spec = Some((*spec).clone());
                        connection.your_car = Some(*spec);
                    }
                }
            }
            ToClient::Maps { fingerprint, text } => {
                let text = text.and_then(|text| crate::physics::engine::store::unpack(&text.0));
                // Kept with the maps baked here, if they're what was asked for.
                match text.filter(|text| EngineMaps::from_text(text, fingerprint).is_some()) {
                    Some(text) => crate::workshop::keep_text(fingerprint, &text),
                    None => {
                        connection.maps_missing.insert(fingerprint);
                    }
                }
            }
            ToClient::Shops(shops) => connection.shops = shops,
            ToClient::Events(events) => connection.events = events,
            ToClient::Round(round) => connection.round = round,
            ToClient::Busy(busy) => connection.busy = busy.into_iter().collect(),
            ToClient::Heli(pose) => connection.hunt_heli = Some((pose, now)),
            ToClient::Pace { early } => connection.early = Some((early, now)),
            ToClient::Yours(yours) => {
                let newer = (connection.timeline)
                    .is_none_or(|(step, _)| (yours.step.wrapping_sub(step) as i32) >= 0);
                if newer {
                    connection.timeline = Some((yours.step, yours.time));
                }
                prediction.yours = Some(yours);
            }
            ToClient::Pings(pings) => {
                connection.pings = pings.into_iter().collect();
            }
            ToClient::Chat { from, text } => {
                let from = match (from, &connection.status) {
                    (SERVER, _) => None,
                    (id, Status::Joined { you }) if id == *you => Some(connection.name.clone()),
                    (id, _) => Some(connection.players.get(&id).cloned().unwrap_or_default()),
                };
                connection.chat.push_back(ChatLine {
                    from,
                    text,
                    at: now,
                });
                if connection.chat.len() > CHAT_KEPT {
                    connection.chat.pop_front();
                }
            }
        }
    }
    let report = &mut connection.report;
    if now - report.since >= REPORT {
        if report.states > 0 {
            info!(
                "Last {REPORT} s: {} car states, of a car near the farthest {:.2} m from its \
                 prediction (car {}); {} came over {:.0} ms later than the quickest, the latest {:.0} ms",
                report.states,
                report.jump.0,
                report.jump.1,
                report.late_count,
                LATE * 1000.0,
                report.late * 1000.0
            );
        }
        *report = Report {
            since: now,
            ..Report::default()
        };
    }
    (connection.map).retain(|_, &mut (_, heard)| now - heard < MAP_GONE);
    (connection.known).retain(|_, known| now - known.heard < FORGET);
    let ended = matches!(connection.status, Status::Lost(_) | Status::Refused(_));
    if !connection.link.is_open() && !ended {
        let why = (connection.link.problem())
            .unwrap_or_else(|| "The connection to the server was lost".to_string());
        warn!("{why}");
        connection.status = Status::Lost(why);
        for (entity, ..) in &cars {
            commands.entity(entity).despawn();
        }
    }
    let joined = matches!(connection.status, Status::Joined { .. });
    for (id, states) in fresh.into_iter().filter(|_| joined) {
        let state = *states.back().expect("a state");
        let mut entity = commands.spawn((
            RemoteCar {
                id,
                states,
                heard: now,
                body: None,
                met: 0.0,
                kicks: Vec::new(),
                drawn: Drawn::default(),
            },
            Transform::from_translation(state.position).with_rotation(state.rotation),
            state.motion,
        ));
        if let Some(known) = connection.known.get(&id) {
            if let Some(looks) = &known.looks {
                entity.insert(looks.clone());
            }
            if let Some(name) = &known.tag {
                entity.insert(Tag(name.clone()));
            }
        }
    }
}

/// Runs our simulation's clock a little faster when our inputs come to the server late,
/// slower when they come too early, so they come just in time (the server drives our car
/// on its own clock). Each step simulates `sim::STEP` all the same: only how many there
/// are a second changes, by a few percent at most. A slow frame catches up to a second.
fn pace(
    connection: Res<Connection>,
    time: Res<Time<Real>>,
    mut fixed: ResMut<Time<Fixed>>,
    mut virtual_time: ResMut<Time<Virtual>>,
) {
    let most = std::time::Duration::from_secs_f64(MOST_FRAME);
    if virtual_time.max_delta() != most {
        virtual_time.set_max_delta(most);
    }
    let now = now(&time);
    let pace = match connection.early {
        Some((early, at)) if now - at < PACE_GONE => {
            (PACE_GAIN * f64::from(f32::from(early) - EARLY)).clamp(-MOST_PACE, MOST_PACE)
        }
        _ => 0.0,
    };
    let step = std::time::Duration::from_secs_f64((1.0 + pace) / crate::sim::CHASSIS_HZ);
    if fixed.timestep() != step {
        fixed.set_timestep(step);
    }
}

/// Gives a remote car its looks and name once they're here: a state can overtake them
/// (it comes as a datagram, they come over the stream) and spawn the car first.
fn dress_cars(
    mut commands: Commands,
    connection: Res<Connection>,
    cars: Query<(Entity, &RemoteCar, Has<Looks>, Has<Tag>)>,
) {
    for (entity, car, has_looks, has_tag) in &cars {
        let Some(known) = connection.known.get(&car.id) else {
            continue;
        };
        if !has_looks && let Some(looks) = &known.looks {
            commands.entity(entity).insert(looks.clone());
        }
        if !has_tag && let Some(name) = &known.tag {
            commands.entity(entity).insert(Tag(name.clone()));
        }
    }
}

/// Keeps the driver's input in each simulation step, to send.
fn record_input(mut connection: ResMut<Connection>, input: Res<StepInput>) {
    let inputs = &mut connection.inputs;
    // (A game that doesn't simulate its car, a test's, has no steps.)
    if inputs.back().is_some_and(|&(step, _)| step == input.step) {
        return;
    }
    inputs.push_back((input.step, input.input));
    while inputs.len() > MOST_INPUTS {
        inputs.pop_front();
    }
}

/// Our car's whole state after a step to check, once all of the step is done (the
/// pushes off other cars, the rescue helicopter).
fn take_check(
    mut connection: ResMut<Connection>,
    (active, step): (Res<ActiveCar>, Res<StepInput>),
    cars: Query<&Car, Without<ServerDriven>>,
) {
    if step.step % CHECK_STEPS != 0 {
        return;
    }
    if let Some(car) = cars.iter().find(|car| car.index == active.0) {
        connection.check = Some((step.step, crate::snapshot::save(car)));
    }
}

/// Our car's looks when they change, and its state every so often; what it's built of
/// when that changes, the driver's input in every step, and its whole state now and then
/// (for the server to drive it too).
fn send(
    mut connection: ResMut<Connection>,
    time: Res<Time<Real>>,
    fixed: Res<Time<Fixed>>,
    active: Res<ActiveCar>,
    mut prediction: ResMut<Prediction>,
    cars: Query<(&Car, &Motion, Has<Ghost>)>,
) {
    let now = now(&time);
    if !matches!(connection.status, Status::Joined { .. }) {
        return;
    }
    let Some((car, motion, ghost)) = cars.iter().find(|(car, ..)| car.index == active.0) else {
        return;
    };
    let spec = CarSpec::of(&car.setup);
    if connection.sent_spec.as_ref() != Some(&spec) {
        connection.say(&ToServer::Car(Box::new(spec.clone())));
        if connection.unanswered.len() >= MOST_UNANSWERED {
            connection.unanswered.pop_front();
        }
        connection.unanswered.push_back(spec.clone());
        connection.sent_spec = Some(spec);
    }
    if let Some((step, state)) = prediction.place.take() {
        connection.say(&ToServer::Place { step, state });
    }
    if let Some(&(last, _)) = connection.inputs.back()
        && connection.inputs_sent != Some(last)
    {
        let new = connection
            .inputs_sent
            .map_or(u32::MAX, |sent| last.wrapping_sub(sent));
        let count = (new.saturating_add(INPUTS_AGAIN) as usize).min(connection.inputs.len());
        let inputs = connection.inputs.iter().rev().take(count).rev();
        let inputs = inputs.map(|&(_, input)| input).collect();
        connection.inputs_sent = Some(last);
        connection.say(&ToServer::Inputs { last, inputs });
    }
    if let Some((step, state)) = connection.check.take() {
        connection.say(&ToServer::Check { step, state });
    }
    if now - connection.sent >= 1.0 / SEND_RATE {
        connection.sent = now;
        let stamp = super::stamp(&time, &fixed);
        connection.say(&ToServer::State(CarState {
            time: stamp,
            position: car.body.position,
            rotation: car.body.rotation,
            velocity: car.body.velocity,
            angular_velocity: car.body.angular_velocity,
            motion: *motion,
            ghost,
            scripted: false,
            heli: None,
        }));
    }
}

/// While the server has our car (`Spectating`): it isn't driven or simulated here
/// (`ServerDriven`), and it's hidden, following our car as the server sends it to everyone,
/// which is drawn (and heard, helicopter and all) like any other car instead. Given back,
/// ours is shown again and the server's copy hidden until it's gone.
#[allow(clippy::type_complexity)]
fn spectate(
    mut commands: Commands,
    connection: Res<Connection>,
    spectating: Option<Res<Spectating>>,
    active: Res<ActiveCar>,
    mut cars: Query<(Entity, &mut Car, &mut Transform, Has<ServerDriven>)>,
    remote: Query<(Entity, &RemoteCar, &Transform, Option<&Visibility>), Without<Car>>,
) {
    let on = spectating.is_some_and(|s| s.0.is_some());
    let mut seen = None;
    for (entity, car, transform, visibility) in &remote {
        if Some(car.id) != connection.car {
            continue;
        }
        let wanted = if on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if visibility != Some(&wanted) {
            commands.entity(entity).insert(wanted);
        }
        let velocity = car.latest().map_or(Vec3::ZERO, |state| state.velocity);
        seen = Some((*transform, velocity));
    }
    for (entity, mut car, mut transform, driven) in &mut cars {
        if car.index != active.0 {
            continue;
        }
        if on && !driven {
            commands
                .entity(entity)
                .insert((ServerDriven, Visibility::Hidden));
        } else if !on && driven {
            commands
                .entity(entity)
                .remove::<ServerDriven>()
                .insert(Visibility::Inherited);
        }
        if on && let Some((at, velocity)) = seen {
            car.body.position = at.translation;
            car.body.rotation = at.rotation;
            car.body.velocity = velocity;
            car.body.angular_velocity = Vec3::ZERO;
            *transform = at;
        }
    }
}

/// What `move_cars` reads and moves of a remote car.
type RemoteParts = (
    Entity,
    &'static mut RemoteCar,
    &'static mut Transform,
    &'static mut Motion,
    Option<&'static Looks>,
    Has<Ghost>,
    Has<Tag>,
);

/// Shows each remote car where it's predicted to be by the end of the last simulation
/// step (as the server has it when it drives that step of ours: `Connection::meet_time`),
/// and keeps its body there. That's when our own car is drawn (and the camera placed):
/// drawn at now instead, a car keeping pace with ours would shake by its speed times the
/// step's overstep, which changes from frame to frame.
fn move_cars(
    time: Res<Time<Real>>,
    fixed: Res<Time<Fixed>>,
    connection: Res<Connection>,
    ghost_players: Res<GhostPlayers>,
    mut drawn_at: Local<Option<f64>>,
    mut commands: Commands,
    mut cars: Query<RemoteParts>,
) {
    let at = super::stamp(&time, &fixed);
    let Some(their) = connection.clock.their(at) else {
        return;
    };
    let dt = drawn_at.map_or(0.0, |before| (at - before).max(0.0) as f32);
    *drawn_at = Some(at);
    let step = connection.last_step();
    for (entity, mut car, mut transform, mut motion, looks, ghost, tagged) in &mut cars {
        let car = &mut *car;
        let latest = car.states.back().unwrap();
        let time = step.map_or(their, |step| {
            connection.meet_time(step, their, car.id, latest)
        });
        // (A state from a push on shows how it went.)
        let latest = latest.time;
        car.kicks.retain(|kick| kick.time > latest);
        let (by_now, from, ahead) = car.at(time);
        (transform.translation, transform.rotation) = car.drawn.follow(&by_now, dt);
        // In a race without contact, a car with a name (in a round, a player's) is one.
        let ghostly = car.states.back().unwrap().ghost || (ghost_players.0 && tagged);
        if ghostly && !ghost {
            commands.entity(entity).insert(Ghost);
        } else if !ghostly && ghost {
            commands.entity(entity).remove::<Ghost>();
        }
        // As the state it's from, the wheels rolling on as fast as they did from it to the
        // next (from the one before to the latest).
        *motion = car.states[from].motion;
        if let Some(last) = car.states.len().checked_sub(2) {
            let first = from.min(last);
            let (before, after) = (&car.states[first], &car.states[first + 1]);
            if after.time > before.time {
                let between = (after.time - before.time) as f32;
                let wheels = after.motion.wheels.iter().zip(&before.motion.wheels);
                for (wheel, (to, was)) in motion.wheels.iter_mut().zip(wheels) {
                    wheel.spin += (to.spin - was.spin) / between * ahead;
                }
            }
        }
        car.body = looks.map(|looks| body_of(&by_now, looks));
        car.met = time;
    }
}

/// In each simulation step, before our car meets them: the remote cars' bodies where they
/// are as of the step (`move_cars` only places them once a frame, after the steps).
fn place_bodies(
    time: Res<Time<Real>>,
    fixed: Res<Time<Fixed>>,
    connection: Res<Connection>,
    mut cars: Query<(&mut RemoteCar, &Looks)>,
) {
    let (Some(step), Some(heard)) = (
        connection.last_step(),
        connection.clock.their(super::stamp(&time, &fixed)),
    ) else {
        return;
    };
    for (mut car, looks) in &mut cars {
        car.body = car.body_at(&connection, looks, step, heard);
        if let Some(latest) = car.states.back() {
            car.met = connection.meet_time(step, heard, car.id, latest);
        }
    }
}

/// How a car moves at some moment.
#[derive(Clone, Copy, Debug)]
struct Kinematics {
    position: Vec3,
    rotation: Quat,
    velocity: Vec3,
    angular_velocity: Vec3,
}

impl Kinematics {
    /// Carried on `dt` seconds: turning as it turns, its velocity turning with it and
    /// changing by `acceleration` (in the car's frame) until it stops.
    fn ahead(mut self, mut acceleration: Vec3, dt: f32) -> Self {
        let mut local = self.rotation.inverse() * self.velocity;
        let steps = (dt / STEP).ceil().max(1.0);
        let h = dt / steps;
        for _ in 0..steps as usize {
            let next = local + acceleration * h;
            if next.dot(local) < 0.0 {
                // Braked to a stop (not into reversing).
                local = Vec3::ZERO;
                acceleration = Vec3::ZERO;
                self.angular_velocity = Vec3::ZERO;
            } else {
                local = next;
            }
            self.rotation =
                (Quat::from_scaled_axis(self.angular_velocity * h) * self.rotation).normalize();
            self.position += self.rotation * local * h;
        }
        self.velocity = self.rotation * local;
        self
    }
}

impl CarState {
    fn kinematics(&self) -> Kinematics {
        Kinematics {
            position: self.position,
            rotation: self.rotation,
            velocity: self.velocity,
            angular_velocity: self.angular_velocity,
        }
    }
}

/// The acceleration from `before` to `after`, in the car's frame (so a steady turn has
/// none: the velocity turns with the car), along and across it only (not the bumps).
fn acceleration(before: &CarState, after: &CarState) -> Vec3 {
    let between = after.time - before.time;
    if !(0.001..=1.0).contains(&between) {
        return Vec3::ZERO;
    }
    let local = |k: Kinematics| k.rotation.inverse() * k.velocity;
    let change = (local(after.kinematics()) - local(before.kinematics())) / between as f32;
    Vec3::new(change.x, 0.0, change.z).clamp_length_max(MOST_ACCELERATION)
}

/// Where a car is at `time` (its clock): its latest state carried on with the
/// acceleration between its last two (up to `PREDICT`), and how long after that state.
fn predict(states: &VecDeque<CarState>, time: f64) -> (Kinematics, f32) {
    let latest = states.back().unwrap();
    let acceleration = match states.len() {
        n if n >= 2 => acceleration(&states[n - 2], latest),
        _ => Vec3::ZERO,
    };
    let ahead = (time - latest.time).clamp(0.0, PREDICT) as f32;
    (latest.kinematics().ahead(acceleration, ahead), ahead)
}

/// Where a car was at `time` (its clock), before its latest state: on a cubic through the
/// positions and velocities of its states around it (as the first, before that), the
/// state before it and how long after that.
fn interpolate(states: &VecDeque<CarState>, time: f64) -> (Kinematics, usize, f32) {
    match states.iter().position(|state| state.time > time) {
        Some(after) if after > 0 => {
            let (a, b) = (&states[after - 1], &states[after]);
            let h = (b.time - a.time) as f32;
            let since = (time - a.time) as f32;
            let u = since / h;
            let (position, velocity) =
                hermite(a.position, a.velocity, b.position, b.velocity, h, u);
            let at = Kinematics {
                position,
                rotation: a.rotation.slerp(b.rotation, u).normalize(),
                velocity,
                angular_velocity: a.angular_velocity.lerp(b.angular_velocity, u),
            };
            (at, after - 1, since)
        }
        _ => (states[0].kinematics(), 0, 0.0),
    }
}

/// The position and velocity a share `u` of the way from `a` (moving at `va`) to `b` (at
/// `vb`), `h` seconds apart: the cubic Hermite curve through both.
fn hermite(a: Vec3, va: Vec3, b: Vec3, vb: Vec3, h: f32, u: f32) -> (Vec3, Vec3) {
    let (u2, u3) = (u * u, u * u * u);
    let position = a * (2.0 * u3 - 3.0 * u2 + 1.0)
        + va * h * (u3 - 2.0 * u2 + u)
        + b * (-2.0 * u3 + 3.0 * u2)
        + vb * h * (u3 - u2);
    let velocity = (a * (6.0 * u2 - 6.0 * u)
        + va * h * (3.0 * u2 - 4.0 * u + 1.0)
        + b * (-6.0 * u2 + 6.0 * u)
        + vb * h * (3.0 * u2 - 2.0 * u))
        / h;
    (position, velocity)
}

/// Where a car is drawn: moving as it's predicted to and closing on where that is, so a
/// new state that disagrees with the last doesn't make it jump.
#[derive(Default)]
struct Drawn(Option<(Vec3, Quat)>);

impl Drawn {
    /// Where to draw it now, `dt` after last time, with it predicted to be `at`.
    fn follow(&mut self, at: &Kinematics, dt: f32) -> (Vec3, Quat) {
        let pose = match self.0 {
            Some((position, rotation)) if position.distance(at.position) < SNAP => {
                let position = position + at.velocity * dt;
                let rotation = Quat::from_scaled_axis(at.angular_velocity * dt) * rotation;
                let closing = 1.0 - (-dt / SMOOTHING).exp();
                (
                    position.lerp(at.position, closing),
                    rotation.slerp(at.rotation, closing).normalize(),
                )
            }
            _ => (at.position, at.rotation),
        };
        self.0 = Some(pose);
        pose
    }
}

/// Remote cars not heard of for a while are out of range.
fn remove_cars(mut commands: Commands, time: Res<Time<Real>>, cars: Query<(Entity, &RemoteCar)>) {
    let now = now(&time);
    for (entity, car) in &cars {
        if now - car.heard > GONE {
            commands.entity(entity).despawn();
        }
    }
}

/// Off a server, the cars it sent go (the traffic and the other players').
fn forget_cars(
    mut commands: Commands,
    connection: Option<Res<Connection>>,
    remote: Query<Entity, With<RemoteCar>>,
) {
    if !connection.is_some_and(|c| matches!(c.status, Status::Joined { .. })) {
        for entity in &remote {
            commands.entity(entity).despawn();
        }
    }
}

/// Whether the game wants our car to be a ghost (stopped at a shop, or in one); it stays
/// one after that while it overlaps another car, so cars leaving a shop together aren't
/// thrown apart.
#[derive(Resource, Default)]
pub struct WantsGhost(pub bool);

/// Whether the other players' cars are ghosts to ours (a race without contact): drawn half
/// transparent, run into by nothing; traffic still counts. Their games do the same with
/// ours. (In a round's realm every player's car shows its name and traffic none, which
/// is how they're told apart.)
#[derive(Resource, Default)]
pub struct GhostPlayers(pub bool);

/// The remote cars ours has touched since whoever wants to know last took them (for the
/// car hunt's tag), each once.
#[derive(Resource, Default)]
pub struct Touching(pub Vec<CarId>);

/// Makes our car a ghost, or not, as `WantsGhost` and the cars around it say (never
/// alone).
fn haunt(
    mut commands: Commands,
    connection: Option<Res<Connection>>,
    (wants, spectating): (Res<WantsGhost>, Option<Res<Spectating>>),
    active: Res<ActiveCar>,
    cars: Query<(Entity, &Car, Has<Ghost>)>,
    remote: Query<(&RemoteCar, &Looks)>,
) {
    let joined = connection.is_some_and(|c| matches!(c.status, Status::Joined { .. }));
    for (entity, car, ghost) in &cars {
        let overlapping = || {
            let hull = car.setup.hull.placed(car.body.position, car.body.rotation);
            (remote.iter()).any(|(other, looks)| {
                (other.body.as_ref()).is_some_and(|body| {
                    let other = looks.hull.placed(body.position, body.rotation);
                    let reach = hull.radius() + other.radius();
                    hull.center.distance_squared(other.center) <= reach * reach
                        && hull.contact(&other).is_some()
                })
            })
        };
        let wanted = joined
            && car.index == active.0
            && (wants.0
                || spectating.as_ref().is_some_and(|s| s.0.is_some())
                || (ghost && overlapping()));
        if wanted && !ghost {
            commands.entity(entity).insert(Ghost);
        } else if !wanted && ghost {
            commands.entity(entity).remove::<Ghost>();
        }
    }
}

/// The remote cars that can be run into (not ghosts).
pub(super) type SolidCars<'w, 's> =
    Query<'w, 's, (&'static RemoteCar, &'static Looks), Without<Ghost>>;

/// Our cars push themselves off the remote cars they run into. Those aren't moved for
/// good (the other player's game pushes theirs off ours, the server its traffic), but are
/// taken to move as pushed until the server's word shows how they did (`Kick`). Ghosts,
/// ours or theirs, run into nothing.
fn push_off_remote_cars(
    time: Res<Time>,
    mut cars: Query<&mut Car, (Without<Ghost>, Without<ServerDriven>)>,
    mut remote: Query<(&mut RemoteCar, &Looks), Without<Ghost>>,
    mut touching: ResMut<Touching>,
) {
    let dt = time.delta_secs();
    let others: Vec<(CarId, RigidBody, Obb)> = (remote.iter())
        .filter_map(|(car, looks)| Some((car.id, car.body.clone()?, looks.hull)))
        .collect();
    for mut car in &mut cars {
        for touch in push_off(&mut car, &others, dt) {
            car.knocks += touch.impulse / car.body.mass;
            if !touching.0.contains(&touch.id) {
                touching.0.push(touch.id);
            }
            let Some((mut other, _)) = remote.iter_mut().find(|(other, _)| other.id == touch.id)
            else {
                continue;
            };
            let kick = Kick {
                time: other.met,
                velocity: touch.velocity,
                angular_velocity: touch.angular_velocity,
            };
            other.kicks.push(kick);
            if let Some(body) = &mut other.body {
                body.velocity += kick.velocity;
                body.angular_velocity += kick.angular_velocity;
            }
        }
    }
}

/// Our car touching a remote car: how hard (the impulse, N·s) and how that changed the
/// remote car's velocities.
struct Touch {
    id: CarId,
    impulse: f32,
    velocity: Vec3,
    angular_velocity: Vec3,
}

/// Pushes `car` off the `others` (bodies with their boxes) it runs into, which aren't
/// moved: which it touched, and how.
fn push_off(car: &mut Car, others: &[(CarId, RigidBody, Obb)], dt: f32) -> Vec<Touch> {
    let mut touched = Vec::new();
    let hull = car.setup.hull.placed(car.body.position, car.body.rotation);
    for (id, body, other) in others {
        let mut body = body.clone();
        let other = other.placed(body.position, body.rotation);
        let reach = hull.radius() + other.radius();
        if hull.center.distance_squared(other.center) > reach * reach {
            continue;
        }
        if let Some(contact) = hull.contact(&other) {
            let (velocity, angular_velocity) = (body.velocity, body.angular_velocity);
            let impulse =
                collision::resolve(&mut car.body, Some(&mut body), &[contact], OBSTACLE, dt);
            touched.push(Touch {
                id: *id,
                impulse,
                velocity: body.velocity - velocity,
                angular_velocity: body.angular_velocity - angular_velocity,
            });
        }
    }
    touched
}

/// The remote cars our car runs into in our step `step` (not ghosts), where they are as
/// of it: for driving steps again (`predict::reconcile`), as they were driven the first
/// time.
pub(super) fn others_at(
    connection: &Connection,
    step: u32,
    heard: f64,
    remote: &SolidCars,
) -> Vec<(CarId, RigidBody, Obb)> {
    (remote.iter())
        .filter_map(|(car, looks)| {
            let body = car.body_at(connection, looks, step, heard)?;
            Some((car.id, body, looks.hull))
        })
        .collect()
}

/// The end of a step driven again: `car` meets the remote cars (`others_at` the step).
/// Nothing's heard of it: the crashes were heard as it was first driven.
pub(super) fn meet_again(car: &mut Car, others: &[(CarId, RigidBody, Obb)]) {
    push_off(car, others, sim::STEP);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state of a car on a circle of `radius` around the origin at `speed`, braking at
    /// `braking`, `time` into it (and its speed then).
    fn on_circle(radius: f32, speed: f32, braking: f32, time: f64) -> CarState {
        let t = time as f32;
        let speed = speed - braking * t;
        let angle = (speed + braking * t / 2.0) * t / radius;
        let yaw_rate = speed / radius;
        let rotation = Quat::from_rotation_y(angle);
        // Starts at (radius, 0, 0) heading -z, turning left (around +y).
        let position = Vec3::new(radius * angle.cos(), 0.0, -radius * angle.sin());
        CarState {
            time,
            position,
            rotation,
            velocity: rotation * Vec3::new(0.0, 0.0, -speed),
            angular_velocity: Vec3::new(0.0, yaw_rate, 0.0),
            motion: Motion::default(),
            ghost: false,
            scripted: false,
            heli: None,
        }
    }

    #[test]
    fn a_car_braking_in_a_bend_is_predicted_along_it() {
        // 20 m/s on a 60 m bend, braking at 4 m/s², states 3 a second (far away).
        let states = VecDeque::from([
            on_circle(60.0, 20.0, 4.0, 0.0),
            on_circle(60.0, 20.0, 4.0, 1.0 / 3.0),
        ]);
        for ahead in [0.1, 0.3, 0.6] {
            let time = 1.0 / 3.0 + ahead;
            let (predicted, _) = predict(&states, time);
            let truth = on_circle(60.0, 20.0, 4.0, time);
            let off = predicted.position.distance(truth.position);
            // Straight on at the same speed, as before, for comparison.
            let latest = states[1];
            let straight = latest.position + latest.velocity * ahead as f32;
            println!(
                "{ahead} s ahead: {off:.3} m off (straight on: {:.2} m)",
                straight.distance(truth.position)
            );
            assert!(off < 0.1, "{off}");
        }
    }

    #[test]
    fn a_scripted_car_is_shown_on_its_way_between_its_states() {
        // Braking hard in a tight bend, states 30 a second: what dead reckoning overshoots.
        let states: VecDeque<CarState> = (0..KEPT)
            .map(|n| CarState {
                scripted: true,
                ..on_circle(15.0, 10.0, 6.0, n as f64 / 30.0)
            })
            .collect();
        let car = RemoteCar {
            id: 1,
            states,
            heard: 0.0,
            body: None,
            met: 0.0,
            kicks: Vec::new(),
            drawn: Drawn::default(),
        };
        let latest = car.states.back().unwrap().time;
        let mut worst: f32 = 0.0;
        for n in 0..=30 {
            let time = latest - SCRIPTED_DELAY + f64::from(n) * SCRIPTED_DELAY / 30.0;
            let (shown, from, since) = car.at(time);
            let truth = on_circle(15.0, 10.0, 6.0, time);
            worst = worst.max(shown.position.distance(truth.position));
            assert!((car.states[from].time + f64::from(since) - time).abs() < 1e-6);
        }
        assert!(worst < 0.005, "{worst} m off its way");
        // Past its latest state it's carried on only so far.
        let (held, ..) = car.at(latest + 1.0);
        let (ahead, ..) = car.at(latest + SCRIPTED_AHEAD);
        assert_eq!(held.position, ahead.position);
    }

    #[test]
    fn a_car_braking_to_a_stop_stays_stopped() {
        let state = |time: f64, speed: f32| CarState {
            time,
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            velocity: Vec3::new(0.0, 0.0, -speed),
            angular_velocity: Vec3::new(0.0, 0.2, 0.0),
            motion: Motion::default(),
            ghost: false,
            scripted: false,
            heli: None,
        };
        // 2 m/s, braking at 6 m/s²: stopped after 1/3 s, 0.33 m on.
        let states = VecDeque::from([state(0.0, 3.0), state(1.0 / 6.0, 2.0)]);
        let (stopped, _) = predict(&states, 1.0 / 6.0 + PREDICT);
        assert_eq!(stopped.velocity, Vec3::ZERO);
        assert_eq!(stopped.angular_velocity, Vec3::ZERO);
        let travelled = stopped.position.length();
        assert!((0.3..0.36).contains(&travelled), "{travelled}");
    }

    #[test]
    fn the_clock_stays_with_the_quickest_delivery_however_much_comes() {
        // A server 1000 s ahead of us sends 20 cars' states 30 times a second, two
        // messages each time; they take 50 ms at best and up to 40 ms more.
        let mut clock = ClockOffset::default();
        let mut random = 12345u32;
        let mut jitter = || {
            random = random.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            f64::from(random >> 8) / f64::from(1u32 << 24) * 0.04
        };
        let mut delay = 0.05;
        let mut worst = 0.0f64;
        for sent in 0..30 * 40 {
            let then = 1000.0 + f64::from(sent) / 30.0;
            for _message in 0..2 {
                let now = then - 1000.0 + delay + jitter();
                for _car in 0..10 {
                    clock.heard(then, now);
                }
                if (30 * 10..30 * 20).contains(&sent) {
                    // How much later than the quickest delivery it shows them.
                    let late = (now + 1000.0 - delay) - clock.their(now).unwrap();
                    worst = worst.max(late);
                }
            }
            if sent == 30 * 20 {
                // The route gets 20 ms slower for good.
                delay += 0.02;
            }
        }
        // Its idea of the server's time stays within a few ms of the quickest delivery
        // (a rule per sample sat near the middle of the jitter).
        assert!(worst < 0.006, "{worst}");
        // And it followed the slower route (20 ms in 4 s, it had 20).
        let (offset, _) = clock.0.unwrap();
        assert!((offset - (-1000.0 + 0.07)).abs() < 0.006, "{offset}");
    }

    #[test]
    fn a_car_drawn_closes_on_a_correction_without_jumping() {
        let mut drawn = Drawn::default();
        let mut at = Kinematics {
            position: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            velocity: Vec3::new(0.0, 0.0, -10.0),
            angular_velocity: Vec3::ZERO,
        };
        let dt = 1.0 / 60.0;
        drawn.follow(&at, dt);
        // A new state puts it a metre further on.
        at.position.z -= 1.0;
        let mut was = Vec3::ZERO;
        for frame in 1..=30 {
            at.position += at.velocity * dt;
            let (position, _) = drawn.follow(&at, dt);
            let step = was.distance(position);
            assert!(step < 10.0 * dt + 0.2, "frame {frame}: moved {step} m");
            was = position;
        }
        // Half a second on (five time constants), it's there.
        assert!(
            was.distance(at.position) < 0.01,
            "{}",
            was.distance(at.position)
        );
    }
}
