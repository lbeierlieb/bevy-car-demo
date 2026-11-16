//! Engine sound for the active car, synthesised sample by sample from its exhaust pulses.
//! Each firing sends a pulse down its bank's exhaust; the engine's layout says how many
//! there are in a cycle and which bank each goes down. A cross-plane V8 fires each bank at
//! uneven intervals (the burble), a flat four's unequal headers bunch its pulses up in
//! pairs (the boxer rumble), and the others fire evenly, the more cylinders the smoother.
//! The pulses ring a few resonances of the exhaust, so the tone changes with engine speed
//! rather than just its pitch, with a little combustion noise on top. Two banks with an
//! exhaust each are panned apart.
//!
//! The exhaust hardware colours it: the silencer and the catalyst take out level and the
//! higher resonances (a straight pipe is loud and rasping), a bigger bore rings a little
//! deeper and louder, the header primaries add a resonance of their own (short ones ring
//! higher), and a turbine in the way smears the pulses and quietens them. A turbo whistles
//! with its shaft speed, louder as it spools under load.
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
use crate::physics::engine::families::{Layout, relative_size};
use crate::physics::engine::hardware::EngineHardware;

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
/// Length of an exhaust pulse (seconds), and how fast the combustion noise after it dies.
const PULSE: f32 = 0.0025;
const NOISE_DECAY: f32 = 0.004;
/// Random variation of each firing's strength.
const JITTER: f32 = 0.2;
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
/// How much louder a bigger pipe is per relative diameter, and how its resonances move
/// (frequency ∝ diameter^-this).
const BORE_LEVEL: f32 = 0.3;
const BORE_PITCH: f32 = 0.25;
/// A turbine in the exhaust: the time constant its smearing of the pulses adds (seconds),
/// and the level that gets through.
const TURBINE_SMEAR: f32 = 0.0012;
const TURBINE_LEVEL: f32 = 0.75;
/// Turbo whistle: its pitch per shaft revolution per second, level at the shaft's speed
/// limit (at full load; `WHISTLE_IDLE` of it off throttle), its second harmonic, and how
/// much its breathiness wobbles it.
const WHISTLE_PITCH: f32 = 1.0;
const WHISTLE: f32 = 0.04;
const WHISTLE_IDLE: f32 = 0.3;
const WHISTLE_HARMONIC: f32 = 0.3;
const WHISTLE_BREATH: f32 = 0.4;
/// Highest whistle pitch (Hz), well below the Nyquist frequency.
const WHISTLE_MAX: f32 = 8000.0;

/// What the exhaust hardware does to the sound.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ExhaustSound {
    /// How much the silencer and the catalyst take out (0 open, 1 fully muffled).
    muffle: f32,
    /// Pipe diameter relative to the stock one for the engine's size.
    bore: f32,
    /// The header primaries' quarter-wave resonance (Hz).
    header: f32,
    /// 1 with a turbine in the exhaust, 0 without.
    turbine: f32,
}

