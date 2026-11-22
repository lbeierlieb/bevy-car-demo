//! Sounds besides the engines: the driven car's tires squealing as they slide, its gears
//! grinding when the lever is pushed into a gear that can't go in, and the trumpet calling
//! a race's countdown.
//!
//! - **Squeal**: rubber sticking and slipping on the road rings the tread at a pitch of
//!   its own, a few hundred Hz to a kilohertz. Each wheel is a band of noise at that
//!   pitch and two overtones (narrow band-passes, so it's tonal but alive), wobbling in
//!   pitch and fluttering in level, with a little broadband scrub under it. It starts as
//!   the tire nears its peak slip and is loudest past it, louder under more load, and
//!   only on paved ground and while the tread actually slides over the road.
//! - **Grinding**: the dog teeth of a gear that isn't turning with the shaft clash against
//!   each other. Clicks at the rate the teeth meet (the speed difference times the
//!   teeth), each a little different, ring a few metallic resonances.
//! - **Crashes**: the body hitting houses, trees, fences, other cars or the ground. A
//!   knock thuds (a low boom gliding down), crunches (a burst of noise and crackles of
//!   crumpling metal ringing the panels' resonances, longer the harder) and, hard enough,
//!   shatters glass (bright pings scattered over most of a second). Pushed or sliding
//!   along something, the body scrapes (a rough band of noise).
//! - **Trumpet**: a band-limited sawtooth through a low-pass that opens as the note
//!   swells (brass gets brighter the louder it plays), a lip that scoops up into the
//!   pitch and a vibrato that sets in late. Two players a few cents apart per note.
//!
//! As with the engine, the game only publishes targets (`Cues`), and the synth glides
//! towards them every sample.

use std::f32::consts::{PI, TAU};
use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bevy::audio::{AddAudioSource, ChannelCount, Decodable, SampleRate, Source};
use bevy::prelude::*;

use super::{EngineAudio, Resonator, SAMPLE_RATE, pan, read, store};
use crate::car::{ActiveCar, Car, Rescued, ReverseRefused};
use crate::physics::surface::Surface;

/// The squeal's pitch (Hz) at the peak slip, its overtones (times the pitch, gain) and
/// the band-passes' Q: a wider band sounds rougher.
const SQUEAL_PITCH: f32 = 780.0;
const SQUEAL_OVERTONES: [(f32, f32); 3] = [(1.0, 1.0), (2.03, 0.45), (3.1, 0.2)];
const SQUEAL_Q: f32 = 28.0;
/// How each wheel's tire rings a little differently (times the pitch).
const SQUEAL_DETUNE: [f32; 4] = [1.0, 1.035, 0.955, 0.985];
/// The pitch rises with how far past the peak the tire slides (share per peak's worth).
const SQUEAL_RISE: f32 = 0.12;
/// Pitch wobble: depth (share) and how quickly it wanders (Hz).
const SQUEAL_WOBBLE: (f32, f32) = (0.025, 6.0);
/// Level flutter (stick-slip chatter): depth and rate (Hz).
const SQUEAL_FLUTTER: (f32, f32) = (0.5, 28.0);
/// Broadband scrub under the tone, relative to it, and its low-pass (Hz).
const SCRUB: (f32, f32) = (0.25, 2500.0);
/// Level of one wheel squealing all out.
const SQUEAL: f32 = 0.16;
/// Slip, relative to the peak (combined, as the tire model has it), where the squeal
/// starts and where it's full.
const SQUEAL_FROM: f32 = 0.55;
const SQUEAL_FULL: f32 = 1.4;
/// Speed the tread slides over the road at (m/s) for a full squeal: less only scuffs.
const SQUEAL_SLIDE: f32 = 2.0;
/// How quickly a squeal swells and dies (s).
const SQUEAL_GLIDE: f32 = 0.03;

/// Dog teeth meeting per revolution of the difference, the clash rate's range (Hz), the
/// speed difference (rpm) for a full grind and the least share of it.
const TEETH: f32 = 24.0;
const CLASH_RATE: (f32, f32) = (35.0, 900.0);
const GRIND_FULL: f32 = 1500.0;
const GRIND_LEAST: f32 = 0.45;
/// The gearbox's resonances: frequency (Hz), Q, gain.
const GRIND_RINGS: [(f32, f32, f32); 4] = [
    (1850.0, 18.0, 1.0),
    (3120.0, 24.0, 0.8),
    (4730.0, 30.0, 0.6),
    (6900.0, 26.0, 0.4),
];
/// Rattle on top (teeth bouncing off each other): depth and rate (Hz).
const GRIND_RATTLE: (f32, f32) = (0.6, 13.0);
/// Level of a full grind, and how quickly it starts and stops (s).
const GRIND: f32 = 0.15;
/// Noise between the clicks (scraping), relative to a click.
const GRIND_HISS: f32 = 0.01;
const GRIND_GLIDE: f32 = 0.008;
/// How long a refused reverse grinds when asked for with a key or paddle (s).
const GRIND_BURST: f32 = 0.3;

