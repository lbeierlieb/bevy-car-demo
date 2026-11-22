//! The car hunt's rules, the hunted's settings and how a round of it ended (the round
//! itself, who's in and how it stands, is `round`'s).
//!
//! A round starts as a lobby at the event's marker: the first one in is the hunted, the
//! rest hunters; the hunted sets the options, may hand the part to another and starts it
//! (with at least one hunter). Running, it's in a realm of its own: everyone starts by the
//! marker, the hunters held for the head start; they win once the hunted's car has been
//! slower than `CAUGHT_SPEED` for `CAUGHT_TIME` (anywhere), or (with `Catch::Tag`) once a
//! hunter's car touches it, the hunted when the time is up. If the hunted leaves, or every hunter, it ends. Then everyone is back at the marker,
//! in its lobby.

use crate::net::wire::{Reader, Wire};

/// The hunted is caught slower than this for this long (m/s, s).
pub const CAUGHT_SPEED: f32 = 10.0 / 3.6;
pub const CAUGHT_TIME: f64 = 10.0;
/// A touch counts only if the two cars are this near on the server (m): their states can be
/// a little behind, but not further apart than that.
pub const TAG_RANGE: f32 = 15.0;
/// What the hunted can set the head start and the time to win to (s).
pub const HEAD_START: (u16, u16) = (0, 120);
pub const DURATION: (u16, u16) = (60, 3600);

/// Which cars are driven.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cars {
    /// Everyone their own.
    Own,
    /// The hunters a muscle car, the hunted a rally car (lent for the round).
    MuscleVsRally,
}

impl Cars {
    pub const ALL: [Cars; 2] = [Self::Own, Self::MuscleVsRally];

    pub fn title(self) -> &'static str {
        match self {
            Self::Own => "Player cars",
            Self::MuscleVsRally => "Muscle vs rally",
        }
    }
}

/// How the hunters catch the hunted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Catch {
    /// Keep them slow (`CAUGHT_SPEED` for `CAUGHT_TIME`).
    Stop,
    /// Touch their car, or keep them slow.
    Tag,
}

impl Catch {
    pub const ALL: [Catch; 2] = [Self::Stop, Self::Tag];

    pub fn title(self) -> &'static str {
        match self {
            Self::Stop => "Stop them",
            Self::Tag => "Tag them",
        }
    }

    /// The rule, for people.
    pub fn rule(self) -> &'static str {
        match self {
            Self::Stop => {
                "The hunted has to get away. The hunters catch them by keeping them slower \
                 than 10 km/h for 10 s."
            }
            Self::Tag => {
                "The hunted has to get away. The hunters catch them by touching their car \
                 (or by keeping them slower than 10 km/h for 10 s)."
            }
        }
    }
}

/// What the hunted sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    /// How long the hunters are held at the start (s).
    pub head_start: u16,
    /// How long the hunted has to get away, from the end of the head start (s).
    pub duration: u16,
    /// Whether there's traffic (a round's own).
    pub traffic: bool,
    /// Whether a hunter's R calls the rescue helicopter (the time it takes is their
    /// penalty for crashing); the hunted's never does.
    pub hunters_reset: bool,
    pub cars: Cars,
    pub catch: Catch,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            head_start: 10,
            duration: 600,
            traffic: true,
            hunters_reset: true,
            cars: Cars::Own,
            catch: Catch::Tag,
        }
    }
}

impl Settings {
    /// Within what can be set.
    pub fn clamped(self) -> Self {
        Self {
            head_start: self.head_start.clamp(HEAD_START.0, HEAD_START.1),
            duration: self.duration.clamp(DURATION.0, DURATION.1),
            ..self
        }
    }
}

/// How a round ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Ending {
    /// The hunters stopped the hunted, this long after the head start (s).
    Caught(f32),
    /// A hunter (as they were called) touched the hunted's car, this long after the head
    /// start (s).
    Tagged { by: String, after: f32 },
    /// The hunted got away: the time was up.
    Escaped,
    /// The hunted left.
    HuntedLeft,
    /// Every hunter left.
    NoHunters,
}

/// The last round's end, as shown in the lobby afterwards.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    /// Who was hunted (as they were called).
    pub hunted: String,
    pub ending: Ending,
}

impl Outcome {
    /// For people.
    pub fn text(&self) -> String {
        let name = &self.hunted;
        let time = |after: f32| {
            let after = after.max(0.0) as u32;
            format!("{}:{:02}", after / 60, after % 60)
        };
        match &self.ending {
            Ending::Caught(after) => format!("The hunters caught {name} after {}", time(*after)),
            Ending::Tagged { by, after } => format!("{by} tagged {name} after {}", time(*after)),
            Ending::Escaped => format!("{name} got away"),
            Ending::HuntedLeft => format!("{name} left the hunt: it's over"),
            Ending::NoHunters => format!("Every hunter left: {name} got away"),
        }
    }
}

impl Wire for Cars {
    fn put(&self, out: &mut Vec<u8>) {
        (*self as u8).put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Self::ALL.get(usize::from(r.get::<u8>()?)).copied()
    }
}

impl Wire for Catch {
    fn put(&self, out: &mut Vec<u8>) {
        (*self as u8).put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Self::ALL.get(usize::from(r.get::<u8>()?)).copied()
    }
}

impl Wire for Settings {
    fn put(&self, out: &mut Vec<u8>) {
        self.head_start.put(out);
        self.duration.put(out);
        self.traffic.put(out);
        self.hunters_reset.put(out);
        self.cars.put(out);
        self.catch.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            head_start: r.get()?,
            duration: r.get()?,
            traffic: r.get()?,
            hunters_reset: r.get()?,
            cars: r.get()?,
            catch: r.get()?,
        })
    }
}

impl Wire for Outcome {
    fn put(&self, out: &mut Vec<u8>) {
        self.hunted.put(out);
        match &self.ending {
            Ending::Caught(after) => {
                0u8.put(out);
                after.put(out);
            }
            Ending::Escaped => 1u8.put(out),
            Ending::HuntedLeft => 2u8.put(out),
            Ending::NoHunters => 3u8.put(out),
            Ending::Tagged { by, after } => {
                4u8.put(out);
                by.put(out);
                after.put(out);
            }
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let hunted = r.get()?;
        let ending = match r.get::<u8>()? {
            0 => Ending::Caught(r.get()?),
            1 => Ending::Escaped,
            2 => Ending::HuntedLeft,
            3 => Ending::NoHunters,
            4 => Ending::Tagged {
                by: r.get()?,
                after: r.get()?,
            },
            _ => return None,
        };
        Some(Self { hunted, ending })
    }
}
