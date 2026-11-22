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

/// What the driven car's tires and gearbox do, heard from the camera.
#[allow(clippy::too_many_arguments)]
fn cue(
    time: Res<Time>,
    effects: Res<Effects>,
    engine: Res<EngineAudio>,
    active: Res<ActiveCar>,
    rescued: Res<Rescued>,
    cars: Query<(&Car, &GlobalTransform)>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut refused: MessageReader<ReverseRefused>,
    mut burst: Local<f32>,
) {
    let cues = &effects.cues;
    let quiet = engine.muted || engine.hushed;
    store(&cues.volume, if engine.muted { 0.0 } else { 1.0 });
    *burst = (*burst - time.delta_secs()).max(0.0);
    if refused.read().count() > 0 {
        *burst = GRIND_BURST;
    }
    let car = cars.iter().find(|(car, _)| car.index == active.0);
    let (Some((car, transform)), false, false) = (car, quiet, rescued.0) else {
        for wheel in 0..4 {
            cues.set_squeal(wheel, 0.0, 1.0, 0.0);
        }
        cues.set_grind(0.0, CLASH_RATE.0);
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
    /// listening: a countdown, a slide that builds up and lets go, and a grind.
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