/// A knock (the body's speed change in a frame, m/s) this hard is a crash, and this hard
/// is the hardest; a crash right after another (s) only counts if it's harder.
const CRASH_FROM: f32 = 1.0;
const CRASH_FULL: f32 = 15.0;
const CRASH_GAP: f32 = 0.15;
/// The thud: level, its pitch at the start and the end (Hz), how fast it dies (s).
const THUD: f32 = 0.9;
const THUD_PITCH: (f32, f32) = (120.0, 50.0);
const THUD_DECAY: f32 = 0.12;
/// The panels' resonances: frequency (Hz), Q, gain.
const PANELS: [(f32, f32, f32); 6] = [
    (310.0, 9.0, 1.0),
    (520.0, 10.0, 0.9),
    (870.0, 12.0, 0.8),
    (1380.0, 12.0, 0.6),
    (2250.0, 14.0, 0.45),
    (3600.0, 14.0, 0.3),
];
/// The crunch: level, the first burst of noise (s), and the crackles: how many a second
/// at the start of the hardest crash and how long they go on (s, at the least and the
/// hardest).
const CRUNCH: f32 = 0.05;
const BURST: f32 = 0.02;
const CRACKLES: f32 = 900.0;
const CRACKLING: (f32, f32) = (0.05, 0.4);
/// Glass breaks from this strength on: pings a second at the start, for how long (s),
/// their pitch range (Hz) and how fast they die (s), and their level.
const GLASS_FROM: f32 = 0.45;
const PINGS: f32 = 400.0;
const PINGING: f32 = 0.3;
const PING_PITCH: (f32, f32) = (2500.0, 9000.0);
const PING_DECAY: (f32, f32) = (0.015, 0.08);
const PING: f32 = 0.12;
/// At most this many crashes and pings at once.
const IMPACTS: usize = 4;
const RINGING: usize = 32;
/// Scraping: the knocks a second (m/s²) and the speed (m/s) for a full scrape, how
/// quickly it follows (s), its bands (Hz, Q), roughness (depth and rate, Hz) and level.
const SCRAPE_KNOCKS: f32 = 8.0;
const SCRAPE_SPEED: f32 = 5.0;
const SCRAPE_GLIDE: f32 = 0.05;
const SCRAPE_BANDS: [(f32, f32); 3] = [(650.0, 4.0), (1500.0, 5.0), (3300.0, 6.0)];
const SCRAPE_ROUGH: (f32, f32) = (0.7, 35.0);
const SCRAPE: f32 = 0.3;

/// The trumpet's notes: the countdown's (Hz) and "Go!"'s chord.
const COUNT_NOTE: f32 = 392.0;
const GO_CHORD: [f32; 4] = [523.25, 659.26, 783.99, 1046.5];
/// How long each sounds before its release (s).
const COUNT_LENGTH: f32 = 0.35;
const GO_LENGTH: f32 = 1.1;
/// Envelope: attack, the swell's overshoot and how fast it settles, release (s).
const ATTACK: f32 = 0.03;
const OVERSHOOT: (f32, f32) = (0.2, 0.08);
const RELEASE: f32 = 0.12;
/// The lip's scoop (share flat at the start) and how long it takes (s).
const SCOOP: (f32, f32) = (0.04, 0.05);
/// Vibrato: depth (share), rate (Hz), and when it sets in (s).
const VIBRATO: (f32, f32, f32) = (0.004, 5.5, 0.18);
/// The low-pass's cutoff, times the note, closed and open.
const BRIGHTNESS: (f32, f32) = (1.5, 9.0);
/// The two players' detune (share either way) and each one's level.
const DETUNE: f32 = 0.0025;
const TRUMPET: f32 = 0.15;
/// At most this many trumpets at once (a chord's notes each take two).
const PLAYERS: usize = 8;

/// A note to play on the trumpet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    /// A countdown's beat.
    Count,
    /// The start.
    Go,
}

/// Targets shared between the game and the audio thread (f32 bits in atomics).
#[derive(Default)]
struct Cues {
    /// Per wheel: the squeal's level (0..1), its pitch (times the wheel's own) and pan.
    squeal: [[AtomicU32; 3]; 4],
    /// Grinding: level (0..1) and the teeth's clash rate (Hz).
    grind: [AtomicU32; 2],
    /// Calls asked for: a counter (above the lowest 4 bits) and the latest's `Call`.
    call: AtomicU32,
    /// Crashes: a counter (above the lowest 8 bits) and the latest's strength (0..255).
    crash: AtomicU32,
    /// Scraping: level (0..1) and pan.
    scrape: [AtomicU32; 2],
    /// Master volume (0 muted).
    volume: AtomicU32,
}

impl Cues {
    fn ask(&self, call: Call) {
        let kind = match call {
            Call::Count => 0,
            Call::Go => 1,
        };
        let count = (self.call.load(Ordering::Relaxed) >> 4).wrapping_add(1);
        self.call.store(count << 4 | kind, Ordering::Relaxed);
    }

    fn set_squeal(&self, wheel: usize, level: f32, pitch: f32, pan: f32) {
        for (atomic, value) in self.squeal[wheel].iter().zip([level, pitch, pan]) {
            store(atomic, value);
        }
    }

    fn crash(&self, strength: f32) {
        let strength = (strength.clamp(0.0, 1.0) * 255.0).round() as u32;
        let count = (self.crash.load(Ordering::Relaxed) >> 8).wrapping_add(1);
        self.crash.store(count << 8 | strength, Ordering::Relaxed);
    }

    fn set_scrape(&self, level: f32, pan: f32) {
        store(&self.scrape[0], level);
        store(&self.scrape[1], pan);
    }

    fn set_grind(&self, level: f32, rate: f32) {
        store(&self.grind[0], level);
        store(&self.grind[1], rate);
    }
}

/// The sound effects: what the game tells the audio thread to play.
#[derive(Resource, Clone)]
pub struct Effects {
    cues: Arc<Cues>,
}

impl Effects {
    /// Plays a trumpet call.
    pub fn call(&self, call: Call) {
        self.cues.ask(call);
    }
}

#[derive(Asset, TypePath, Clone)]
struct EffectsSynth {
    cues: Arc<Cues>,
}

impl Decodable for EffectsSynth {
    type Decoder = EffectsVoice;

    fn decoder(&self) -> EffectsVoice {
        EffectsVoice::new(self.cues.clone())
    }
}

pub(super) fn build(app: &mut App) {
    app.add_audio_source::<EffectsSynth>()
        .add_systems(Startup, start)
        .add_systems(
            PostUpdate,
            cue.after(bevy::transform::TransformSystems::Propagate),
        );
}