impl ExhaustSound {
    fn of(hardware: &EngineHardware) -> Self {
        // The parts are sized with the engine: compare them with the stock ones for its
        // size.
        let size = relative_size(hardware);
        let stock = EngineHardware::default().exhaust;
        let exhaust = &hardware.exhaust;
        let muffling = |area: Option<f32>, stock: Option<f32>, stock_muffling: f32| match (
            area, stock,
        ) {
            (Some(area), Some(stock)) => {
                stock_muffling * (stock * size / area).powf(MUFFLER_POWER).min(1.0 / stock_muffling)
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

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<EngineSynth>()
            .add_systems(Startup, start_engine)
            .add_systems(Update, update_engine);
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
        targets
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
    /// The pulse after the turbine.
    smeared: f32,
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
            smeared: 0.0,
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
    }

    fn next(&mut self, noise: f32, shape: &Shape) -> f32 {
        let dt = 1.0 / SAMPLE_RATE as f32;
        let excitation = if self.since < 0.0 {
            0.0
        } else {
            let pulse = if self.since < PULSE {
                (PI * self.since / PULSE).sin().powi(2)
            } else {
                0.0
            };
            let combustion = 0.3 * shape.noise * noise * (-self.since / NOISE_DECAY).exp();
            self.strength * (pulse + combustion)
        };
        self.since += dt;
        self.smeared += shape.smoothing * (excitation - self.smeared);
        let mut out = 0.0;
        for (resonator, gain) in self.resonators.iter_mut().zip(shape.gains) {
            out += gain * resonator.process(self.smeared);
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
    /// Turbo shaft speed (revolutions per second) and its share of the limit, the
    /// whistle's phase (0..1), and the slow noise that makes it breathe.
    turbo_speed: f32,
    turbo_share: f32,
    whistle: f32,
    breath: f32,
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
            whistle: 0.0,
            breath: 0.0,
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
        glide(&mut self.exhaust.muffle, exhaust.muffle.clamp(0.0, 1.0), GLIDE);
        glide(&mut self.exhaust.bore, exhaust.bore.clamp(0.5, 2.0), GLIDE);
        glide(&mut self.exhaust.header, exhaust.header.clamp(50.0, 1000.0), GLIDE);
        glide(&mut self.exhaust.turbine, exhaust.turbine.clamp(0.0, 1.0), GLIDE);
        let (revolutions, share) = self.targets.turbo();
        glide(&mut self.turbo_speed, revolutions.max(0.0), GLIDE);
        glide(&mut self.turbo_share, share.clamp(0.0, 1.5), GLIDE);

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
        self.cycle = (self.cycle + self.rpm / 120.0 * firings as f32 / rate) % firings as f32;
        if self.cycle.floor() != before.floor() {
            let firing = self.cycle.floor() as usize % firings;
            let strength =
                (IDLE_STRENGTH + (1.0 - IDLE_STRENGTH) * self.load) * (1.0 + JITTER * self.noise());
            let bank = &mut self.banks[usize::from(self.sound.banks[firing])];
            bank.since = -bank.delay;
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
            smoothing: 1.0 / (1.0 + TURBINE_SMEAR * turbine * rate),
        };
        let pitch = bore.powf(-BORE_PITCH);
        for bank in &mut self.banks {
            bank.tune(pitch, header);
        }
        let left = self.noise();
        let right = self.noise();
        let left = self.banks[0].next(left, &shape);
        let right = self.banks[1].next(right, &shape);

        // The turbo whistles at its shaft speed, louder spooled up and under load.
        let frequency = (self.turbo_speed * WHISTLE_PITCH).min(WHISTLE_MAX / 2.0);
        self.whistle = (self.whistle + frequency / rate).fract();
        let breath = self.noise();
        self.breath += 0.01 * (breath - self.breath);
        let wobble = (1.0 + WHISTLE_BREATH * 10.0 * self.breath).max(0.0);
        let phase = TAU * self.whistle;
        let whistle = WHISTLE
            * self.turbo_share.powi(2)
            * (WHISTLE_IDLE + (1.0 - WHISTLE_IDLE) * load)
            * wobble
            * (phase.sin() + WHISTLE_HARMONIC * (2.0 * phase).sin());

        // Fewer cylinders pulse less often: level them out somewhat.
        let exhaust_level = (MUFFLED_LEVEL + (OPEN_LEVEL - MUFFLED_LEVEL) * open)
            * (1.0 + BORE_LEVEL * (bore - 1.0))
            * (1.0 - (1.0 - TURBINE_LEVEL) * turbine);
        let level = GAIN * self.volume * exhaust_level * (8.0 / firings as f32).sqrt();
        let whistle = GAIN * self.volume * whistle;
        let mix = |near: f32, far: f32| (level * (near + CROSSFEED * far) + whistle).tanh();
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

#[derive(Resource)]
struct Engine {
    targets: Arc<Targets>,
    muted: bool,
}

fn start_engine(mut commands: Commands, mut synths: ResMut<Assets<EngineSynth>>) {
    let targets = Arc::new(Targets::new());
    commands.spawn(AudioPlayer(synths.add(EngineSynth {
        targets: targets.clone(),
    })));
    commands.insert_resource(Engine {
        targets,
        muted: false,
    });
}

fn update_engine(
    keys: Res<ButtonInput<KeyCode>>,
    active: Res<ActiveCar>,
    cars: Query<&Car>,
    mut engine: ResMut<Engine>,
) {
    if keys.just_pressed(KeyCode::KeyM) {
        engine.muted = !engine.muted;
    }
    let Some(car) = cars.iter().find(|car| car.index == active.0) else {
        return;
    };
    let drivetrain = &car.drivetrain;
    let volume = if engine.muted || !drivetrain.running {
        0.0
    } else {
        1.0
    };
    engine.targets.set(drivetrain.rpm, drivetrain.load, volume);
    let layout = car.setup.engine.parts.family().layout;
    engine.targets.set_layout(layout);
    let hardware = &car.setup.engine.build.hardware;
    engine.targets.set_exhaust(ExhaustSound::of(hardware));
    let speed = drivetrain.engine.turbo_speed;
    let share = hardware.turbo.map_or(0.0, |turbo| speed / turbo.max_speed());
    engine.targets.set_turbo(speed / TAU, share);
}

#[cfg(test)]
mod tests {
    use super::*;

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
                assert!((sound.bore - 1.0).abs() < 0.05, "{}: {sound:?}", hardware.name);
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
        assert!(loudness(turbo, 1.0) > turbo_quiet * 1.01);
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
