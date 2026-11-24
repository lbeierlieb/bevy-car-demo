//! Engine sound for the active car and the cars around it, synthesised sample by sample from its exhaust pulses.
//! Each firing sends a pulse down its bank's exhaust; the engine's layout says how many
//! there are in a cycle and which bank each goes down. A cross-plane V8 fires each bank at
//! uneven intervals (the burble), a flat four's unequal headers bunch its pulses up in
//! pairs (the boxer rumble), and the others fire evenly, the more cylinders the smoother.
//! The pulses run through the header, the front pipe and the tailpipe, each a delay line
//! ringing a series of resonances (a wave to the far end and back, partly reflected), which
//! the engine's orders sweep through as it revs, so its harmonics swell and fade; then
//! they ring a few broad resonances of the exhaust, with a little combustion noise on top. Two banks with an
//! exhaust each are panned apart.
//!
//! The exhaust hardware colours it: the silencer and the catalyst take out level and the
//! higher resonances (a straight pipe is loud and rasping), a bigger bore rings a little
//! deeper and louder, the header primaries add a resonance of their own (short ones ring
//! higher), and a turbine in the way smears the pulses and quietens them. A turbo whistles
//! with its shaft speed, louder as it spools under load.
//!
//! The cars around (other players' and traffic) are heard too, the nearest few within
//! 150 m, each its own sound: one source panned to where it is from the camera, quieter
//! and duller further away, shifted by the Doppler effect as it closes in or pulls away.
//! What their engines do comes with their latest state (on a server, a network delay
//! late, as real sound is late by its travel), never predicted.
//!
//! The game only publishes targets (engine speed, load, volume, the exhaust, turbo speed);
//! the synth glides towards them every sample, so nothing steps audibly.

use std::f32::consts::{PI, TAU};
use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bevy::audio::{AddAudioSource, ChannelCount, Decodable, SampleRate, Source};
use bevy::prelude::*;

use crate::car::{ActiveCar, Car};
use crate::looks::{Looks, Motion};
use crate::physics::drivetrain::{Drivetrain, EngineSetup};
use crate::physics::engine::families::{Layout, relative_size};
use crate::physics::engine::hardware::EngineHardware;

pub mod effects;

const SAMPLE_RATE: u32 = 44_100;
/// A cross-plane V8's firing order 1-8-4-3-6-5-7-2, 90° of crank apart, with the odd
/// cylinders on the left bank (0): each bank fires at 90°, 180°, 270° and 180° intervals.
const CROSS_PLANE: [u8; 8] = [0, 1, 1, 0, 1, 0, 0, 1];
/// Banks firing in turn, as in every other engine with two.
const ALTERNATING: [u8; 12] = [0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1];
/// A flat four fires both cylinders of a bank one after the other.
const BOXER: [u8; 4] = [0, 0, 1, 1];
const ONE_BANK: [u8; 6] = [0; 6];
/// The layouts by the number the game publishes.
const LAYOUTS: [Layout; 10] = [
    Layout::Inline3,
    Layout::Inline4,
    Layout::Flat4,
    Layout::Inline6,
    Layout::Flat6,
    Layout::V6,
    Layout::V8,
    Layout::FlatPlaneV8,
    Layout::V10,
    Layout::V12,
];

/// How a layout sounds.
struct Sound {
    /// The bank each firing of a cycle (two revolutions) goes down, evenly spaced.
    banks: &'static [u8],
    /// The banks have an exhaust each (panned apart), rather than joining into one.
    dual: bool,
    /// The exhaust's resonances relative to the big V8's: smaller, shorter pipes ring
    /// higher.
    pipes: f32,
    /// How much later the second bank's pulses arrive (seconds), through longer pipes.
    delay: f32,
}

fn sound(layout: Layout) -> Sound {
    let (banks, dual, pipes, delay): (&[u8], _, _, _) = match layout {
        Layout::Inline3 => (&ONE_BANK[..3], false, 1.3, 0.0),
        Layout::Inline4 => (&ONE_BANK[..4], false, 1.2, 0.0),
        // Unequal headers into one exhaust.
        Layout::Flat4 => (&BOXER, false, 1.0, 0.004),
        Layout::Inline6 => (&ONE_BANK, false, 1.25, 0.0),
        Layout::Flat6 => (&ALTERNATING[..6], true, 1.3, RIGHT_DELAY),
        Layout::V6 => (&ALTERNATING[..6], true, 1.15, RIGHT_DELAY),
        Layout::V8 => (&CROSS_PLANE, true, 1.0, RIGHT_DELAY),
        Layout::FlatPlaneV8 => (&ALTERNATING[..8], true, 1.4, RIGHT_DELAY),
        Layout::V10 => (&ALTERNATING[..10], true, 1.5, RIGHT_DELAY),
        Layout::V12 => (&ALTERNATING, true, 1.55, RIGHT_DELAY),
    };
    Sound {
        banks,
        dual,
        pipes,
        delay,
    }
}
/// Time constants of the glide towards new targets.
const GLIDE: f32 = 0.05;
const LOAD_GLIDE: f32 = 0.02;
/// An exhaust pulse in crank degrees: the blowdown rises quickly as the valve opens, then
/// falls more slowly. So it's long at idle (full, round) and short near the redline. The
/// rise is a sharp front at full throttle, the flow through the opening valve choked by
/// the cylinder's pressure, and that front is what carries the upper harmonics.
const PULSE_RISE: f32 = 20.0;
const PULSE_SHARP: f32 = 4.0;
const PULSE_FALL: f32 = 70.0;
/// Shortest the rise may get (seconds), a few samples.
const PULSE_SHORTEST: f32 = 0.00012;
/// How fast the combustion noise after a pulse dies (seconds).
const NOISE_DECAY: f32 = 0.004;
/// Random variation of each firing's strength, from cycle to cycle: most at idle, where
/// combustion is least steady, least at full throttle.
const JITTER_IDLE: f32 = 0.15;
const JITTER_LOAD: f32 = 0.05;
/// Each cylinder's own character, the same every cycle (filling, runner and header
/// lengths differ a little): its pulse's strength, and how late it arrives, in firing
/// intervals. Repeating once a cycle, it's what gives the half orders (the lope).
const CYLINDER_STRENGTH: f32 = 0.12;
const CYLINDER_LATE: f32 = 0.04;
/// Per cylinder in firing order, -1 to 1 (made up, fixed, so every engine of a layout
/// sounds the same).
const CYLINDERS: [(f32, f32); 12] = [
    (0.6, -0.3),
    (-0.9, 0.5),
    (0.2, 0.9),
    (-0.3, -0.8),
    (1.0, 0.1),
    (-0.5, -0.4),
    (0.4, 0.7),
    (-0.1, -1.0),
    (0.8, 0.3),
    (-0.7, -0.1),
    (0.1, 0.6),
    (-0.2, -0.6),
];
/// Exhaust resonances: frequency (Hz), Q, gain off throttle and at full throttle. Load
/// brings out the higher ones (a harder, rasping note).
const RESONANCES: [(f32, f32, f32, f32); 4] = [
    (95.0, 1.0, 1.0, 1.3),
    (240.0, 2.0, 0.35, 0.8),
    (620.0, 2.5, 0.1, 0.45),
    (1800.0, 1.5, 0.02, 0.15),
];
/// The right bank's pipes are a little shorter, so its resonances sit higher, and its
/// pulses arrive later (seconds; a longer way round to the tail pipe). With two identical
/// banks their sum (what a mono speaker plays) would be an even pulse train, without the
/// burble.
const RIGHT_PIPES: f32 = 1.15;
const RIGHT_DELAY: f32 = 0.001;
/// Share of each bank that reaches the other ear.
const CROSSFEED: f32 = 0.5;
/// Strength of a firing off throttle, relative to full throttle.
const IDLE_STRENGTH: f32 = 0.45;
/// Overall level before the soft limiter.
const GAIN: f32 = 1.5;