fn start(mut commands: Commands, mut synths: ResMut<Assets<EffectsSynth>>) {
    let cues = Arc::new(Cues::default());
    store(&cues.volume, 1.0);
    commands.spawn(AudioPlayer(synths.add(EffectsSynth { cues: cues.clone() })));
    commands.insert_resource(Effects { cues });
}

/// Smooth step from 0 at `from` to 1 at `to`.
fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How loud a tire squeals (0..1) and the pitch it squeals at (times its own), from its
/// slip relative to the peak, how fast its tread slides over the road (m/s), its load
/// relative to the design load, and the ground.
fn squeal(slip: f32, sliding: f32, load: f32, surface: Surface) -> (f32, f32) {
    let ground = match surface {
        Surface::Asphalt => 1.0,
        Surface::Cobbles => 0.5,
        _ => 0.0,
    };
    let level = smoothstep(SQUEAL_FROM, SQUEAL_FULL, slip)
        * (sliding / SQUEAL_SLIDE).clamp(0.0, 1.0)
        * load.clamp(0.0, 1.5).sqrt()
        * ground;
    let pitch = 1.0 + SQUEAL_RISE * (slip - 1.0).clamp(-0.5, 1.5);
    (level, pitch)
}

/// The share of a full grind and the teeth's clash rate (Hz) for a speed difference
/// (rpm) between the gear and the shaft.
fn grind(difference: f32) -> (f32, f32) {
    let difference = difference.abs();
    let level = (difference / GRIND_FULL).clamp(GRIND_LEAST, 1.0);
    let rate = (difference / 60.0 * TEETH).clamp(CLASH_RATE.0, CLASH_RATE.1);
    (level, rate)
}

/// The strength of a crash (0..1) for a knock (m/s), if it's one.
fn crash(knock: f32) -> Option<f32> {
    (knock >= CRASH_FROM).then(|| ((knock - CRASH_FROM) / (CRASH_FULL - CRASH_FROM)).min(1.0))
}

/// How loud the body scrapes (0..1), knocked at `knocks` a second (m/s²) at `speed`.
fn scrape(knocks: f32, speed: f32) -> f32 {
    (knocks / SCRAPE_KNOCKS).min(1.0) * (speed / SCRAPE_SPEED).min(1.0)
}

/// What's going on with crashes: how long since the last one and how hard it was, the
/// knocks a second, smoothed.
#[derive(Default)]
struct Crashes {
    since: f32,
    last: f32,
    knocks: f32,
}

/// What the driven car's tires, gearbox and body do, heard from the camera.
#[allow(clippy::too_many_arguments)]
fn cue(
    time: Res<Time>,
    effects: Res<Effects>,
    engine: Res<EngineAudio>,
    active: Res<ActiveCar>,
    rescued: Res<Rescued>,
    mut cars: Query<(&mut Car, &GlobalTransform)>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut refused: MessageReader<ReverseRefused>,
    (mut burst, mut crashes): (Local<f32>, Local<Crashes>),
) {
    let cues = &effects.cues;
    let dt = time.delta_secs();
    let quiet = engine.muted || engine.hushed;
    store(&cues.volume, if engine.muted { 0.0 } else { 1.0 });
    *burst = (*burst - dt).max(0.0);
    if refused.read().count() > 0 {
        *burst = GRIND_BURST;
    }
    // Every car's knocks are taken, heard or not.
    let mut knock = 0.0;
    for (mut car, _) in &mut cars {
        if car.index == active.0 {
            knock = car.knocks;
        }
        car.knocks = 0.0;
    }
    let car = cars.iter().find(|(car, _)| car.index == active.0);
    let (Some((car, transform)), false, false) = (car, quiet, rescued.0) else {
        for wheel in 0..4 {
            cues.set_squeal(wheel, 0.0, 1.0, 0.0);
        }
        cues.set_grind(0.0, CLASH_RATE.0);
        cues.set_scrape(0.0, 0.0);
        *crashes = Crashes::default();
        return;
    };
    let (ear, right) = match cameras.single() {
        Ok(camera) => (camera.translation(), camera.right()),
        Err(_) => (transform.translation(), Dir3::X),
    };

    for (wheel, corner) in car.corners.iter().enumerate() {
        let tire = &car.setup.axle(corner.front).tire;
        let (level, pitch) = match &corner.contact {
            Some(contact) => {
                let slip = (contact.tire.slip_angle / tire.peak_angle())
                    .hypot(contact.tire.slip_ratio / tire.compound.peak_ratio);
                // How fast the tread slides over the road: the patch's own speed times
                // the slips (along: against the rolling speed; across: the slip angle).
                let rolling = corner.omega.abs() * tire.radius;
                let speed = car.body.velocity.length().max(rolling);
                let sliding = speed
                    * contact
                        .tire
                        .slip_ratio
                        .abs()
                        .min(1.0)
                        .hypot(contact.tire.slip_angle.sin().abs());
                let load = contact.load / tire.nominal_load;
                squeal(slip, sliding, load, contact.surface)
            }
            None => (0.0, 1.0),
        };
        let at = transform.transform_point(corner.pose.wheel_center);
        let pan = 0.6 * pan(at - ear, right);
        cues.set_squeal(wheel, level, pitch, pan);
    }

    // The lever held in a gear that won't go in, or a refused reverse just asked for.
    let drivetrain = &car.drivetrain;
    let gate = drivetrain.grinding.or((*burst > 0.0).then_some(-1));
    let (level, rate) = match gate {
        Some(gate) => {
            let engine = &car.setup.engine;
            let shares = engine.drive.shares();
            let wheel_omega: f32 = (0..4).map(|i| shares[i] * car.corners[i].omega).sum();
            grind(drivetrain.rpm - engine.rpm_in(gate, wheel_omega))
        }
        None => (0.0, CLASH_RATE.0),
    };
    cues.set_grind(level, rate);

    // A crash when the body is knocked hard (not again within a moment unless harder),
    // scraping while it keeps being knocked on the move.
    let body = pan(transform.translation() - ear, right) * 0.6;
    crashes.since += dt;
    if let Some(strength) = crash(knock)
        && (crashes.since > CRASH_GAP || strength > crashes.last)
    {
        cues.crash(strength);
        crashes.since = 0.0;
        crashes.last = strength;
    }
    if dt > 0.0 {
        let share = (dt / SCRAPE_GLIDE).min(1.0);
        crashes.knocks += (knock / dt - crashes.knocks) * share;
    }
    let speed = car.body.velocity.length();
    cues.set_scrape(scrape(crashes.knocks, speed), body);
}

