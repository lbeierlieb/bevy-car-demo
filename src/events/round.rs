//! A round of an event, whatever the game: who's in, who decides, the game's settings and
//! how it stands (a lobby at the marker, then under way in a realm of its own, then back
//! in the lobby with how it ended), and what players ask of it.

use super::{hunt, race};
use crate::car::ResetRule;
use crate::net::protocol::{CarId, PlayerId};
use crate::net::wire::{Reader, Wire};

/// What's played in a round, with its settings.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Game {
    Hunt(hunt::Settings),
    /// A race, racers from the lobby.
    Race(race::Settings),
    /// A time trial: a round of one, private.
    Trial(race::Settings),
}

impl Game {
    /// A race's or time trial's settings.
    pub fn race(&self) -> Option<&race::Settings> {
        match self {
            Self::Race(settings) | Self::Trial(settings) => Some(settings),
            Self::Hunt(_) => None,
        }
    }

    /// The same game with other settings of its kind, if they are (a hunt's for a hunt,
    /// a race's for a race or time trial).
    pub fn with(&self, other: &Game) -> Option<Game> {
        match (self, other) {
            (Self::Hunt(_), Self::Hunt(s)) => Some(Self::Hunt(*s)),
            (Self::Race(_), Self::Race(s) | Self::Trial(s)) => Some(Self::Race(*s)),
            (Self::Trial(_), Self::Race(s) | Self::Trial(s)) => Some(Self::Trial(*s)),
            _ => None,
        }
    }

    /// What it is, for people.
    pub fn title(&self) -> &'static str {
        match self {
            Self::Hunt(_) => "Car hunt",
            Self::Race(_) => "Race",
            Self::Trial(_) => "Time trial",
        }
    }
}

/// How a round under way stands, per game.
#[derive(Clone, Debug, PartialEq)]
pub enum Play {
    /// The hunted has been slow since then (s, the server's clock), if they are.
    Hunt { slow: Option<f64> },
    /// A race's or time trial's.
    Race(race::Standing),
}

/// How a round ended, per game.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Hunt(hunt::Outcome),
    Race(race::Results),
}

impl Outcome {
    /// For people, in a line.
    pub fn text(&self) -> String {
        match self {
            Self::Hunt(outcome) => outcome.text(),
            Self::Race(results) => results.text(),
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
    /// A race's or time trial's best times on the server (this session), for its route
    /// and traffic, best first.
    pub best: Vec<race::Best>,
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
            _ => None,
        }
    }

    /// What R does for `player` while the round runs (`course`: a race's), the same in
    /// their game and on the server: in a hunt, the rescue helicopter takes hunters to the
    /// road if the hunted allows it, never the hunted; in a race that allows it, to the
    /// last gate passed (before any, the grid spot); never in a time trial.
    pub fn reset_rule(&self, course: Option<&race::Course>, player: PlayerId) -> ResetRule {
        if !self.running() {
            return ResetRule::Road;
        }
        match &self.game {
            Game::Hunt(settings) if settings.hunters_reset && self.host != player => {
                ResetRule::Road
            }
            Game::Hunt(_) | Game::Trial(_) => ResetRule::Never,
            Game::Race(settings) => match course {
                Some(course) if settings.resets => {
                    let passed = (self.standing())
                        .and_then(|standing| standing.racers.iter().find(|r| r.player == player))
                        .map_or(0, |racer| usize::from(racer.gates));
                    let (p, yaw) = course.reset_spot(passed, self.slot(player));
                    ResetRule::Spot(p, yaw)
                }
                _ => ResetRule::Never,
            },
        }
    }

    /// How long `player`'s car is held when the round starts (s): a hunter for the head
    /// start, racers for the countdown.
    pub fn hold(&self, player: PlayerId) -> f32 {
        match &self.game {
            Game::Hunt(settings) if self.host != player => f32::from(settings.head_start),
            Game::Hunt(_) => 0.0,
            Game::Race(_) | Game::Trial(_) => race::COUNTDOWN as f32,
        }
    }

    /// The chassis (its place in `CHASSIS`) lent to `player` for the round, with its base
    /// engine, if it lends cars ("muscle vs rally": the hunted the rally car, the hunters
    /// the muscle car with its smaller V8).
    pub fn lent_chassis(&self, player: PlayerId) -> Option<usize> {
        let name = match &self.game {
            Game::Hunt(settings) if settings.cars == hunt::Cars::MuscleVsRally => {
                if self.host == player {
                    "Skarn"
                } else {
                    "Harrow"
                }
            }
            _ => return None,
        };
        crate::chassis::CHASSIS.iter().position(|c| c.name == name)
    }

    /// How a race or time trial under way stands.
    pub fn standing(&self) -> Option<&race::Standing> {
        match &self.phase {
            Phase::Running {
                play: Play::Race(standing),
                ..
            } => Some(standing),
            _ => None,
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
    /// A time trial at this event (stopped on its marker): a round of one.
    Trial(u16),
    /// The time trial under way starts again, from the grid.
    Restart,
}

impl Wire for Game {
    fn put(&self, out: &mut Vec<u8>) {
        match self {
            Self::Hunt(settings) => {
                0u8.put(out);
                settings.put(out);
            }
            Self::Race(settings) => {
                1u8.put(out);
                settings.put(out);
            }
            Self::Trial(settings) => {
                2u8.put(out);
                settings.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Self::Hunt(r.get()?),
            1 => Self::Race(r.get()?),
            2 => Self::Trial(r.get()?),
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
            Self::Race(standing) => {
                1u8.put(out);
                standing.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Self::Hunt { slow: r.get()? },
            1 => Self::Race(r.get()?),
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
            Self::Race(results) => {
                1u8.put(out);
                results.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(match r.get::<u8>()? {
            0 => Self::Hunt(r.get()?),
            1 => Self::Race(r.get()?),
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
        self.best.put(out);
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
            best: r.get()?,
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
            Action::Trial(event) => {
                6u8.put(out);
                event.put(out);
            }
            Action::Restart => 7u8.put(out),
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
            6 => Action::Trial(r.get()?),
            7 => Action::Restart,
            _ => return None,
        })
    }
}
