//! Engine sound for the active car: a cross-plane V8, synthesised sample by sample from its
//! exhaust pulses. Each firing sends a pulse down its bank's exhaust, and a cross-plane
//! crank fires each bank at uneven intervals: that's the V8 burble. The pulses ring a few
//! fixed resonances of the exhaust, so the tone changes with engine speed rather than just
//! its pitch, with a little combustion noise on top. The banks are panned apart.
//!
//! The game only publishes targets (engine speed, load, volume); the synth glides towards
//! them every sample, so nothing steps audibly.

use std::f32::consts::{PI, TAU};
use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bevy::audio::{AddAudioSource, ChannelCount, Decodable, SampleRate, Source};
use bevy::prelude::*;

use crate::car::{ActiveCar, Car};

const SAMPLE_RATE: u32 = 44_100;
/// Firing order 1-8-4-3-6-5-7-2, 90° of crank apart; the odd cylinders are on the left bank.
/// Each bank fires at 90°, 180°, 270° and 180° intervals.
const LEFT_BANK: [bool; 8] = [true, false, false, true, false, true, true, false];
/// Firings per crank revolution.
const FIRINGS_PER_REV: f32 = 4.0;
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
const GAIN: f32 = 1.2;

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
}

impl Targets {
    fn set(&self, rpm: f32, load: f32, volume: f32) {
        self.rpm.store(rpm.to_bits(), Ordering::Relaxed);
        self.load.store(load.to_bits(), Ordering::Relaxed);
        self.volume.store(volume.to_bits(), Ordering::Relaxed);
    }

    fn get(&self) -> (f32, f32, f32) {
        let load = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        (load(&self.rpm), load(&self.load), load(&self.volume))
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
    fn new(frequency: f32, q: f32) -> Self {
        let w = TAU * frequency / SAMPLE_RATE as f32;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        Self {
            b0: alpha / a0,
            a1: -2.0 * w.cos() / a0,
            a2: (1.0 - alpha) / a0,
            ..default()
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * (x - self.x[1]) - self.a1 * self.y[0] - self.a2 * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
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
    resonators: [Resonator; 4],
    /// Last input and output of the DC blocker.
    dc: (f32, f32),
}

impl Bank {
    /// A bank with its resonances `pipes` times the base frequencies.
    fn new(pipes: f32, delay: f32) -> Self {
        Self {
            since: 1.0,
            strength: 0.0,
            delay,
            resonators: RESONANCES.map(|(frequency, q, _, _)| Resonator::new(frequency * pipes, q)),
            dc: (0.0, 0.0),
        }
    }

    fn next(&mut self, noise: f32, load: f32) -> f32 {
        let dt = 1.0 / SAMPLE_RATE as f32;
        let excitation = if self.since < 0.0 {
            0.0
        } else {
            let pulse = if self.since < PULSE {
                (PI * self.since / PULSE).sin().powi(2)
            } else {
                0.0
            };
            self.strength * (pulse + 0.3 * noise * (-self.since / NOISE_DECAY).exp())
        };
        self.since += dt;
        let mut out = 0.0;
        for (resonator, &(_, _, off, full)) in self.resonators.iter_mut().zip(&RESONANCES) {
            out += (off + (full - off) * load) * resonator.process(excitation);
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
    /// Position in the four-stroke cycle, in firings (0 to 8).
    cycle: f32,
    banks: [Bank; 2],
    /// State of the noise generator.
    seed: u32,
    /// The right sample, waiting for its turn after the left one.
    right: Option<f32>,
}

impl EngineVoice {
    fn new(targets: Arc<Targets>) -> Self {
        Self {
            targets,
            rpm: 0.0,
            load: 0.0,
            volume: 0.0,
            cycle: 0.0,
            banks: [Bank::new(1.0, 0.0), Bank::new(RIGHT_PIPES, RIGHT_DELAY)],
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
        let (rpm, load, volume) = self.targets.get();
        self.rpm += (rpm.max(0.0) - self.rpm) / (GLIDE * rate);
        self.load += (load.clamp(0.0, 1.0) - self.load) / (LOAD_GLIDE * rate);
        self.volume += (volume - self.volume) / (GLIDE * rate);

        let before = self.cycle;
        self.cycle = (self.cycle + self.rpm / 60.0 * FIRINGS_PER_REV / rate) % 8.0;
        if self.cycle.floor() != before.floor() {
            let cylinder = self.cycle.floor() as usize % 8;
            let strength =
                (IDLE_STRENGTH + (1.0 - IDLE_STRENGTH) * self.load) * (1.0 + JITTER * self.noise());
            let bank = &mut self.banks[usize::from(!LEFT_BANK[cylinder])];
            bank.since = -bank.delay;
            bank.strength = strength;
        }

        let load = self.load;
        let left = self.noise();
        let right = self.noise();
        let left = self.banks[0].next(left, load);
        let right = self.banks[1].next(right, load);
        let level = GAIN * self.volume;
        let mix = |near: f32, far: f32| (level * (near + CROSSFEED * far)).tanh();
        (mix(left, right), mix(right, left))
    }
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
    let targets = Arc::new(Targets::default());
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_bank_fires_unevenly() {
        // Firing slots of each bank in the 8-slot cycle, and the gaps between them.
        for left in [true, false] {
            let slots: Vec<usize> = (0..8).filter(|&i| LEFT_BANK[i] == left).collect();
            assert_eq!(slots.len(), 4);
            let mut gaps: Vec<usize> = (0..4)
                .map(|i| (slots[(i + 1) % 4] + 8 - slots[i]) % 8)
                .collect();
            gaps.sort();
            // In slots of 90° crank: 90°, 180°, 180° and 270°.
            assert_eq!(gaps, [1, 2, 2, 3], "left bank {left}");
        }
    }

    #[test]
    fn stays_in_range_and_glides() {
        let targets = Arc::new(Targets::default());
        let mut voice = EngineVoice::new(targets.clone());
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
            assert!(peak > 0.05, "{rpm} rpm: peak {peak}");
            assert!((voice.rpm - rpm).abs() < 1.0);
        }
    }

    #[test]
    fn silent_when_off() {
        let targets = Arc::new(Targets::default());
        let mut voice = EngineVoice::new(targets.clone());
        targets.set(850.0, 0.0, 0.0);
        for _ in 0..SAMPLE_RATE / 10 {
            assert_eq!(voice.frame(), (0.0, 0.0));
        }
    }
}