/// One wheel's squeal.
#[derive(Default)]
struct Squeal {
    level: f32,
    pitch: f32,
    pan: f32,
    rings: [Resonator; 3],
    /// The pitch the rings are tuned to (Hz).
    tuned: f32,
    /// Wobble and flutter: random walks, smoothed.
    wobble: f32,
    flutter: f32,
    scrub: f32,
}

/// One crash sounding.
#[derive(Clone, Default)]
struct Impact {
    strength: f32,
    age: f32,
    /// The thud's phase (radians).
    thud: f32,
    /// The panels, each ringing a little differently per crash, and their gains.
    panels: [Resonator; 6],
    gains: [f32; 6],
    /// How long its crackles go on (s).
    crackling: f32,
    playing: bool,
}

/// A decaying sine (a piece of glass ringing): its state and its step, both as complex
/// numbers.
#[derive(Clone, Copy, Default)]
struct Ping {
    z: [f32; 2],
    step: [f32; 2],
}

impl Ping {
    fn next(&mut self) -> f32 {
        let [a, b] = self.z;
        let [c, d] = self.step;
        self.z = [a * c - b * d, a * d + b * c];
        self.z[1]
    }
}

/// One trumpet playing a note.
#[derive(Clone, Copy, Default)]
struct Player {
    note: f32,
    /// How long it has played (s), how long it holds (s).
    age: f32,
    length: f32,
    /// Sawtooth phase (0..1) and the low-pass's two stages.
    phase: f32,
    low: [f32; 2],
    vibrato: f32,
    playing: bool,
}

impl Player {
    fn start(note: f32, length: f32) -> Self {
        Self {
            note,
            length,
            playing: true,
            ..default()
        }
    }

    /// The next sample.
    fn next(&mut self) -> f32 {
        let dt = 1.0 / SAMPLE_RATE as f32;
        let age = self.age;
        self.age += dt;
        let envelope = if age < ATTACK {
            age / ATTACK
        } else if age < self.length {
            1.0 + OVERSHOOT.0 * (-(age - ATTACK) / OVERSHOOT.1).exp()
        } else {
            let tail = 1.0 - (age - self.length) / RELEASE;
            if tail <= 0.0 {
                self.playing = false;
                return 0.0;
            }
            tail * tail
        };
        let scoop = 1.0 - SCOOP.0 * (-age / SCOOP.1).exp();
        let vibrato = if age > VIBRATO.2 {
            let depth = VIBRATO.0 * ((age - VIBRATO.2) / 0.15).min(1.0);
            self.vibrato = (self.vibrato + VIBRATO.1 * dt).fract();
            1.0 + depth * (TAU * self.vibrato).sin()
        } else {
            1.0
        };
        let frequency = self.note * scoop * vibrato;
        let step = frequency * dt;
        self.phase += step;
        if self.phase >= 1.0 {
            self.phase -= 1.0;
        }
        // Band-limited sawtooth (polyBLEP at the wrap).
        let t = self.phase;
        let mut saw = 2.0 * t - 1.0;
        if t < step {
            let x = t / step;
            saw -= x + x - x * x - 1.0;
        } else if t > 1.0 - step {
            let x = (t - 1.0) / step;
            saw -= x * x + x + x + 1.0;
        }
        // Two one-pole low-passes, opening with the swell.
        let open = envelope.min(1.0);
        let cutoff = frequency * (BRIGHTNESS.0 + (BRIGHTNESS.1 - BRIGHTNESS.0) * open);
        let share = 1.0 - (-TAU * cutoff * dt).exp();
        self.low[0] += share * (saw - self.low[0]);
        self.low[1] += share * (self.low[0] - self.low[1]);
        envelope * self.low[1]
    }
}

/// An endless stereo stream (left and right samples interleaved).
struct EffectsVoice {
    cues: Arc<Cues>,
    volume: f32,
    squeals: [Squeal; 4],
    grind: f32,
    /// Where the clashes are (0..1 to the next), how hard the last one was, the rings.
    clash: f32,
    grind_rings: [Resonator; 4],
    /// Each ring's gain that makes a click ring at about its own level.
    grind_gains: [f32; 4],
    rattle: f32,
    players: [Player; PLAYERS],
    /// The last call counter seen.
    call: u32,
    /// Crashes sounding, glass ringing, and the last crash counter seen.
    impacts: [Impact; IMPACTS],
    pings: [Ping; RINGING],
    crash: u32,
    /// Scraping: its level and pan (gliding), bands, the bands' gains and roughness.
    scrape: f32,
    scrape_pan: f32,
    scrape_bands: [Resonator; 3],
    scrape_gains: [f32; 3],
    rough: f32,
    seed: u32,
    right: Option<f32>,
}

