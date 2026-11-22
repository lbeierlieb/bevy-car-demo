//! The server's side of the events: the rounds of car hunts, from the lobby at a marker
//! to the end (see `hunt`). Each running round has a realm of its own: its players' entities
//! get its `Realm` (so they see only each other and its traffic), and the traffic has
//! the round's density there (none without traffic).

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;

use super::hunt::{
    Action, CAUGHT_SPEED, CAUGHT_TIME, Catch, Ending, Outcome, Phase, Round, TAG_RANGE,
};
use crate::net::protocol::CarId;
use crate::net::protocol::{PlayerId, ToClient};
use crate::net::server::{Peer, Server};
use crate::traffic::{Realm, Traffic};

/// A player asks to enter only this near the event's marker (m): stopped on it, as their
/// game sees it, give or take what their state is behind.
const ENTER_RANGE: f32 = 15.0;

/// A player asked something of the events.
#[derive(Message, Clone, Debug)]
pub struct EventAction {
    pub player: PlayerId,
    pub action: Action,
}

/// Every round there is, lobbies and running ones.
#[derive(Resource, Default)]
pub struct Rounds {
    list: Vec<Hunt>,
    /// The last realm given to a round.
    realm: u32,
    /// Who's in which event, as everyone was last told, and who was told.
    busy: Vec<(PlayerId, u16)>,
    told_busy: HashSet<PlayerId>,
    /// The players given a round's realm (only theirs are taken away again).
    in_realms: HashSet<PlayerId>,
}

impl Rounds {
    /// The rounds there are.
    pub fn iter(&self) -> impl Iterator<Item = &Round> {
        self.list.iter().map(|hunt| &hunt.round)
    }
}

/// A round on the server.
struct Hunt {
    round: Round,
    /// Its realm while it runs.
    realm: Realm,
    /// What the hunted is called (kept for the outcome should they leave).
    hunted_name: String,
    /// Its players need telling how it stands.
    changed: bool,
}