/// How much a stock silencer and a stock catalyst muffle (0..1); less restrictive ones
/// muffle less, with the power of their relative restriction.
const SILENCER: f32 = 0.85;
const CATALYST: f32 = 0.25;
const MUFFLER_POWER: f32 = 1.5;
/// Share of each resonance (lowest first) and of the header's that full muffling takes out.
const MUFFLED: [f32; 5] = [0.1, 0.4, 0.75, 0.9, 0.5];
/// Level fully muffled and open, and the combustion noise's level open (relative to its
/// level fully muffled).
const MUFFLED_LEVEL: f32 = 0.7;
const OPEN_LEVEL: f32 = 1.3;
const OPEN_NOISE: f32 = 2.0;
/// The header primaries' quarter-wave resonance: speed of sound in the hot exhaust (m/s),
/// Q, and gain off and at full throttle.
const EXHAUST_SOUND_SPEED: f32 = 500.0;
const HEADER: (f32, f32, f32) = (3.0, 0.25, 0.5);
/// The pipes a pulse runs through, as in the big V8 (shorter in smaller engines, `pipes`):
/// from the header's collector to the silencer (or catalyst), and the tailpipe after it
/// (m), with the speed of sound in the cooler gas there (m/s). Each rings a series of
/// resonances (a wave to the far end and back, partly reflected), which the engine's
/// orders sweep through as it revs: they swell and fade.
const FRONT_PIPE: f32 = 1.8;
const TAIL_PIPE: f32 = 0.8;
const TAIL_SOUND_SPEED: f32 = 420.0;
/// What comes back from each pipe's far end (inverted: an opening): from the collector
/// to the header; from the silencer to the front pipe, little without one (straight
/// pipe) and much with a restrictive one; from the open end of the tailpipe, more without
/// a silencer to soak it up.
const HEADER_REFLECTION: f32 = 0.5;
const FRONT_REFLECTION: (f32, f32) = (0.2, 0.6);
const TAIL_REFLECTION: (f32, f32) = (0.8, 0.5);
/// The pipes' losses: what comes back loses its highs above this (Hz, × the bore: wider
/// pipes lose less), fully muffled and open (a silencer's packing soaks up the highs).
const PIPE_LOSS: (f32, f32) = (3000.0, 6000.0);
/// What leaves the tailpipe above `BRIGHT_SPLIT` (Hz) besides the broad resonances, fully
/// muffled and open, off throttle and at full throttle: the rasp the silencer takes out.
const BRIGHT_SPLIT: f32 = 1000.0;
const BRIGHT: (f32, f32) = (0.4, 1.0);
const BRIGHT_IDLE: f32 = 0.5;
/// Level after the pipes, which ring up to a few times louder at their resonances, so the
/// engine is about as loud as without them.
const PIPE_LEVEL: f32 = 0.22;
/// Longest way to a pipe's end and back (samples), at the lowest Doppler factor.
const TUBE: usize = 1024;
/// How much louder a bigger pipe is per relative diameter, and how its resonances move
/// (frequency ∝ diameter^-this).
const BORE_LEVEL: f32 = 0.3;
const BORE_PITCH: f32 = 0.25;
/// A turbine in the exhaust: the time constant its smearing of the pulses adds (seconds),
/// and the level that gets through.
const TURBINE_SMEAR: f32 = 0.0012;
const TURBINE_LEVEL: f32 = 0.75;
/// Turbo whistle, a band of noise rather than a tone: its centre per shaft revolution per
/// second, its Q (lower is airier, higher more of a pitch), and its root mean square level
/// at the shaft's speed limit (at full load; `WHISTLE_IDLE` of it off throttle).
const WHISTLE_PITCH: f32 = 1.0;
const WHISTLE_Q: f32 = 4.0;
const WHISTLE: f32 = 0.007;
const WHISTLE_IDLE: f32 = 0.3;
/// Lowest and highest centre of the whistle (Hz).
const WHISTLE_MIN: f32 = 100.0;
const WHISTLE_MAX: f32 = 6000.0;

/// Other cars are heard up to this far (m), fading out over the last fifth, and a car
/// starts being heard from a little nearer (so one at the edge doesn't come and go).
const HEARING: f32 = 150.0;
const HEARD_FROM: f32 = 0.9;
/// At most this many other cars at once, the nearest.
const VOICES: usize = 6;
/// Another car's engine is as loud as our own at this distance (m, about the chase
/// camera's), and falls off with the distance beyond.
const NEAR: f32 = 6.0;
/// Further away an engine sounds duller: a low-pass falling from this (Hz) at `NEAR` with
/// the square root of the distance.
const NEAR_CUTOFF: f32 = 10_000.0;
/// The speed of sound (m/s), for the Doppler shift, which is capped for jumps.
const SOUND_SPEED: f32 = 343.0;
const MAX_CLOSING: f32 = 100.0;
/// Time constant of the closing speed's smoothing (s): the distance changes in steps with
/// every frame.
const CLOSING_SMOOTHING: f32 = 0.1;
/// A voice no longer needed fades out over this long (s) before it stops.
const FADE: f32 = 0.2;

/// What an engine sounds like: all the synth needs that doesn't change while it runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EngineCharacter {
    pub layout: Layout,
    pub exhaust: ExhaustSound,
    /// The turbo's shaft speed limit (rad/s), 0 without a turbo.
    pub turbo_limit: f32,
}

impl EngineCharacter {
    pub fn of(setup: &EngineSetup) -> Self {
        let hardware = &setup.build.hardware;
        Self {
            layout: setup.parts.family().layout,
            exhaust: ExhaustSound::of(hardware),
            turbo_limit: hardware.turbo.map_or(0.0, |turbo| turbo.max_speed()),
        }
    }
}

/// What an engine is doing right now, as far as it's heard.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EngineRun {
    pub running: bool,
    pub rpm: f32,
    /// 0 to 1.
    pub load: f32,
    /// The turbo's shaft speed (rad/s).
    pub turbo_speed: f32,
}

impl EngineRun {
    pub fn of(drivetrain: &Drivetrain) -> Self {
        Self {
            // Cranking, its compressions are heard (slow chugs at the starter's speed).
            running: drivetrain.running || drivetrain.cranking(),
            rpm: drivetrain.rpm,
            load: drivetrain.load,
            turbo_speed: drivetrain.engine.turbo_speed,
        }
    }
}

/// What the exhaust hardware does to the sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExhaustSound {
    /// How much the silencer and the catalyst take out (0 open, 1 fully muffled).
    pub muffle: f32,
    /// Pipe diameter relative to the stock one for the engine's size.
    pub bore: f32,
    /// The header primaries' quarter-wave resonance (Hz).
    pub header: f32,
    /// 1 with a turbine in the exhaust, 0 without.
    pub turbine: f32,
}