impl EffectsVoice {
    fn new(cues: Arc<Cues>) -> Self {
        let mut grind_rings = [Resonator::default(); 4];
        let mut grind_gains = [0.0; 4];
        for (i, &(frequency, q, gain)) in GRIND_RINGS.iter().enumerate() {
            grind_rings[i].tune(frequency, q);
            // A band-pass rings at about its `b0` from a click.
            grind_gains[i] = gain / grind_rings[i].b0;
        }
        let call = cues.call.load(Ordering::Relaxed);
        let crash = cues.crash.load(Ordering::Relaxed);
        let mut scrape_bands = [Resonator::default(); 3];
        let mut scrape_gains = [0.0; 3];
        for (i, &(frequency, q)) in SCRAPE_BANDS.iter().enumerate() {
            scrape_bands[i].tune(frequency, q);
            // Back to an RMS of 1, as the turbo's whistle.
            let band = (PI * frequency / (q * SAMPLE_RATE as f32)).sqrt() / 3f32.sqrt();
            scrape_gains[i] = 1.0 / band / SCRAPE_BANDS.len() as f32;
        }
        Self {
            cues,
            volume: 0.0,
            squeals: default(),
            grind: 0.0,
            clash: 0.0,
            grind_rings,
            grind_gains,
            rattle: 0.0,
            players: [Player::default(); PLAYERS],
            call,
            impacts: default(),
            pings: [Ping::default(); RINGING],
            crash,
            scrape: 0.0,
            scrape_pan: 0.0,
            scrape_bands,
            scrape_gains,
            rough: 0.0,
            seed: 0x9e37_79b9,
            right: None,
        }
    }

    /// Uniform noise from -1 to 1 (xorshift).
    fn noise(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        self.seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    }

    /// Uniform noise from 0 to 1.
    fn uniform(&mut self) -> f32 {
        0.5 * (self.noise() + 1.0)
    }

    /// Starts a crash of `strength` (0..1).
    fn smash(&mut self, strength: f32) {
        // A free slot, or the one that has sounded longest.
        let slot = (0..IMPACTS)
            .find(|&i| !self.impacts[i].playing)
            .or_else(|| {
                (0..IMPACTS).max_by(|&a, &b| self.impacts[a].age.total_cmp(&self.impacts[b].age))
            })
            .unwrap_or(0);
        let mut panels = [Resonator::default(); 6];
        let mut gains = [0.0; 6];
        for (i, &(frequency, q, gain)) in PANELS.iter().enumerate() {
            let frequency = frequency * (0.85 + 0.3 * self.uniform());
            panels[i].tune(frequency, q);
            // A band-pass rings at about its `b0` from a click.
            gains[i] = gain / panels[i].b0;
        }
        self.impacts[slot] = Impact {
            strength,
            age: 0.0,
            thud: 0.0,
            panels,
            gains,
            crackling: CRACKLING.0 + (CRACKLING.1 - CRACKLING.0) * strength,
            playing: true,
        };
    }

    /// Starts a piece of glass ringing.
    fn ping(&mut self, level: f32) {
        let frequency = PING_PITCH.0 * (PING_PITCH.1 / PING_PITCH.0).powf(self.uniform());
        let decay = PING_DECAY.0 + (PING_DECAY.1 - PING_DECAY.0) * self.uniform();
        let rate = SAMPLE_RATE as f32;
        let r = (-1.0 / (decay * rate)).exp();
        let w = TAU * frequency / rate;
        let amplitude = level * (0.3 + 0.7 * self.uniform());
        // The quietest (or a silent) one makes way.
        let slot = (0..RINGING)
            .min_by(|&a, &b| {
                let size = |p: &Ping| p.z[0].hypot(p.z[1]);
                size(&self.pings[a]).total_cmp(&size(&self.pings[b]))
            })
            .unwrap_or(0);
        self.pings[slot] = Ping {
            z: [amplitude, 0.0],
            step: [r * w.cos(), r * w.sin()],
        };
    }

    /// The crashes, the glass and the scraping in this sample: centred, and the scrape
    /// with its pan.
    fn crashes(&mut self) -> (f32, f32, f32) {
        let rate = SAMPLE_RATE as f32;
        let dt = 1.0 / rate;
        let crash = self.cues.crash.load(Ordering::Relaxed);
        if crash != self.crash {
            self.crash = crash;
            self.smash((crash & 0xff) as f32 / 255.0);
        }
        let mut out = 0.0;
        let mut glass = 0.0;
        for i in 0..IMPACTS {
            if !self.impacts[i].playing {
                continue;
            }
            let (noise, chance, size, ping_chance) =
                (self.noise(), self.uniform(), self.noise(), self.uniform());
            let impact = &mut self.impacts[i];
            let age = impact.age;
            impact.age += dt;
            let strength = impact.strength;
            // The thud: a boom gliding down.
            let pitch =
                THUD_PITCH.1 + (THUD_PITCH.0 - THUD_PITCH.1) * (-age / (2.0 * THUD_DECAY)).exp();
            impact.thud += TAU * pitch * dt;
            let level = 0.3 + 0.7 * strength;
            out += THUD * level.powf(0.8) * impact.thud.sin() * (-age / THUD_DECAY).exp();
            // The crunch: a burst of noise, then crackles thinning out.
            let mut excitation = 0.0;
            if age < BURST {
                excitation += 0.15 * noise * (1.0 - age / BURST);
            }
            let crackles = CRACKLES * (0.2 + 0.8 * strength) * (-age / impact.crackling).exp();
            if chance < crackles * dt {
                excitation += size;
            }
            let mut ring = 0.0;
            for (panel, gain) in impact.panels.iter_mut().zip(impact.gains) {
                ring += gain * panel.process(excitation);
            }
            out += CRUNCH * level * ring;
            let glassy =
                strength >= GLASS_FROM && ping_chance < PINGS * (-age / PINGING).exp() * dt;
            if age > 3.0 * impact.crackling + 6.0 * THUD_DECAY + 0.2 {
                impact.playing = false;
            }
            if glassy {
                glass += strength;
            }
        }
        if glass > 0.0 {
            self.ping(PING * glass.min(1.0));
        }
        for ping in &mut self.pings {
            out += ping.next();
        }

        // Scraping: rough noise in a few bands.
        let [level, pan] = self.cues.scrape.each_ref().map(read);
        self.scrape += (level.clamp(0.0, 1.0) - self.scrape) / (0.02 * rate);
        self.scrape_pan += (pan.clamp(-1.0, 1.0) - self.scrape_pan) / (0.05 * rate);
        let (noise, walk) = (self.noise(), self.noise());
        let mut scrape = 0.0;
        if self.scrape > 1e-4 {
            self.rough += (walk * 3.0 - self.rough) * TAU * SCRAPE_ROUGH.1 / rate;
            for (band, gain) in self.scrape_bands.iter_mut().zip(self.scrape_gains) {
                scrape += gain * band.process(noise);
            }
            let rough = (1.0 + SCRAPE_ROUGH.0 * self.rough.clamp(-1.0, 1.0)).max(0.0);
            scrape *= SCRAPE * self.scrape * rough;
        }
        (out, scrape, self.scrape_pan)
    }