impl Hunt {
    fn has(&self, player: PlayerId) -> bool {
        self.round.players.contains(&player)
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
    time: Res<Time<Real>>,
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
    for hunt in &mut rounds.list {
        for player in hunt.round.players.clone() {
            if !present.contains_key(&player) {
                leave(hunt, player, &name);
            }
        }
    }
    for EventAction { player, action } in actions.read().cloned() {
        let mine = rounds.list.iter().position(|hunt| hunt.has(player));
        match action {
            Action::Enter(event) => {
                let near = (events.get(usize::from(event)))
                    .zip(present.get(&player).and_then(|p| p.1))
                    .is_some_and(|(event, at)| event.at.distance(at.xz()) < ENTER_RANGE);
                if mine.is_some() || !near {
                    continue;
                }
                let lobby = (rounds.list.iter_mut())
                    .find(|hunt| hunt.round.event == event && !hunt.round.running());
                match lobby {
                    Some(hunt) => {
                        hunt.round.players.push(player);
                        hunt.changed = true;
                    }
                    None => rounds.list.push(Hunt {
                        round: Round {
                            event,
                            players: vec![player],
                            hunted: player,
                            settings: default(),
                            phase: Phase::Lobby { last: None },
                        },
                        realm: Realm::NORMAL,
                        hunted_name: name(player),
                        changed: true,
                    }),
                }
            }
            Action::Leave => {
                if let Some(i) = mine {
                    leave(&mut rounds.list[i], player, &name);
                    out.push(player);
                }
            }
            Action::Settings(settings) => {
                if let Some(hunt) = mine.map(|i| &mut rounds.list[i])
                    && hunt.round.hunted == player
                    && !hunt.round.running()
                {
                    hunt.round.settings = settings.clamped();
                    hunt.changed = true;
                }
            }
            Action::Hunted(other) => {
                if let Some(hunt) = mine.map(|i| &mut rounds.list[i])
                    && hunt.round.hunted == player
                    && !hunt.round.running()
                    && hunt.has(other)
                {
                    hunt.round.hunted = other;
                    hunt.hunted_name = name(other);
                    hunt.changed = true;
                }
            }
            Action::Touched(car) => {
                let (Some(&other), Some(hunt)) =
                    (owners.get(&car), mine.map(|i| &mut rounds.list[i]))
                else {
                    continue;
                };
                let Phase::Running { start, .. } = hunt.round.phase else {
                    continue;
                };
                let hunted = hunt.round.hunted;
                let hunter = match () {
                    _ if player == hunted => other,
                    _ if other == hunted => player,
                    _ => continue,
                };
                let hunting = start + f64::from(hunt.round.settings.head_start);
                let at = |p: PlayerId| present.get(&p).and_then(|p| p.1);
                let near =
                    (at(player).zip(at(other))).is_some_and(|(a, b)| a.distance(b) < TAG_RANGE);
                if hunt.round.settings.catch == Catch::Tag
                    && hunt.has(hunter)
                    && now >= hunting
                    && near
                {
                    let by = name(hunter);
                    end(
                        hunt,
                        Ending::Tagged {
                            by,
                            after: (now - hunting) as f32,
                        },
                    );
                }
            }
            Action::Start => {
                if let Some(hunt) = mine.map(|i| &mut rounds.list[i])
                    && hunt.round.hunted == player
                    && !hunt.round.running()
                    && hunt.round.players.len() >= 2
                {
                    rounds.realm += 1;
                    hunt.realm = Realm(rounds.realm);
                    let cars = if hunt.round.settings.traffic {
                        traffic.cars
                    } else {
                        0
                    };
                    traffic.realms.insert(hunt.realm, cars);
                    hunt.round.phase = Phase::Running {
                        start: now,
                        slow: None,
                    };
                    hunt.changed = true;
                    info!(
                        "A car hunt started in realm {} with {} players",
                        hunt.realm.0,
                        hunt.round.players.len()
                    );
                }
            }
        }
    }

    // The rounds under way: is the hunted caught, or is the time up?
    for hunt in &mut rounds.list {
        let settings = hunt.round.settings;
        let Phase::Running { start, slow } = &mut hunt.round.phase else {
            continue;
        };
        let hunting = *start + f64::from(settings.head_start);
        if now >= hunting + f64::from(settings.duration) {
            end(hunt, Ending::Escaped);
            continue;
        }
        let speed = present.get(&hunt.round.hunted).and_then(|p| p.2);
        let Some(speed) = speed.filter(|_| now >= hunting) else {
            continue;
        };
        if speed >= CAUGHT_SPEED {
            hunt.changed |= slow.take().is_some();
        } else if let Some(since) = *slow {
            if now - since >= CAUGHT_TIME {
                end(hunt, Ending::Caught((now - hunting) as f32));
            }
        } else {
            *slow = Some(now);
            hunt.changed = true;
        }
    }
    // The realms of rounds over go, with their traffic.
    for hunt in &mut rounds.list {
        if !hunt.round.running() && hunt.realm != Realm::NORMAL {
            traffic.realms.remove(&hunt.realm);
            hunt.realm = Realm::NORMAL;
        }
    }
    // Players of a round just over join the lobby already waiting at its marker, if any
    // (keeping its hunted): one lobby per event.
    let mut i = 0;
    while i < rounds.list.len() {
        let round = &rounds.list[i].round;
        let (event, ended) = (
            round.event,
            matches!(round.phase, Phase::Lobby { last: Some(_) }),
        );
        let waiting = (0..rounds.list.len()).find(|&j| {
            let other = &rounds.list[j].round;
            j != i
                && other.event == event
                && !other.running()
                && !round.running()
                && (ended || (j < i && !matches!(other.phase, Phase::Lobby { last: Some(_) })))
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
    rounds.list.retain(|hunt| !hunt.round.players.is_empty());

    // Each player in the realm of their round, if it runs.
    let realm_of: HashMap<PlayerId, Realm> = (rounds.list.iter())
        .filter(|hunt| hunt.round.running())
        .flat_map(|hunt| hunt.round.players.iter().map(|&p| (p, hunt.realm)))
        .collect();
    let busy: Vec<(PlayerId, u16)> = (rounds.list.iter())
        .flat_map(|hunt| hunt.round.players.iter().map(|&p| (p, hunt.round.event)))
        .collect();
    let busy_changed = busy != rounds.busy;
    if busy_changed {
        rounds.busy = busy;
    }
    rounds
        .told_busy
        .retain(|player| present.contains_key(player));
    for (entity, mut peer, realm) in &mut peers {
        let wanted = realm_of.get(&peer.id).copied();
        let ours = wanted.is_some() || rounds.in_realms.contains(&peer.id);
        if ours && realm.copied() != wanted {
            match wanted {
                Some(realm) => commands.entity(entity).insert(realm),
                None => commands.entity(entity).remove::<Realm>(),
            };
        }
        if peer.name.is_none() {
            continue;
        }
        if out.contains(&peer.id) {
            peer.tell(&ToClient::Round(None));
        }
        let round = (rounds.list.iter()).find(|hunt| hunt.changed && hunt.has(peer.id));
        if let Some(hunt) = round {
            peer.tell(&ToClient::Round(Some(hunt.round.clone())));
        }
        if busy_changed || rounds.told_busy.insert(peer.id) {
            peer.tell(&ToClient::Busy(rounds.busy.clone()));
        }
    }
    for hunt in &mut rounds.list {
        hunt.changed = false;
    }
    rounds.in_realms = realm_of.into_keys().collect();
}

/// `player` leaves the round: in the lobby someone else is hunted if they were; under way
/// it ends if they were hunted, or if they were the last hunter.
fn leave(hunt: &mut Hunt, player: PlayerId, name: &impl Fn(PlayerId) -> String) {
    hunt.round.players.retain(|&p| p != player);
    hunt.changed = true;
    let was_hunted = hunt.round.hunted == player;
    if hunt.round.running() {
        if was_hunted {
            end(hunt, Ending::HuntedLeft);
        } else if hunt.round.players.len() < 2 {
            end(hunt, Ending::NoHunters);
        }
    }
    if was_hunted && let Some(&next) = hunt.round.players.first() {
        hunt.round.hunted = next;
        hunt.hunted_name = name(next);
    }
}

/// The round is over: back to its lobby with how it ended (its realm goes afterwards).
fn end(hunt: &mut Hunt, ending: Ending) {
    let outcome = Outcome {
        hunted: hunt.hunted_name.clone(),
        ending,
    };
    info!("A car hunt ended: {}", outcome.text());
    hunt.round.phase = Phase::Lobby {
        last: Some(outcome),
    };
    hunt.changed = true;
}
