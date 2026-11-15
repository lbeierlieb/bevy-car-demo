//! Engine sound for the active car, synthesised sample by sample: pitch follows the rear
//! wheels' spin, volume follows the throttle. The game only publishes targets; the synth
//! glides towards them every sample, so pitch and volume change smoothly.

use std::f32::consts::TAU;
use std::num::NonZero;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bevy::audio::{AddAudioSource, ChannelCount, Decodable, SampleRate, Source};
use bevy::prelude::*;

use crate::car::{ActiveCar, Car};

const SAMPLE_RATE: u32 = 44_100;
/// Firing frequency at standstill, and how much it rises per rad/s of rear wheel spin
/// (about 230 Hz, a four-cylinder at 7000 rpm, at 40 m/s).
const IDLE_HZ: f32 = 35.0;
const HZ_PER_WHEEL_RAD: f32 = 1.8;
/// Time constant of the glide towards new targets.
const GLIDE: f32 = 0.05;

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
    frequency: AtomicU32,
    volume: AtomicU32,
}

impl Targets {
    fn set(&self, frequency: f32, volume: f32) {
        self.frequency.store(frequency.to_bits(), Ordering::Relaxed);
        self.volume.store(volume.to_bits(), Ordering::Relaxed);
    }

    fn get(&self) -> (f32, f32) {
        let load = |a: &AtomicU32| f32::from_bits(a.load(Ordering::Relaxed));
        (load(&self.frequency), load(&self.volume))
    }
}

#[derive(Asset, TypePath, Clone)]
struct EngineSynth {
    targets: Arc<Targets>,
}

impl Decodable for EngineSynth {
    type Decoder = EngineVoice;

    fn decoder(&self) -> EngineVoice {
        EngineVoice {
            targets: self.targets.clone(),
            frequency: IDLE_HZ,
            volume: 0.0,
            phase: 0.0,
        }
    }
}

/// An endless mono stream: a few harmonics of the firing frequency plus a half-order
/// component for the uneven rumble of a real engine.
struct EngineVoice {
    targets: Arc<Targets>,
    frequency: f32,
    volume: f32,
    /// Position in firing cycles, wrapping at 2 so the half-order component stays continuous.
    phase: f32,
}

impl Iterator for EngineVoice {
    type Item = f32;

    fn next(&mut self) -> Option<f32> {
        let (frequency, volume) = self.targets.get();
        let blend = 1.0 / (GLIDE * SAMPLE_RATE as f32);
        self.frequency += (frequency.max(1.0) - self.frequency) * blend;
        self.volume += (volume - self.volume) * blend;
        self.phase = (self.phase + self.frequency / SAMPLE_RATE as f32) % 2.0;

        let p = TAU * self.phase;
        let tone =
            p.sin() + 0.5 * (2.0 * p).sin() + 0.25 * (3.0 * p).sin() + 0.12 * (4.0 * p).sin();
        let rumble = 0.4 * (0.5 * p).sin();
        Some(0.3 * self.volume * (tone + rumble))
    }
}

impl Source for EngineVoice {
    fn current_span_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> ChannelCount {
        NonZero::new(1).unwrap()
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
    targets.set(IDLE_HZ, 0.0);
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
    let wheel = (car.corners[2].omega.abs() + car.corners[3].omega.abs()) / 2.0;
    let volume = if engine.muted {
        0.0
    } else {
        0.25 + 0.75 * car.controls.throttle.abs()
    };
    engine
        .targets
        .set(IDLE_HZ + wheel * HZ_PER_WHEEL_RAD, volume);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_glides_without_steps_or_clipping() {
        let synth = EngineSynth { targets: Arc::new(Targets::default()) };
        let mut voice = synth.decoder();
        // Jump from idle to a high note at full throttle.
        synth.targets.set(IDLE_HZ, 1.0);
        voice.by_ref().take(SAMPLE_RATE as usize).for_each(drop);
        synth.targets.set(220.0, 1.0);
        let mut last = voice.frequency;
        for _ in 0..SAMPLE_RATE {
            let sample = voice.next().unwrap();
            assert!(sample.abs() <= 1.0);
            // Each sample moves the pitch by a tiny fraction of a hertz.
            assert!((voice.frequency - last).abs() < 0.1);
            last = voice.frequency;
        }
        // After a second (20 time constants) it has arrived, to within f32 rounding of the
        // tiny per-sample correction (a few hundredths of a hertz).
        assert!((voice.frequency - 220.0).abs() < 0.05);
    }
}