    /// Starts the notes of a call.
    fn play(&mut self, call: Call) {
        let (notes, length): (&[f32], f32) = match call {
            Call::Count => (&[COUNT_NOTE], COUNT_LENGTH),
            Call::Go => (&GO_CHORD, GO_LENGTH),
        };
        for &note in notes {
            for detune in [1.0 - DETUNE, 1.0 + DETUNE] {
                // A free trumpet, or the one that has played longest.
                let slot = (0..PLAYERS)
                    .find(|&i| !self.players[i].playing)
                    .or_else(|| {
                        (0..PLAYERS)
                            .max_by(|&a, &b| self.players[a].age.total_cmp(&self.players[b].age))
                    })
                    .unwrap_or(0);
                self.players[slot] = Player::start(note * detune, length);
            }
        }
    }

    /// Advances by one sample, returning the left and right outputs.
    fn frame(&mut self) -> (f32, f32) {
        let rate = SAMPLE_RATE as f32;
        let glide = |value: &mut f32, target: f32, time: f32| {
            *value += (target - *value) / (time * rate);
        };
        glide(
            &mut self.volume,
            read(&self.cues.volume).clamp(0.0, 1.0),
            0.05,
        );

        let call = self.cues.call.load(Ordering::Relaxed);
        if call != self.call {
            self.call = call;
            self.play(if call & 0xf == 1 {
                Call::Go
            } else {
                Call::Count
            });
        }

        let (mut left, mut right) = (0.0, 0.0);
        for wheel in 0..4 {
            let [level, pitch, pan] = self.cues.squeal[wheel].each_ref().map(read);
            let walk = [self.noise(), self.noise(), self.noise()];
            let scrub_noise = self.noise();
            let excitation = self.noise();
            let squeal = &mut self.squeals[wheel];
            let (pitch, pan) = (pitch.clamp(0.5, 2.0), pan.clamp(-1.0, 1.0));
            if squeal.level < 1e-4 {
                // Silent: it starts where it is, at its pitch.
                (squeal.pitch, squeal.pan) = (pitch, pan);
            }
            glide(&mut squeal.level, level.clamp(0.0, 1.0), SQUEAL_GLIDE);
            glide(&mut squeal.pitch, pitch, SQUEAL_GLIDE);
            glide(&mut squeal.pan, pan, SQUEAL_GLIDE);
            if squeal.level < 1e-4 {
                continue;
            }
            // Random walks pulled back to the middle: a wobble and a flutter.
            squeal.wobble += (walk[0] * 3.0 - squeal.wobble) * TAU * SQUEAL_WOBBLE.1 / rate;
            squeal.flutter += (walk[1] * 3.0 - squeal.flutter) * TAU * SQUEAL_FLUTTER.1 / rate;
            let frequency = SQUEAL_PITCH
                * SQUEAL_DETUNE[wheel]
                * squeal.pitch
                * (1.0 + SQUEAL_WOBBLE.0 * squeal.wobble);
            if (frequency / squeal.tuned.max(1.0) - 1.0).abs() > 0.002 {
                for (ring, &(overtone, _)) in squeal.rings.iter_mut().zip(&SQUEAL_OVERTONES) {
                    ring.tune(frequency * overtone, SQUEAL_Q);
                }
                squeal.tuned = frequency;
            }
            let mut tone = 0.0;
            for (ring, &(overtone, gain)) in squeal.rings.iter_mut().zip(&SQUEAL_OVERTONES) {
                // Back to an RMS of 1 whatever the band's width (as the turbo's whistle).
                let band = (PI * frequency * overtone / (SQUEAL_Q * rate)).sqrt() / 3f32.sqrt();
                tone += gain * ring.process(excitation) / band;
            }
            let share = 1.0 - (-TAU * SCRUB.1 / rate).exp();
            squeal.scrub += share * (scrub_noise - squeal.scrub);
            let flutter = (1.0 + SQUEAL_FLUTTER.0 * squeal.flutter.clamp(-1.0, 1.0)).max(0.0);
            let out = SQUEAL * squeal.level * flutter * (tone + SCRUB.0 * 3.0 * squeal.scrub);
            let angle = (squeal.pan + 1.0) * PI / 4.0;
            left += out * angle.cos() * 2f32.sqrt();
            right += out * angle.sin() * 2f32.sqrt();
        }

        // Grinding: clicks at the clash rate, jittered, ringing the gearbox.
        let [level, clash_rate] = self.cues.grind.each_ref().map(read);
        glide(&mut self.grind, level.clamp(0.0, 1.0), GRIND_GLIDE);
        let jitter = self.noise();
        let strength = self.noise();
        let rattle_noise = self.noise();
        let hiss = self.noise();
        let mut click = 0.0;
        if self.grind > 1e-4 {
            let clash_rate = clash_rate.clamp(CLASH_RATE.0, CLASH_RATE.1);
            self.clash += clash_rate * (1.0 + 0.4 * jitter) / rate;
            if self.clash >= 1.0 {
                self.clash -= 1.0;
                click = 0.6 + 0.4 * strength;
            }
            self.rattle += (rattle_noise * 3.0 - self.rattle) * TAU * GRIND_RATTLE.1 / rate;
        }
        let excitation = click + GRIND_HISS * hiss;
        let mut ring = 0.0;
        for (resonator, gain) in self.grind_rings.iter_mut().zip(self.grind_gains) {
            ring += gain * resonator.process(excitation);
        }
        let rattle = (1.0 + GRIND_RATTLE.0 * self.rattle.clamp(-1.0, 1.0)).max(0.0);
        let grind = GRIND * self.grind * rattle * ring;

        let (crashes, scrape, scrape_pan) = self.crashes();
        let angle = (scrape_pan + 1.0) * PI / 4.0;
        left += crashes + scrape * angle.cos() * 2f32.sqrt();
        right += crashes + scrape * angle.sin() * 2f32.sqrt();

        let mut brass = 0.0;
        for player in &mut self.players {
            if player.playing {
                brass += TRUMPET * player.next();
            }
        }

        let volume = self.volume;
        (
            (volume * (left + grind + brass)).tanh(),
            (volume * (right + grind + brass)).tanh(),
        )
    }
}