impl ExhaustSound {
    fn of(hardware: &EngineHardware) -> Self {
        // The parts are sized with the engine: compare them with the stock ones for its
        // size.
        let size = relative_size(hardware);
        let stock = EngineHardware::default().exhaust;
        let exhaust = &hardware.exhaust;
        let muffling =
            |area: Option<f32>, stock: Option<f32>, stock_muffling: f32| match (area, stock) {
                (Some(area), Some(stock)) => {
                    stock_muffling
                        * (stock * size / area)
                            .powf(MUFFLER_POWER)
                            .min(1.0 / stock_muffling)
                }
                _ => 0.0,
            };
        let silencer = muffling(exhaust.silencer_area, stock.silencer_area, SILENCER);
        let catalyst = muffling(exhaust.cat_area, stock.cat_area, CATALYST);
        Self {
            muffle: 1.0 - (1.0 - silencer) * (1.0 - catalyst),
            bore: (exhaust.pipe_area / (stock.pipe_area * size)).sqrt(),
            header: EXHAUST_SOUND_SPEED / (4.0 * exhaust.header_length.max(0.05)),
            turbine: if hardware.turbo.is_some() { 1.0 } else { 0.0 },
        }
    }
}

impl Default for ExhaustSound {
    fn default() -> Self {
        Self::of(&EngineHardware::default())
    }
}

pub struct AudioPlugin;

/// The sound on or off and its volume, set in the central menu and kept between runs
/// (`~/.config/car-sim/settings/sound.cfg`).
#[derive(Resource, Clone, Copy, PartialEq, Debug)]
pub struct SoundSettings {
    pub on: bool,
    /// 0 to 1.
    pub volume: f32,
}

impl Default for SoundSettings {
    fn default() -> Self {
        Self {
            on: true,
            volume: 1.0,
        }
    }
}

impl SoundSettings {
    /// What everything is played at.
    pub fn master(&self) -> f32 {
        if self.on { self.volume } else { 0.0 }
    }

    fn load() -> Self {
        let mut sound = Self::default();
        for line in crate::car::SETTINGS
            .load("sound")
            .unwrap_or_default()
            .lines()
        {
            match line.split_once(' ') {
                Some(("on", on)) => sound.on = on.trim().parse().unwrap_or(sound.on),
                Some(("volume", volume)) => {
                    if let Ok(volume) = volume.trim().parse::<f32>()
                        && volume.is_finite()
                    {
                        sound.volume = volume.clamp(0.0, 1.0);
                    }
                }
                _ => {}
            }
        }
        sound
    }
}

fn save_sound(sound: Res<SoundSettings>) {
    if sound.is_changed() && !sound.is_added() {
        let text = format!("on {}\nvolume {}\n", sound.on, sound.volume);
        if let Err(e) = crate::car::SETTINGS.save("sound", &text) {
            warn!("Couldn't save the sound settings: {e}");
        }
    }
}

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<EngineSynth>()
            .insert_resource(SoundSettings::load())
            .add_systems(Startup, start_engine)
            .add_systems(
                Update,
                (update_engine.after(crate::looks::show_cars), save_sound),
            )
            // Where everything ended up this frame, the camera included.
            .add_systems(
                PostUpdate,
                hear_others.after(bevy::transform::TransformSystems::Propagate),
            );
        effects::build(app);
    }
}

/// Targets shared between the game and the audio thread (f32 bits in atomics).
#[derive(Default)]
struct Targets {
    rpm: AtomicU32,
    load: AtomicU32,
    volume: AtomicU32,
    /// The engine's layout, as its place in `LAYOUTS`.
    layout: AtomicU32,
    /// `ExhaustSound`'s fields.
    exhaust: [AtomicU32; 4],
    /// Turbo shaft speed (revolutions per second), and that relative to its limit.
    turbo: [AtomicU32; 2],
    /// Where another car is heard from: the pan (-1 left to 1 right), the low-pass's
    /// cutoff (Hz; 0 for our own car: stereo, unfiltered) and the Doppler factor.
    place: [AtomicU32; 3],
}

fn store(atomic: &AtomicU32, value: f32) {
    atomic.store(value.to_bits(), Ordering::Relaxed);
}

fn read(atomic: &AtomicU32) -> f32 {
    f32::from_bits(atomic.load(Ordering::Relaxed))
}

impl Targets {
    fn new() -> Self {
        let targets = Self::default();
        targets.set_exhaust(ExhaustSound::default());
        targets.set_place(0.0, 0.0, 1.0);
        targets
    }

    fn set_place(&self, pan: f32, cutoff: f32, doppler: f32) {
        for (atomic, value) in self.place.iter().zip([pan, cutoff, doppler]) {
            store(atomic, value);
        }
    }

    fn place(&self) -> (f32, f32, f32) {
        let [pan, cutoff, doppler] = self.place.each_ref().map(read);
        (pan, cutoff, doppler)
    }

    fn set(&self, rpm: f32, load: f32, volume: f32) {
        store(&self.rpm, rpm);
        store(&self.load, load);
        store(&self.volume, volume);
    }

    fn set_exhaust(&self, sound: ExhaustSound) {
        let values = [sound.muffle, sound.bore, sound.header, sound.turbine];
        for (atomic, value) in self.exhaust.iter().zip(values) {
            store(atomic, value);
        }
    }

    fn exhaust(&self) -> ExhaustSound {
        let [muffle, bore, header, turbine] = self.exhaust.each_ref().map(read);
        ExhaustSound {
            muffle,
            bore,
            header,
            turbine,
        }
    }

    fn set_turbo(&self, revolutions: f32, share: f32) {
        store(&self.turbo[0], revolutions);
        store(&self.turbo[1], share);
    }

    fn turbo(&self) -> (f32, f32) {
        (read(&self.turbo[0]), read(&self.turbo[1]))
    }

    fn set_layout(&self, layout: Layout) {
        let index = LAYOUTS.iter().position(|&l| l == layout).unwrap_or(0);
        self.layout.store(index as u32, Ordering::Relaxed);
    }

    fn layout(&self) -> Layout {
        LAYOUTS[self.layout.load(Ordering::Relaxed) as usize % LAYOUTS.len()]
    }

    fn get(&self) -> (f32, f32, f32) {
        (read(&self.rpm), read(&self.load), read(&self.volume))
    }
}

#[derive(Asset, TypePath, Clone)]
struct EngineSynth {
    targets: Arc<Targets>,
}

impl Decodable for EngineSynth {
    type Decoder = EngineVoice;

    fn decoder(&self) -> EngineVoice {
        EngineVoice::new(self.targets.clone())
    }
}

/// A band-pass filter (peak gain 1) for one exhaust resonance.
#[derive(Clone, Copy, Default)]
struct Resonator {
    b0: f32,
    a1: f32,
    a2: f32,
    x: [f32; 2],
    y: [f32; 2],
}

