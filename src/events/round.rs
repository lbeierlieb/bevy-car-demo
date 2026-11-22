//! A round of an event, whatever the game: who's in, who decides, the game's settings and
//! how it stands (a lobby at the marker, then under way in a realm of its own, then back
//! in the lobby with how it ended), and what players ask of it.

use super::hunt;
use crate::net::protocol::{CarId, PlayerId};
use crate::net::wire::{Reader, Wire};

/// What's played in a round, with its settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Game {
    Hunt(hunt::Settings),
}

/// How a round under way stands, per game.
#[derive(Clone, Debug, PartialEq)]
pub enum Play {
    /// The hunted has been slow since then (s, the server's clock), if they are.
    Hunt { slow: Option<f64> },
}

/// How a round ended, per game.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Hunt(hunt::Outcome),
}

impl Outcome {
    /// For people, in a line.
    pub fn text(&self) -> String {
        match self {
            Self::Hunt(outcome) => outcome.text(),
        }
    }
}

/// How a round stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    /// Waiting at the marker for the host to start it, with how the last round there
    /// ended, if the players came from one.
    Lobby { last: Option<Outcome> },
    /// Under way since `start` on the server's clock (s).
    Running { start: f64, play: Play },
}

/// A round of an event, as its players are told.
#[derive(Clone, Debug, PartialEq)]
pub struct Round {
    /// The event (its place in the server's list).
    pub event: u16,
    /// Who's in, in the order they came (each starts at their own spot).
    pub players: Vec<PlayerId>,
    /// Who sets the options and starts it: in a hunt, the hunted.
    pub host: PlayerId,
    pub game: Game,
    pub phase: Phase,
}

impl Round {
    /// Where `player` starts: their place among the players.
    pub fn slot(&self, player: PlayerId) -> usize {
        self.players.iter().position(|&p| p == player).unwrap_or(0)
    }

    pub fn running(&self) -> bool {
        matches!(self.phase, Phase::Running { .. })
    }

    /// The hunt's settings, if it's a hunt.
    pub fn hunt(&self) -> Option<&hunt::Settings> {
        match &self.game {
            Game::Hunt(settings) => Some(settings),
        }
    }
}

/// What a player asks of the events.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Into the lobby at this event (its place in the list): stopped on its marker.
    Enter(u16),
    /// Out of the round they're in.
    Leave,
    /// The host's settings.
    Game(Game),
    /// Our car touched this one (a player's, we hope: the server checks), for a tag.
    Touched(CarId),
    /// The host hands their part (in a hunt, the hunted's) to this player.
    Host(PlayerId),
    /// The host starts the round.
    Start,
}

impl Wire for Game {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            Self::Hunt(settings) => {
                0u8.put(out);
                settings.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Self::Hunt(r.get()?),
            _ => return None,
        })
    }
}

impl Wire for Play {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            Self::Hunt { slow } => {
                0u8.put(out);
                slow.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Self::Hunt { slow: r.get()? },
            _ => return None,
        })
    }
}

impl Wire for Outcome {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            Self::Hunt(outcome) => {
                0u8.put(out);
                outcome.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Self::Hunt(r.get()?),
            _ => return None,
        })
    }
}

impl Wire for Round {
    fn put(&self, out: &mut Vec<u8>) {
        self.event.put(out);
        self.players.put(out);
        self.host.put(out);
        self.game.put(out);
        match &self.phase {
            Phase::Lobby { last } => {
                0u8.put(out);
                last.put(out);
            }
            Phase::Running { start, play } => {
                1u8.put(out);
                start.put(out);
                play.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            event: r.get()?,
            players: r.get()?,
            host: r.get()?,
            game: r.get()?,
            phase: match r.get::<u8>()? {
                0 => Phase::Lobby { last: r.get()? },
                1 => Phase::Running {
                    start: r.get()?,
                    play: r.get()?,
                },
                _ => return None,
            },
        })
    }
}

impl Wire for Action {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            Action::Enter(event) => {
                0u8.put(out);
                event.put(out);
            }
            Action::Leave => 1u8.put(out),
            Action::Game(game) => {
                2u8.put(out);
                game.put(out);
            }
            Action::Host(player) => {
                3u8.put(out);
                player.put(out);
            }
            Action::Start => 4u8.put(out),
            Action::Touched(car) => {
                5u8.put(out);
                car.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Action::Enter(r.get()?),
            1 => Action::Leave,
            2 => Action::Game(r.get()?),
            3 => Action::Host(r.get()?),
            4 => Action::Start,
            5 => Action::Touched(r.get()?),
            _ => return None,
        })
    }
}