impl Iterator for EffectsVoice {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        if let Some(right) = self.right.take() {
            return Some(right);
        }
        let (left, right) = self.frame();
        self.right = Some(right);
        Some(left)
    }
}

impl Source for EffectsVoice {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        NonZero::new(2).unwrap()
    }

    fn sample_rate(&self) -> SampleRate {
        NonZero::new(SAMPLE_RATE).unwrap()
    }

    fn total_duration(&self) -> Option<Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `seconds` of the synth, returning both channels' RMS and peak.
    fn run(voice: &mut EffectsVoice, seconds: f32) -> (f32, f32) {
        let samples = (seconds * SAMPLE_RATE as f32) as usize;
        let (mut sum, mut peak) = (0.0, 0.0f32);
        for _ in 0..samples {
            let (left, right) = voice.frame();
            assert!(left.is_finite() && right.is_finite());
            assert!(left.abs() <= 1.0 && right.abs() <= 1.0);
            sum += left * left + right * right;
            peak = peak.max(left.abs()).max(right.abs());
        }
        ((sum / (2 * samples) as f32).sqrt(), peak)
    }

    fn voice() -> (Arc<Cues>, EffectsVoice) {
        let cues = Arc::new(Cues::default());
        store(&cues.volume, 1.0);
        let voice = EffectsVoice::new(cues.clone());
        (cues, voice)
    }

    #[test]
    fn silent_until_something_happens() {
        let (_, mut voice) = voice();
        assert_eq!(run(&mut voice, 0.5).1, 0.0);
    }

    #[test]
    fn tires_squeal_only_sliding_on_a_road() {
        // Gripping, or rolling past the peak barely moving: nothing.
        assert_eq!(squeal(0.3, 5.0, 1.0, Surface::Asphalt).0, 0.0);
        assert_eq!(squeal(1.5, 0.0, 1.0, Surface::Asphalt).0, 0.0);
        // Sliding on grass: nothing; on asphalt, all out; cobbles in between.
        assert_eq!(squeal(1.5, 5.0, 1.0, Surface::Grass).0, 0.0);
        let (full, pitch) = squeal(1.5, 5.0, 1.0, Surface::Asphalt);
        assert!(full > 0.99 && pitch > 1.0, "{full} {pitch}");
        let cobbles = squeal(1.5, 5.0, 1.0, Surface::Cobbles).0;
        assert!(cobbles > 0.2 && cobbles < full);
        // Near the peak it's starting; a lightly loaded tire is quieter.
        let near = squeal(0.9, 5.0, 1.0, Surface::Asphalt).0;
        assert!(near > 0.2 && near < full);
        assert!(squeal(1.5, 5.0, 0.3, Surface::Asphalt).0 < 0.6 * full);

        // Heard, and from the side it's on.
        let (cues, mut voice) = voice();
        cues.set_squeal(1, 1.0, 1.0, 1.0);
        cues.set_squeal(3, 1.0, 1.1, 1.0);
        let samples = SAMPLE_RATE as usize;
        let (mut left, mut right) = (0.0f32, 0.0f32);
        for _ in 0..samples {
            let (l, r) = voice.frame();
            left = left.max(l.abs());
            right = right.max(r.abs());
        }
        assert!(right > 0.1 && left < 0.01 * right, "{left} {right}");
        let (loud, _) = run(&mut voice, 1.0);
        assert!(loud > 0.03, "{loud}");
        // It stops when the tires grip again.
        cues.set_squeal(1, 0.0, 1.0, 1.0);
        cues.set_squeal(3, 0.0, 1.0, 1.0);
        run(&mut voice, 0.5);
        assert!(run(&mut voice, 0.2).1 < 1e-3);
    }

    #[test]
    fn gears_grind_harder_the_more_they_differ() {
        let (least, slow) = grind(0.0);
        let (most, fast) = grind(4000.0);
        assert_eq!(least, GRIND_LEAST);
        assert_eq!(most, 1.0);
        assert_eq!(slow, CLASH_RATE.0);
        assert_eq!(fast, CLASH_RATE.1);
        assert_eq!(grind(-2000.0), grind(2000.0));

        let (cues, mut voice) = voice();
        cues.set_grind(1.0, 400.0);
        let (loud, peak) = run(&mut voice, 0.5);
        assert!(loud > 0.03 && peak > 0.1, "{loud} {peak}");
        cues.set_grind(0.0, 400.0);
        run(&mut voice, 0.2);
        assert!(run(&mut voice, 0.2).1 < 1e-2);
    }

    #[test]
    fn crashes_thud_crunch_and_shatter() {
        // Bumps and resting against something are no crash; a hard one is the hardest.
        assert_eq!(crash(0.5), None);
        assert!(crash(3.0).unwrap() < GLASS_FROM);
        assert_eq!(crash(30.0), Some(1.0));
        // Scraping needs both pushing and moving.
        assert_eq!(scrape(20.0, 0.0), 0.0);
        assert_eq!(scrape(0.0, 20.0), 0.0);
        assert_eq!(scrape(20.0, 20.0), 1.0);

        let (cues, mut voice) = voice();
        cues.crash(0.2);
        let (light, _) = run(&mut voice, 0.5);
        assert!(voice.pings.iter().all(|p| p.z == [0.0; 2]), "no glass");
        run(&mut voice, 2.0);
        assert!(voice.impacts.iter().all(|i| !i.playing));
        assert!(run(&mut voice, 0.2).1 < 1e-3);
        cues.crash(1.0);
        let (hard, _) = run(&mut voice, 0.5);
        assert!(hard > 1.5 * light && light > 0.02, "{light} {hard}");
        assert!(voice.pings.iter().any(|p| p.z != [0.0; 2]), "glass");
        run(&mut voice, 2.0);
        assert!(run(&mut voice, 0.2).1 < 1e-3);
        // Several at once (more than it plays) are fine.
        for i in 0..10 {
            cues.crash(i as f32 / 10.0);
            run(&mut voice, 0.01);
        }
        run(&mut voice, 0.5);
        // Scraping: heard while it goes on.
        cues.set_scrape(1.0, -1.0);
        let samples = SAMPLE_RATE as usize;
        run(&mut voice, 3.0);
        let (mut left, mut right) = (0.0f32, 0.0f32);
        for _ in 0..samples {
            let (l, r) = voice.frame();
            left = left.max(l.abs());
            right = right.max(r.abs());
        }
        assert!(left > 0.1 && right < 0.01 * left, "{left} {right}");
        cues.set_scrape(0.0, -1.0);
        run(&mut voice, 0.5);
        assert!(run(&mut voice, 0.2).1 < 1e-3);
    }

    #[test]
    fn the_trumpet_plays_its_calls_and_stops() {
        let (cues, mut voice) = voice();
        let effects = Effects { cues: cues.clone() };
        effects.call(Call::Count);
        let (count, _) = run(&mut voice, COUNT_LENGTH);
        assert!(count > 0.03, "{count}");
        // Over after its release.
        run(&mut voice, RELEASE);
        assert!(run(&mut voice, 0.1).1 < 1e-4);
        // "Go!" is a chord: louder, and longer.
        effects.call(Call::Go);
        let (go, _) = run(&mut voice, COUNT_LENGTH);
        assert_eq!(voice.players.iter().filter(|p| p.playing).count(), 8);
        assert!(go > count, "{go} {count}");
        assert!(run(&mut voice, GO_LENGTH - COUNT_LENGTH).0 > 0.03);
        run(&mut voice, RELEASE);
        assert!(run(&mut voice, 0.1).1 < 1e-4);
        // Muted: silent once the volume has faded (it fades over 0.05 s).
        store(&cues.volume, 0.0);
        effects.call(Call::Go);
        run(&mut voice, 0.5);
        assert!(run(&mut voice, 0.2).1 < 1e-3);
    }

    /// Writes a few seconds of each effect to `EFFECTS_WAV` (a 16-bit stereo WAV), for
    /// listening: a countdown, a slide that builds up and lets go, a grind, crashes and a
    /// scrape.
    #[test]
    #[ignore]
    fn write_effects_wav() {
        let path = std::env::var("EFFECTS_WAV").unwrap_or("effects.wav".into());
        let (cues, mut voice) = voice();
        let effects = Effects { cues: cues.clone() };
        let mut samples: Vec<i16> = Vec::new();
        let mut play = |voice: &mut EffectsVoice, seconds: f32| {
            for _ in 0..(seconds * SAMPLE_RATE as f32) as usize {
                let (left, right) = voice.frame();
                samples.push((left * 32767.0) as i16);
                samples.push((right * 32767.0) as i16);
            }
        };
        for _ in 0..3 {
            effects.call(Call::Count);
            play(&mut voice, 1.0);
        }
        effects.call(Call::Go);
        play(&mut voice, 2.0);
        // A slide: the rear wheels building up past the peak, then gripping again.
        for step in 0..30 {
            let slip = 0.4 + step as f32 * 0.04;
            for wheel in [2, 3] {
                let (level, pitch) = squeal(slip, 6.0, 1.1, Surface::Asphalt);
                cues.set_squeal(wheel, level, pitch, if wheel == 2 { -0.5 } else { 0.5 });
            }
            play(&mut voice, 0.1);
        }
        for wheel in [2, 3] {
            cues.set_squeal(wheel, 0.0, 1.0, 0.0);
        }
        play(&mut voice, 1.0);
        // A grind, slowly and then far apart.
        for difference in [300.0, 2500.0] {
            let (level, rate) = grind(difference);
            cues.set_grind(level, rate);
            play(&mut voice, 1.0);
            cues.set_grind(0.0, rate);
            play(&mut voice, 0.5);
        }
        // Crashes: a bump, a solid knock, a big one with glass; then scraping along.
        for strength in [0.1, 0.4, 1.0] {
            cues.crash(strength);
            play(&mut voice, 1.5);
        }
        cues.set_scrape(0.8, 0.3);
        play(&mut voice, 1.5);
        cues.set_scrape(0.0, 0.3);
        play(&mut voice, 0.5);
        let mut wav = Vec::new();
        let data = (samples.len() * 2) as u32;
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
        wav.extend_from_slice(&(SAMPLE_RATE * 4).to_le_bytes());
        wav.extend_from_slice(&4u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data.to_le_bytes());
        for sample in samples {
            wav.extend_from_slice(&sample.to_le_bytes());
        }
        std::fs::write(path, wav).unwrap();
    }
}