impl Resonator {
    /// Moves the resonance, keeping what's ringing.
    fn tune(&mut self, frequency: f32, q: f32) {
        let w = TAU * frequency / SAMPLE_RATE as f32;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = alpha / a0;
        self.a1 = -2.0 * w.cos() / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * (x - self.x[1]) - self.a1 * self.y[0] - self.a2 * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

/// How the exhaust shapes a bank's sound in this sample.
struct Shape {
    /// Gains of the resonances, the header's last.
    gains: [f32; 5],
    /// Level of the combustion noise, relative to fully muffled.
    noise: f32,
    /// Share of the way the smeared pulse moves towards the bare one each sample (1 without
    /// a turbine).
    smoothing: f32,
    /// The engine's time passing in a sample (seconds; shifted by Doppler).
    dt: f32,
    /// How long a pulse rises and falls (seconds of the engine's time).
    pulse: (f32, f32),
    /// What comes back from the far end of the header, the front pipe and the tailpipe
    /// (negative: inverted), and the share of the way the pipes' low-pass moves each
    /// sample.
    reflections: [f32; 3],
    loss: f32,
    /// Gain of what's above `BRIGHT_SPLIT`, and the share of the way the low-pass that
    /// splits it off moves each sample.
    bright: f32,
    split: f32,
}

/// A length of pipe: what goes in comes back from the far end after a while (a delay
/// line), partly reflected and losing its highs, so it rings at a series of frequencies.
#[derive(Clone, Copy)]
struct Tube {
    line: [f32; TUBE],
    at: usize,
    lowpass: f32,
}

impl Default for Tube {
    fn default() -> Self {
        Self {
            line: [0.0; TUBE],
            at: 0,
            lowpass: 0.0,
        }
    }
}

impl Tube {
    /// What went in `lag` samples ago (between samples).
    fn read(&self, lag: f32) -> f32 {
        let back = self.at as f32 + TUBE as f32 - lag.clamp(1.0, (TUBE - 2) as f32);
        let i = back as usize;
        let share = back.fract();
        let (a, b) = (self.line[i % TUBE], self.line[(i + 1) % TUBE]);
        a + (b - a) * share
    }

    /// The next sample out, for `x` going in, a way to the end and back of `trip` samples.
    /// Scaled so the lowest frequencies pass at their level whatever the reflection (an
    /// inverted one would take them out).
    fn process(&mut self, x: f32, trip: f32, reflection: f32, loss: f32) -> f32 {
        self.lowpass += loss * (self.read(trip) - self.lowpass);
        let y = x + reflection * self.lowpass;
        self.line[self.at] = y;
        self.at = (self.at + 1) % TUBE;
        y * (1.0 - reflection)
    }
}

/// One bank's exhaust: the last pulse and the resonances it rings.
#[derive(Clone, Copy)]
struct Bank {
    /// Time since the bank's last pulse arrived (seconds; negative while it's on its way),
    /// and how hard that firing was.
    since: f32,
    strength: f32,
    /// How long a pulse takes to arrive.
    delay: f32,
    /// Resonances relative to the big V8's: from the layout's pipes, and this bank's side.
    pipes: f32,
    side: f32,
    /// The pipe pitch and header resonance the resonators are tuned to.
    tuned: (f32, f32),
    /// The exhaust's resonances, then the header primaries'.
    resonators: [Resonator; 5],
    /// The header, the front pipe and the tailpipe, and the ways to their ends and back
    /// (samples).
    tubes: [Tube; 3],
    trips: [f32; 3],
    /// The pulse after the turbine.
    smeared: f32,
    /// What's below `BRIGHT_SPLIT` of the piped pulse.
    below: f32,
    /// Last input and output of the DC blocker.
    dc: (f32, f32),
}

impl Bank {
    fn new(pipes: f32, side: f32, delay: f32, exhaust: &ExhaustSound) -> Self {
        let mut bank = Self {
            since: 1.0,
            strength: 0.0,
            delay,
            pipes,
            side,
            tuned: (0.0, 0.0),
            resonators: [Resonator::default(); 5],
            tubes: [Tube::default(); 3],
            trips: [1.0; 3],
            smeared: 0.0,
            below: 0.0,
            dc: (0.0, 0.0),
        };
        bank.tune(exhaust.bore.powf(-BORE_PITCH), exhaust.header);
        bank
    }

    /// Tunes the resonances to the pipes' pitch (relative to stock) and the header's
    /// resonance (Hz), if they moved.
    fn tune(&mut self, pitch: f32, header: f32) {
        let (tuned_pitch, tuned_header) = self.tuned;
        if (pitch - tuned_pitch).abs() < 1e-3 && (header - tuned_header).abs() < 0.5 {
            return;
        }
        self.tuned = (pitch, header);
        let scale = self.pipes * self.side * pitch;
        for (resonator, &(frequency, q, _, _)) in self.resonators.iter_mut().zip(&RESONANCES) {
            resonator.tune(frequency * scale, q);
        }
        self.resonators[4].tune(header * self.side, HEADER.0);
        // The header's quarter-wave resonance is a way there and back in half its period.
        let rate = SAMPLE_RATE as f32;
        self.trips = [
            rate / (2.0 * header * self.side),
            rate * 2.0 * FRONT_PIPE / (EXHAUST_SOUND_SPEED * scale),
            rate * 2.0 * TAIL_PIPE / (TAIL_SOUND_SPEED * scale),
        ];
    }

    fn next(&mut self, noise: f32, shape: &Shape) -> f32 {
        let dt = shape.dt;
        let excitation = if self.since < 0.0 {
            0.0
        } else {
            let (rise, fall) = shape.pulse;
            // The front starts at once (a kink, not easing in), and rounds over at the top.
            let pulse = if self.since < rise {
                (0.5 * PI * self.since / rise).sin()
            } else if self.since < rise + fall {
                (0.5 * PI * (self.since - rise) / fall).cos().powi(2)
            } else {
                0.0
            };
            let combustion = 0.3 * shape.noise * noise * (-self.since / NOISE_DECAY).exp();
            self.strength * (pulse + combustion)
        };
        self.since += dt;
        self.smeared += shape.smoothing * (excitation - self.smeared);
        // Through the header, the front pipe and the tailpipe, then the exhaust's broad
        // resonances.
        let mut piped = self.smeared;
        for ((tube, &trip), reflection) in self
            .tubes
            .iter_mut()
            .zip(&self.trips)
            .zip(shape.reflections)
        {
            piped = tube.process(piped, trip, reflection, shape.loss);
        }
        let piped = PIPE_LEVEL * piped;
        self.below += shape.split * (piped - self.below);
        let mut out = shape.bright * (piped - self.below);
        for (resonator, gain) in self.resonators.iter_mut().zip(shape.gains) {
            out += gain * resonator.process(piped);
        }
        // Block any DC offset, which the pulses (all positive pressure) would leave.
        let y = out - self.dc.0 + 0.995 * self.dc.1;
        self.dc = (out, y);
        y
    }
}

/// An endless stereo stream (left and right samples interleaved).
struct EngineVoice {
    targets: Arc<Targets>,
    rpm: f32,
    load: f32,
    volume: f32,
    /// The layout the banks are set up for, and how it sounds.
    layout: Layout,
    sound: Sound,
    /// The exhaust, gliding towards the target.
    exhaust: ExhaustSound,
    /// Position in the four-stroke cycle, in firings.
    cycle: f32,
    banks: [Bank; 2],
    /// Turbo shaft speed (revolutions per second) and its share of the limit, and the
    /// whistle's band-pass filter with the centre it's tuned to (Hz).
    turbo_speed: f32,
    turbo_share: f32,
    whistle: Resonator,
    whistle_tuned: f32,
    /// Where it's heard from (see `Targets::place`), gliding, and the low-pass's state.
    pan: f32,
    cutoff: f32,
    doppler: f32,
    lowpass: f32,
    /// State of the noise generator.
    seed: u32,
    /// The right sample, waiting for its turn after the left one.
    right: Option<f32>,
}

impl EngineVoice {
    fn new(targets: Arc<Targets>) -> Self {
        let layout = targets.layout();
        let sound = sound(layout);
        let exhaust = targets.exhaust();
        // Heard from where it is from the start.
        let (pan, cutoff, doppler) = targets.place();
        Self {
            targets,
            rpm: 0.0,
            load: 0.0,
            volume: 0.0,
            layout,
            banks: banks(&sound, &exhaust),
            sound,
            exhaust,
            cycle: 0.0,
            turbo_speed: 0.0,
            turbo_share: 0.0,
            whistle: Resonator::default(),
            whistle_tuned: 0.0,
            pan,
            cutoff,
            doppler,
            lowpass: 0.0,
            seed: 0x2545_f491,
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

    /// Advances by one sample, returning the left and right outputs.
    fn frame(&mut self) -> (f32, f32) {
        let rate = SAMPLE_RATE as f32;
        let glide = |value: &mut f32, target: f32, time: f32| {
            *value += (target - *value) / (time * rate);
        };
        let (rpm, load, volume) = self.targets.get();
        glide(&mut self.rpm, rpm.max(0.0), GLIDE);
        glide(&mut self.load, load.clamp(0.0, 1.0), LOAD_GLIDE);
        glide(&mut self.volume, volume, GLIDE);
        let exhaust = self.targets.exhaust();
        glide(
            &mut self.exhaust.muffle,
            exhaust.muffle.clamp(0.0, 1.0),
            GLIDE,
        );
        glide(&mut self.exhaust.bore, exhaust.bore.clamp(0.5, 2.0), GLIDE);
        glide(
            &mut self.exhaust.header,
            exhaust.header.clamp(50.0, 1000.0),
            GLIDE,
        );
        glide(
            &mut self.exhaust.turbine,
            exhaust.turbine.clamp(0.0, 1.0),
            GLIDE,
        );
        let (revolutions, share) = self.targets.turbo();
        glide(&mut self.turbo_speed, revolutions.max(0.0), GLIDE);
        glide(&mut self.turbo_share, share.clamp(0.0, 1.5), GLIDE);
        let (pan, cutoff, doppler) = self.targets.place();
        glide(&mut self.pan, pan.clamp(-1.0, 1.0), GLIDE);
        glide(&mut self.doppler, doppler.clamp(0.5, 2.0), GLIDE);
        if cutoff > 0.0 {
            glide(&mut self.cutoff, cutoff.clamp(100.0, 20_000.0), GLIDE);
        }
        // Doppler: everything about the engine happens faster or slower.
        let doppler = self.doppler;

        // Another engine: its exhaust rings differently.
        let layout = self.targets.layout();
        if layout != self.layout {
            self.layout = layout;
            self.sound = sound(layout);
            self.banks = banks(&self.sound, &self.exhaust);
            self.cycle = 0.0;
        }

        let firings = self.sound.banks.len();
        let before = self.cycle;
        // Every cylinder fires once in two revolutions.
        self.cycle =
            (self.cycle + doppler * self.rpm / 120.0 * firings as f32 / rate) % firings as f32;
        if self.cycle.floor() != before.floor() {
            let firing = self.cycle.floor() as usize % firings;
            let (own_strength, own_late) = CYLINDERS[firing % CYLINDERS.len()];
            let jitter = JITTER_IDLE + (JITTER_LOAD - JITTER_IDLE) * self.load;
            let strength = (IDLE_STRENGTH + (1.0 - IDLE_STRENGTH) * self.load)
                * (1.0 + CYLINDER_STRENGTH * own_strength)
                * (1.0 + jitter * self.noise());
            // Seconds of the engine's time between firings.
            let interval = 120.0 / (self.rpm.max(100.0) * firings as f32);
            let late = CYLINDER_LATE * (own_late + 1.0) * interval;
            let bank = &mut self.banks[usize::from(self.sound.banks[firing])];
            bank.since = -bank.delay - late;
            bank.strength = strength;
        }

        let load = self.load;
        let ExhaustSound {
            muffle,
            bore,
            header,
            turbine,
        } = self.exhaust;
        let open = 1.0 - muffle;
        let shape = Shape {
            gains: std::array::from_fn(|i| {
                let (off, full) = match RESONANCES.get(i) {
                    Some(&(_, _, off, full)) => (off, full),
                    None => (HEADER.1, HEADER.2),
                };
                (off + (full - off) * load) * (1.0 - muffle * MUFFLED[i])
            }),
            noise: 1.0 + (OPEN_NOISE - 1.0) * open,
            smoothing: 1.0 / (1.0 + TURBINE_SMEAR * turbine * rate / doppler),
            dt: doppler / rate,
            pulse: {
                // Crank degrees a second.
                let degrees = 6.0 * self.rpm.max(100.0);
                let rise = PULSE_RISE + (PULSE_SHARP - PULSE_RISE) * load;
                ((rise / degrees).max(PULSE_SHORTEST), PULSE_FALL / degrees)
            },
            reflections: [
                -HEADER_REFLECTION,
                -(FRONT_REFLECTION.0 + (FRONT_REFLECTION.1 - FRONT_REFLECTION.0) * muffle),
                -(TAIL_REFLECTION.0 + (TAIL_REFLECTION.1 - TAIL_REFLECTION.0) * muffle),
            ],
            loss: 1.0
                - (-TAU * (PIPE_LOSS.0 + (PIPE_LOSS.1 - PIPE_LOSS.0) * open) * bore / rate).exp(),
            bright: (BRIGHT.0 + (BRIGHT.1 - BRIGHT.0) * open)
                * (BRIGHT_IDLE + (1.0 - BRIGHT_IDLE) * load),
            split: 1.0 - (-TAU * BRIGHT_SPLIT / rate).exp(),
        };
        let pitch = bore.powf(-BORE_PITCH) * doppler;
        for bank in &mut self.banks {
            bank.tune(pitch, header * doppler);
        }
        let left = self.noise();
        let right = self.noise();
        let left = self.banks[0].next(left, &shape);
        let right = self.banks[1].next(right, &shape);

        // The turbo whistles around its shaft speed, louder spooled up and under load: a
        // band of noise, retuned as the shaft speeds up.
        let frequency =
            (doppler * self.turbo_speed * WHISTLE_PITCH).clamp(WHISTLE_MIN, WHISTLE_MAX);
        if (frequency / self.whistle_tuned.max(1.0) - 1.0).abs() > 0.005 {
            self.whistle.tune(frequency, WHISTLE_Q);
            self.whistle_tuned = frequency;
        }
        // Uniform noise has an RMS of 1/√3, and the band-pass passes a share of it that
        // grows with its bandwidth: scale it back to an RMS of 1.
        let band = (PI * self.whistle_tuned / (WHISTLE_Q * rate)).sqrt() / 3f32.sqrt();
        let hiss = self.noise();
        let whistle = WHISTLE
            * self.turbo_share.powi(2)
            * (WHISTLE_IDLE + (1.0 - WHISTLE_IDLE) * load)
            * self.whistle.process(hiss)
            / band;

        // Fewer cylinders pulse less often: level them out somewhat.
        let exhaust_level = (MUFFLED_LEVEL + (OPEN_LEVEL - MUFFLED_LEVEL) * open)
            * (1.0 + BORE_LEVEL * (bore - 1.0))
            * (1.0 - (1.0 - TURBINE_LEVEL) * turbine);
        let level = GAIN * self.volume * exhaust_level * (8.0 / firings as f32).sqrt();
        let whistle = GAIN * self.volume * whistle;
        let mix = |near: f32, far: f32| (level * (near + CROSSFEED * far) + whistle).tanh();
        if self.cutoff > 0.0 {
            // Another car: one source, duller with distance, panned (constant power).
            let both = (level * (1.0 + CROSSFEED) / 2.0 * (left + right) + whistle).tanh();
            let share = 1.0 - (-TAU * self.cutoff / rate).exp();
            self.lowpass += share * (both - self.lowpass);
            let angle = (self.pan + 1.0) * PI / 4.0;
            return (self.lowpass * angle.cos(), self.lowpass * angle.sin());
        }
        if self.sound.dual {
            (mix(left, right), mix(right, left))
        } else {
            let both = (level * (1.0 + CROSSFEED) / 2.0 * (left + right) + whistle).tanh();
            (both, both)
        }
    }
}

/// The two banks' exhausts for a layout. Engines with a single bank only ever fire the
/// first.
fn banks(sound: &Sound, exhaust: &ExhaustSound) -> [Bank; 2] {
    [
        Bank::new(sound.pipes, 1.0, 0.0, exhaust),
        Bank::new(sound.pipes, RIGHT_PIPES, sound.delay, exhaust),
    ]
}

impl Iterator for EngineVoice {
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

impl Source for EngineVoice {
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

/// The engine sound: what the audio thread plays, and whether the
/// driven car is out of earshot (`hushed`: its driver is in the dyno's room).
#[derive(Resource)]
pub struct EngineAudio {
    targets: Arc<Targets>,
    pub hushed: bool,
}

impl Targets {
    /// Plays an engine that sounds like `voice` running as `run` says, at `volume`.
    fn hear(&self, voice: &EngineCharacter, run: &EngineRun, volume: f32) {
        let volume = if run.running { volume } else { 0.0 };
        self.set(run.rpm, run.load, volume);
        self.set_layout(voice.layout);
        self.set_exhaust(voice.exhaust);
        let share = if voice.turbo_limit > 0.0 {
            run.turbo_speed / voice.turbo_limit
        } else {
            0.0
        };
        self.set_turbo(run.turbo_speed / TAU, share);
    }
}

/// Another car's engine being heard: a sound of its own, on an entity of its own.
#[derive(Component)]
struct Voice {
    targets: Arc<Targets>,
    /// The car it's the engine of, `None` once it's fading out.
    car: Option<Entity>,
    /// The distance last frame and how fast the car closes in (m/s, smoothed).
    distance: f32,
    closing: f32,
    /// When a fading voice stops (seconds of game time).
    until: f64,
}

fn start_engine(mut commands: Commands, mut synths: ResMut<Assets<EngineSynth>>) {
    let targets = Arc::new(Targets::new());
    commands.spawn(AudioPlayer(synths.add(EngineSynth {
        targets: targets.clone(),
    })));
    commands.insert_resource(EngineAudio {
        targets,
        hushed: false,
    });
}

fn update_engine(
    sound: Res<SoundSettings>,
    active: Res<ActiveCar>,
    cars: Query<(&Car, &Looks, &Motion)>,
    engine: Res<EngineAudio>,
) {
    let Some((_, looks, motion)) = cars.iter().find(|(car, ..)| car.index == active.0) else {
        // No car to drive (yet): silence.
        let (rpm, load, _) = engine.targets.get();
        engine.targets.set(rpm, load, 0.0);
        return;
    };
    let volume = if engine.hushed { 0.0 } else { sound.master() };
    engine.targets.hear(&looks.engine, &motion.engine, volume);
}

/// How loud another car's engine is at `distance`, its low-pass's cutoff, and the Doppler
/// factor for a car closing in at `closing`.
fn placed(distance: f32, closing: f32) -> (f32, f32, f32) {
    let near = distance.max(NEAR);
    let fade = ((HEARING - distance) / (0.2 * HEARING)).clamp(0.0, 1.0);
    let volume = NEAR / near * fade;
    let cutoff = NEAR_CUTOFF * (NEAR / near).sqrt();
    let doppler = SOUND_SPEED / (SOUND_SPEED - closing.clamp(-MAX_CLOSING, MAX_CLOSING));
    (volume, cutoff, doppler)
}

/// The engines of the cars around (other players' and traffic: any car seen but not
/// driven here), the nearest few, heard from the camera.
#[allow(clippy::type_complexity)]
fn hear_others(
    mut commands: Commands,
    time: Res<Time>,
    sound: Res<SoundSettings>,
    mut synths: ResMut<Assets<EngineSynth>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    cars: Query<(Entity, &GlobalTransform, &Looks, &Motion), Without<Car>>,
    mut voices: Query<(Entity, &mut Voice)>,
) {
    let now = time.elapsed_secs_f64();
    let dt = time.delta_secs();
    let Ok(camera) = cameras.single() else {
        return;
    };
    let ear = camera.translation();
    let right = camera.right();

    // Who's heard: the nearest within hearing, those already heard a little further.
    let mut heard = Vec::new();
    for (car, transform, ..) in &cars {
        let distance = transform.translation().distance(ear);
        let voiced = voices.iter().any(|(_, voice)| voice.car == Some(car));
        let range = if voiced {
            HEARING
        } else {
            HEARING * HEARD_FROM
        };
        if distance < range {
            heard.push((distance, car));
        }
    }
    heard.sort_by(|a, b| a.0.total_cmp(&b.0));
    heard.truncate(VOICES);

    // Voices whose car isn't heard any more fade out, then stop.
    for (entity, mut voice) in &mut voices {
        if let Some(car) = voice.car
            && !heard.iter().any(|&(_, heard)| heard == car)
        {
            voice.car = None;
            voice.until = now + f64::from(FADE);
            let (rpm, load, _) = voice.targets.get();
            voice.targets.set(rpm, load, 0.0);
        }
        if voice.car.is_none() && now >= voice.until {
            commands.entity(entity).despawn();
        }
    }

    let master = sound.master();
    for (distance, car) in heard {
        let Ok((_, transform, looks, motion)) = cars.get(car) else {
            continue;
        };
        let pan = pan(transform.translation() - ear, right);
        if let Some((_, mut voice)) = voices.iter_mut().find(|(_, voice)| voice.car == Some(car)) {
            if dt > 0.0 {
                let closing = (voice.distance - distance) / dt;
                voice.closing += (closing - voice.closing) * (dt / CLOSING_SMOOTHING).min(1.0);
            }
            voice.distance = distance;
            let (volume, cutoff, doppler) = placed(distance, voice.closing);
            voice
                .targets
                .hear(&looks.engine, &motion.engine, master * volume);
            voice.targets.set_place(pan, cutoff, doppler);
            continue;
        }
        // Newly heard: a voice of its own, set up before it starts.
        let (volume, cutoff, doppler) = placed(distance, 0.0);
        let targets = Arc::new(Targets::new());
        targets.hear(&looks.engine, &motion.engine, master * volume);
        targets.set_place(pan, cutoff, doppler);
        commands.spawn((
            Voice {
                targets: targets.clone(),
                car: Some(car),
                distance,
                closing: 0.0,
                until: 0.0,
            },
            AudioPlayer(synths.add(EngineSynth { targets })),
        ));
    }
}

/// Pan (-1 left to 1 right) of a sound in direction `to` for a listener whose right is
/// `right`.
fn pan(to: Vec3, right: Dir3) -> f32 {
    to.normalize_or_zero().dot(*right).clamp(-1.0, 1.0)
}

/// Writes stereo `frames` (left, right; -1 to 1, clipped) to a 16-bit WAV file at `path`,
/// for listening to the synths outside the game.
#[cfg(test)]
fn write_wav(path: impl AsRef<std::path::Path>, frames: &[(f32, f32)]) {
    let data = (frames.len() * 4) as u32;
    let mut wav = Vec::with_capacity(44 + data as usize);
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
    for &(left, right) in frames {
        for sample in [left, right] {
            let sample = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            wav.extend_from_slice(&sample.to_le_bytes());
        }
    }
    std::fs::write(path, wav).unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders every factory engine (or those in `ENGINES`, codes separated by commas) to
    /// `<ENGINE_WAV>/<code>.wav` (default `engine-wav`), through the same program, at the
    /// level the game plays it, so files can be compared with each other and with older
    /// renders: idle, three blips, a full-throttle pull from 2000 rpm to the redline,
    /// lifting off back down, a light cruise, then the same engine passing at 90 km/h,
    /// heard from 8 m beside the road.
    #[test]
    #[ignore]
    fn write_engine_wavs() {
        use crate::physics::engine::families::{FAMILIES, Version};
        let dir = std::env::var("ENGINE_WAV").unwrap_or("engine-wav".into());
        let only = std::env::var("ENGINES").ok();
        std::fs::create_dir_all(&dir).unwrap();
        // Targets change at the game's frame rate; the synth glides between them.
        const TICK: f32 = 1.0 / 60.0;
        const IDLE: f32 = 850.0;
        for (f, family) in FAMILIES.iter().enumerate() {
            for (v, version) in family.versions.iter().enumerate() {
                if let Some(only) = &only
                    && !only.split(',').any(|code| code.trim() == version.code)
                {
                    continue;
                }
                let hardware = Version::parts(f, v).hardware();
                let redline = hardware.safe_rpm(usize::MAX);
                let character = EngineCharacter {
                    layout: family.layout,
                    exhaust: ExhaustSound::of(&hardware),
                    turbo_limit: hardware.turbo.map_or(0.0, |turbo| turbo.max_speed()),
                };
                // A turbo spooling with engine speed and load, lagging a little.
                let mut spool = 0.0;
                let mut run = |rpm: f32, load: f32| {
                    let target = (rpm / redline) * (0.3 + 0.7 * load);
                    spool += (target - spool) * (TICK / 0.4);
                    EngineRun {
                        running: true,
                        rpm,
                        load,
                        turbo_speed: spool * character.turbo_limit,
                    }
                };
                // Engine speed and load over time (seconds since the segment began).
                type Segment<'a> = (f32, Box<dyn Fn(f32) -> (f32, f32) + 'a>);
                let rise = 0.6 * redline;
                let program: Vec<Segment> = vec![
                    (2.0, Box::new(|_| (IDLE, 0.12))),
                    (
                        3.0,
                        Box::new(|t| {
                            // A blip a second: 0.2 s on the gas, then falling back.
                            let t = t % 1.0;
                            if t < 0.2 {
                                (IDLE + (rise - IDLE) * t / 0.2, 1.0)
                            } else {
                                let fall = ((t - 0.2) / 0.5).min(1.0);
                                (rise + (IDLE - rise) * fall, 0.0)
                            }
                        }),
                    ),
                    (1.0, Box::new(|_| (IDLE, 0.12))),
                    (
                        6.0,
                        Box::new(|t| (2000.0 + (redline - 2000.0) * t / 6.0, 1.0)),
                    ),
                    (
                        3.0,
                        Box::new(|t| (redline + (2000.0 - redline) * t / 3.0, 0.0)),
                    ),
                    (2.0, Box::new(|_| (2500.0, 0.3))),
                ];
                let targets = Arc::new(Targets::new());
                let mut voice = EngineVoice::new(targets.clone());
                let mut frames = Vec::new();
                let per_tick = (TICK * SAMPLE_RATE as f32) as usize;
                for (length, at) in &program {
                    for tick in 0..(length / TICK) as usize {
                        let (rpm, load) = at(tick as f32 * TICK);
                        targets.hear(&character, &run(rpm, load), 1.0);
                        frames.extend((0..per_tick).map(|_| voice.frame()));
                    }
                }
                frames.extend((0..SAMPLE_RATE as usize / 2).map(|_| (0.0, 0.0)));

                // The pass-by: in fourth at 4000 rpm and a little gas, along x past a
                // listener 8 m from the road, facing it (their right is +x).
                let targets = Arc::new(Targets::new());
                let (speed, side, length) = (25.0, 8.0, 6.0);
                let place = |t: f32| Vec3::new(-0.5 * speed * length + speed * t, 0.0, -side);
                let right = Dir3::X;
                let (rpm, load) = (4000.0, 0.6);
                let distance = |t: f32| place(t).length();
                let (volume, cutoff, doppler) = placed(distance(0.0), 0.0);
                targets.hear(&character, &run(rpm, load), volume);
                targets.set_place(pan(place(0.0), right), cutoff, doppler);
                let mut voice = EngineVoice::new(targets.clone());
                for tick in 0..(length / TICK) as usize {
                    let t = tick as f32 * TICK;
                    let closing = (distance(t - TICK) - distance(t)) / TICK;
                    let (volume, cutoff, doppler) = placed(distance(t), closing);
                    targets.hear(&character, &run(rpm, load), volume);
                    targets.set_place(pan(place(t), right), cutoff, doppler);
                    frames.extend((0..per_tick).map(|_| voice.frame()));
                }

                let path = std::path::Path::new(&dir).join(format!("{}.wav", version.code));
                write_wav(&path, &frames);
                println!(
                    "{}: {} {} ({:?}), redline {redline} rpm",
                    path.display(),
                    family.name,
                    version.name,
                    family.layout,
                );
            }
        }
    }

    #[test]
    fn each_bank_of_a_cross_plane_v8_fires_unevenly() {
        // Firing slots of each bank in the 8-slot cycle, and the gaps between them.
        for bank in [0, 1] {
            let slots: Vec<usize> = (0..8).filter(|&i| CROSS_PLANE[i] == bank).collect();
            assert_eq!(slots.len(), 4);
            let mut gaps: Vec<usize> = (0..4)
                .map(|i| (slots[(i + 1) % 4] + 8 - slots[i]) % 8)
                .collect();
            gaps.sort();
            // In slots of 90° crank: 90°, 180°, 180° and 270°.
            assert_eq!(gaps, [1, 2, 2, 3], "bank {bank}");
        }
    }

    #[test]
    fn every_layout_fires_each_cylinder_once_a_cycle() {
        use crate::physics::engine::families::FAMILIES;
        for family in &FAMILIES {
            let sound = sound(family.layout);
            let cylinders = family.versions[0].block().cylinders as usize;
            assert_eq!(sound.banks.len(), cylinders, "{}", family.name);
            assert!(LAYOUTS.contains(&family.layout));
            // Two exhausts need two banks, each with half the cylinders.
            let second = sound.banks.iter().filter(|&&bank| bank == 1).count();
            if sound.dual {
                assert_eq!(second * 2, cylinders, "{}", family.name);
            }
        }
    }

    #[test]
    fn stays_in_range_and_glides() {
        let targets = Arc::new(Targets::new());
        let mut voice = EngineVoice::new(targets.clone());
        // Every engine in turn, swapped while running, every other one with straight
        // pipes, short headers, a big bore and a turbo whistling past its limit.
        let wild = ExhaustSound {
            muffle: 0.0,
            bore: 1.4,
            header: 400.0,
            turbine: 1.0,
        };
        for (i, layout) in LAYOUTS.into_iter().enumerate() {
            targets.set_layout(layout);
            if i % 2 == 1 {
                targets.set_exhaust(wild);
                targets.set_turbo(3500.0, 1.1);
            } else {
                targets.set_exhaust(ExhaustSound::default());
                targets.set_turbo(0.0, 0.0);
            }
            // Idle, then a jump to near the redline at full throttle, then lifting off.
            for (rpm, load) in [(850.0, 0.1), (6800.0, 1.0), (6000.0, 0.0)] {
                targets.set(rpm, load, 1.0);
                let mut last = voice.rpm;
                let mut peak = 0.0f32;
                for _ in 0..SAMPLE_RATE {
                    let (left, right) = voice.frame();
                    assert!(left.is_finite() && right.is_finite());
                    peak = peak.max(left.abs()).max(right.abs());
                    // Each sample moves the engine speed only a little.
                    assert!((voice.rpm - last).abs() < 5.0);
                    last = voice.rpm;
                }
                assert!(peak <= 1.0);
                // Audible, after a second (20 time constants) at the new target.
                assert!(peak > 0.05, "{layout:?} at {rpm} rpm: peak {peak}");
                assert!((voice.rpm - rpm).abs() < 1.0);
            }
            assert_eq!(voice.layout, layout);
        }
    }

    /// The stock 2-litre four with `part` in the slot called `slot`.
    fn with(slot: &str, part: usize) -> EngineHardware {
        use crate::physics::engine::parts::{Parts, SLOTS};
        let mut parts = Parts::stock();
        let slot = SLOTS.iter().position(|s| s.name == slot).unwrap();
        parts.chosen[slot] = part;
        parts.hardware()
    }

    #[test]
    fn the_exhaust_parts_change_the_sound() {
        use crate::physics::engine::parts::Parts;
        let stock = ExhaustSound::of(&Parts::stock().hardware());
        assert!((stock.bore - 1.0).abs() < 0.01, "{stock:?}");
        assert!(stock.muffle > 0.85, "{stock:?}");
        assert_eq!(stock.turbine, 0.0);
        // Less restrictive silencers and catalysts muffle less; straight pipes don't.
        let [sport, straight] = [1, 2].map(|part| ExhaustSound::of(&with("Silencer", part)));
        assert!(straight.muffle < sport.muffle && sport.muffle < stock.muffle - 0.2);
        let [high_flow, no_cat] = [1, 2].map(|part| ExhaustSound::of(&with("Catalyst", part)));
        assert!(no_cat.muffle < high_flow.muffle && high_flow.muffle < stock.muffle);
        let big_bore = ExhaustSound::of(&with("Exhaust pipe", 1));
        assert!(big_bore.bore > 1.1);
        let [short, long] = [1, 2].map(|part| ExhaustSound::of(&with("Exhaust manifold", part)));
        assert!(short.header > stock.header && stock.header > long.header);
        assert_eq!(ExhaustSound::of(&with("Turbocharger", 2)).turbine, 1.0);
        // Every factory engine has a silencer and its pipes sized for it.
        for family in 0..crate::physics::engine::families::FAMILIES.len() {
            let versions = crate::physics::engine::families::FAMILIES[family].versions;
            for version in 0..versions.len() {
                let hardware = Parts::all_stock(family, version).hardware();
                let sound = ExhaustSound::of(&hardware);
                assert!(
                    (sound.bore - 1.0).abs() < 0.05,
                    "{}: {sound:?}",
                    hardware.name
                );
                assert!(sound.muffle > 0.85, "{}: {sound:?}", hardware.name);
            }
        }
    }

    /// Root mean square of both channels over a second at `rpm` and full throttle, after
    /// a second to settle.
    fn loudness(exhaust: ExhaustSound, turbo: f32) -> f32 {
        let targets = Arc::new(Targets::new());
        targets.set_layout(Layout::Inline4);
        targets.set_exhaust(exhaust);
        targets.set_turbo(turbo * 3000.0, turbo);
        targets.set(4000.0, 1.0, 1.0);
        let mut voice = EngineVoice::new(targets);
        let mut sum = 0.0;
        for i in 0..2 * SAMPLE_RATE {
            let (left, right) = voice.frame();
            assert!(left.abs() <= 1.0 && right.abs() <= 1.0);
            if i >= SAMPLE_RATE {
                sum += left * left + right * right;
            }
        }
        (sum / (2 * SAMPLE_RATE) as f32).sqrt()
    }

    #[test]
    fn straight_pipes_are_louder_and_turbos_quieter_but_whistling() {
        let stock = ExhaustSound::default();
        let straight = ExhaustSound {
            muffle: 0.0,
            ..stock
        };
        let turbo = ExhaustSound {
            turbine: 1.0,
            ..stock
        };
        let quiet = loudness(stock, 0.0);
        assert!(loudness(straight, 0.0) > 1.3 * quiet);
        let turbo_quiet = loudness(turbo, 0.0);
        assert!(turbo_quiet < quiet);
        // The whistle on its own (uncorrelated with the exhaust, so their energies add):
        // there, though quiet (here, at 4000 rpm, the pipes ring loudly).
        let whistle = (loudness(turbo, 1.0).powi(2) - turbo_quiet.powi(2)).sqrt();
        assert!(whistle > 0.01 * turbo_quiet, "whistle {whistle}");
    }

    #[test]
    fn the_whistle_is_as_loud_as_set_at_any_pitch() {
        // No firings (the engine at a standstill), just the whistle at the shaft's limit.
        for revolutions in [1000.0, 3000.0, 6000.0] {
            let targets = Arc::new(Targets::new());
            targets.set_turbo(revolutions, 1.0);
            targets.set(0.0, 1.0, 1.0);
            let mut voice = EngineVoice::new(targets);
            let mut sum = 0.0;
            for i in 0..2 * SAMPLE_RATE {
                let (left, _) = voice.frame();
                if i >= SAMPLE_RATE {
                    sum += left * left;
                }
            }
            let rms = (sum / SAMPLE_RATE as f32).sqrt();
            let set = GAIN * WHISTLE;
            assert!(
                (rms / set - 1.0).abs() < 0.3,
                "{revolutions}: {rms} for {set}"
            );
        }
    }

    #[test]
    fn other_cars_are_panned_quieter_and_duller_further_away() {
        let (near, near_cutoff, _) = placed(3.0, 0.0);
        let (far, far_cutoff, _) = placed(100.0, 0.0);
        assert_eq!(near, 1.0);
        assert!(far < 0.1 * near && far_cutoff < 0.5 * near_cutoff);
        assert_eq!(placed(HEARING, 0.0).0, 0.0);
        let (_, _, coming) = placed(50.0, 30.0);
        let (_, _, going) = placed(50.0, -30.0);
        assert!(coming > 1.05 && going < 0.95);

        // All the way to the right: nothing on the left.
        let targets = Arc::new(Targets::new());
        targets.set_layout(Layout::V8);
        targets.set(3000.0, 0.5, 1.0);
        targets.set_place(1.0, 5000.0, 1.1);
        let mut voice = EngineVoice::new(targets);
        let (mut left, mut right) = (0.0f32, 0.0f32);
        for _ in 0..SAMPLE_RATE {
            let (l, r) = voice.frame();
            assert!(l.is_finite() && r.is_finite() && r.abs() <= 1.0);
            left = left.max(l.abs());
            right = right.max(r.abs());
        }
        assert!(right > 0.05 && left < 0.01 * right, "{left} {right}");
    }

    #[test]
    fn silent_when_off() {
        let targets = Arc::new(Targets::new());
        let mut voice = EngineVoice::new(targets.clone());
        targets.set(850.0, 0.0, 0.0);
        for _ in 0..SAMPLE_RATE / 10 {
            assert_eq!(voice.frame(), (0.0, 0.0));
        }
    }
}
