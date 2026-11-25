//! The ways a car's sound reaches the listener, besides straight there: off the road
//! outside, and through the car's body from the driver's seat.
//!
//! Outside, the road reflects the exhaust: a second copy arrives a little later, by the
//! longer way via the ground, and the two cancel and add at frequencies spaced by that
//! delay. Behind our own car it stays put; a car passing by sweeps it (the flanging of a
//! pass-by).
//!
//! Inside, the body passes the lows and keeps out the highs (a little leaks through the
//! glass and the seals), the cabin booms at its length's, width's and height's
//! resonances, sound reflects off the glass and the panels within a few milliseconds,
//! and both ears hear much the same. Switching views crossfades between the two. Each
//! car's cabin is its own (`CabinShape`): its size sets the booming, how well it's
//! insulated and how near the engine sits how much comes through and how bright; an
//! open car is mostly heard as from outside.

use bevy::math::Vec3;
use std::f32::consts::TAU;
use std::sync::atomic::AtomicU32;

use super::{Resonator, SAMPLE_RATE, read, store};
use crate::chassis::Chassis;

/// Height of the tailpipe above the road (m), and how high above the road a car's
/// reference point is (its sound is placed there).
const TAILPIPE: f32 = 0.35;
const CAR_HEIGHT: f32 = 0.5;
const SOUND_SPEED: f32 = 343.0;
/// How much of the sound the road reflects, and the highs it soaks up above this (Hz).
const ROAD: f32 = 0.7;
const ROAD_LOSS: f32 = 6000.0;
/// Longest way round via the road (samples): a listener far above the car close by.
const GROUND_LINE: usize = 512;
/// The body's transmission: what's left above this (Hz) falls away steeply (two poles),
/// and the share of the whole sound leaking in regardless; with no insulation and with
/// the most (`Chassis::insulation`), the engine `ENGINE_NEAR` (m) from the driver's eyes.
/// Nearer, more comes through and brighter (with the nearness; the cutoff with its square
/// root), within `NEARNESS`.
const TRANSMISSION: (f32, f32) = (700.0, 350.0);
const LEAK: (f32, f32) = (0.3, 0.08);
const ENGINE_NEAR: f32 = 1.5;
const NEARNESS: (f32, f32) = (0.5, 2.0);
/// The cabin's resonances, lowest first: Q and gain. Their frequencies are those of
/// waves fitting the cabin's length, width and height (the cabin outline's run times
/// `CABIN_RUN`, the body's width times `CABIN_WIDTH`, the roof over the floor less
/// `CABIN_FLOOR`).
const MODES: [(f32, f32); 3] = [(5.0, 0.8), (6.0, 0.5), (6.0, 0.35)];
const CABIN_RUN: f32 = 0.8;
const CABIN_WIDTH: f32 = 0.85;
const CABIN_FLOOR: f32 = 0.15;
/// How much of the cabin's sound an open car has.
const OPEN: f32 = 0.35;
/// Early reflections off the glass and panels: when (s) and how strong, per ear.
const REFLECTIONS: [[(f32, f32); 4]; 2] = [
    [
        (0.0013, 0.45),
        (0.0031, 0.3),
        (0.0047, -0.22),
        (0.0068, 0.15),
    ],
    [
        (0.0017, 0.4),
        (0.0026, 0.32),
        (0.0055, -0.2),
        (0.0072, 0.14),
    ],
];
/// Longest reflection (samples).
const CABIN_LINE: usize = 512;
/// How much of the difference between the ears is left inside.
const WIDTH: f32 = 0.35;
/// Overall level inside, relative to outside.
const INSIDE_LEVEL: f32 = 0.8;
/// How long switching views takes (s), along an S-curve.
const CROSSFADE: f32 = 0.3;

/// What a car's cabin does to sound.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct CabinShape {
    /// The resonances (Hz), lowest first.
    pub modes: [f32; 3],
    /// The body's transmission cutoff (Hz) and the share leaking through regardless.
    pub transmission: f32,
    pub leak: f32,
    /// 1 closed, `OPEN` without a roof.
    pub enclosed: f32,
}

impl CabinShape {
    /// A hatchback's, for tests and before there's a car.
    pub const DEFAULT: Self = Self {
        modes: [72.0, 118.0, 155.0],
        transmission: 450.0,
        leak: 0.15,
        enclosed: 1.0,
    };

    pub(super) fn of(chassis: &Chassis) -> Self {
        let body = &chassis.body;
        let closed = body.cabin.len() > 2;
        let (run, roof) = match (body.cabin.first(), body.cabin.last()) {
            (Some(first), Some(last)) => (
                CABIN_RUN * (last[0] - first[0]),
                body.cabin.iter().map(|p| p[1]).fold(0.0, f32::max),
            ),
            _ => (CABIN_RUN * chassis.wheelbase, chassis.eye[1] + 0.2),
        };
        let height = (roof - body.floor - CABIN_FLOOR).max(0.6);
        let mut modes =
            [run.max(1.0), CABIN_WIDTH * body.width, height].map(|size| SOUND_SPEED / (2.0 * size));
        modes.sort_by(f32::total_cmp);
        let engine = body.front_overhang + chassis.bay.engine_at;
        let distance = (engine - chassis.eye[0]).abs().max(0.3);
        let nearness = (ENGINE_NEAR / distance).clamp(NEARNESS.0, NEARNESS.1);
        let insulation = chassis.insulation.clamp(0.0, 1.0);
        Self {
            modes,
            transmission: (TRANSMISSION.0 + (TRANSMISSION.1 - TRANSMISSION.0) * insulation)
                * nearness.sqrt(),
            leak: (LEAK.0 + (LEAK.1 - LEAK.0) * insulation) * nearness,
            enclosed: if closed { 1.0 } else { OPEN },
        }
    }
}

/// Where everything is heard from, shared by every synth: from the driver's seat or
/// outside, and the cabin of the car driven.
pub(super) struct Listener {
    inside: AtomicU32,
    shape: [AtomicU32; 6],
}

impl Default for Listener {
    fn default() -> Self {
        let listener = Self {
            inside: AtomicU32::default(),
            shape: Default::default(),
        };
        listener.set(false, CabinShape::DEFAULT);
        listener
    }
}

impl Listener {
    pub(super) fn set(&self, inside: bool, shape: CabinShape) {
        store(&self.inside, if inside { 1.0 } else { 0.0 });
        let [a, b, c] = shape.modes;
        let values = [a, b, c, shape.transmission, shape.leak, shape.enclosed];
        for (atomic, value) in self.shape.iter().zip(values) {
            store(atomic, value);
        }
    }

    pub(super) fn get(&self) -> (f32, CabinShape) {
        let [a, b, c, transmission, leak, enclosed] = self.shape.each_ref().map(read);
        let shape = CabinShape {
            modes: [a, b, c].map(|m| m.clamp(20.0, 1000.0)),
            transmission: transmission.clamp(50.0, 5000.0),
            leak: leak.clamp(0.0, 1.0),
            enclosed: enclosed.clamp(0.0, 1.0),
        };
        (read(&self.inside), shape)
    }
}

/// Keeps a sample within -1 to 1, unchanged below 0.8 and bending smoothly towards 1 above.
pub(super) fn limit(x: f32) -> f32 {
    let over = x.abs() - 0.8;
    if over <= 0.0 {
        x
    } else {
        x.signum() * (0.8 + 0.2 * (over / 0.2).tanh())
    }
}

/// How much later the road's reflection of a car at `car` arrives at `ear` than the
/// sound straight from it (s).
pub(super) fn ground_delay(car: Vec3, ear: Vec3) -> f32 {
    let across = (ear - car).with_y(0.0).length();
    let ear_height = (ear.y - car.y + CAR_HEIGHT).max(0.3);
    let via_road = across.hypot(TAILPIPE + ear_height);
    let straight = across.hypot(TAILPIPE - ear_height);
    (via_road - straight) / SOUND_SPEED
}

/// A short delay line read between samples.
#[derive(Clone, Copy)]
struct Line<const N: usize> {
    samples: [f32; N],
    at: usize,
}

impl<const N: usize> Line<N> {
    fn new() -> Self {
        Self {
            samples: [0.0; N],
            at: 0,
        }
    }

    fn push(&mut self, x: f32) {
        self.samples[self.at] = x;
        self.at = (self.at + 1) % N;
    }

    /// What went in `lag` samples before the latest.
    fn read(&self, lag: f32) -> f32 {
        let back = (self.at + N - 1) as f32 - lag.clamp(0.0, (N - 2) as f32);
        let i = back as usize;
        let share = back.fract();
        let (a, b) = (self.samples[i % N], self.samples[(i + 1) % N]);
        a + (b - a) * share
    }
}

/// The road's reflection of one channel.
#[derive(Clone, Copy)]
pub(super) struct Ground {
    line: Line<GROUND_LINE>,
    /// The delay (samples), gliding, and the reflection's low-pass.
    lag: f32,
    lowpass: f32,
}

impl Ground {
    pub(super) fn new() -> Self {
        Self {
            line: Line::new(),
            lag: 0.0,
            lowpass: 0.0,
        }
    }

    /// `x` with the road's reflection `delay` seconds behind it (0: none).
    pub(super) fn process(&mut self, x: f32, delay: f32) -> f32 {
        let rate = SAMPLE_RATE as f32;
        self.line.push(x);
        // Glides over about 20 ms, so a car passing sweeps it smoothly.
        self.lag += (delay * rate - self.lag) / (0.02 * rate);
        if delay <= 0.0 {
            return x;
        }
        let share = 1.0 - (-TAU * ROAD_LOSS / rate).exp();
        self.lowpass += share * (self.line.read(self.lag) - self.lowpass);
        // Scaled so the lows, where the two add up, stay as loud as without the road.
        (x + ROAD * self.lowpass) / (1.0 + ROAD)
    }
}

/// One ear's way through the body.
#[derive(Clone, Copy)]
struct Ear {
    transmitted: [f32; 2],
    modes: [Resonator; 3],
    line: Line<CABIN_LINE>,
}

/// What the cabin does to a stereo sound, crossfaded in and out as the view changes.
pub(super) struct Cabin {
    ears: [Ear; 2],
    /// The resonances the modes are tuned to.
    tuned: [f32; 3],
    /// 0 outside, 1 inside, moving steadily over `CROSSFADE`; heard along an S-curve.
    inside: f32,
}

impl Cabin {
    pub(super) fn new() -> Self {
        let ear = Ear {
            transmitted: [0.0; 2],
            modes: [Resonator::default(); 3],
            line: Line::new(),
        };
        let mut cabin = Self {
            ears: [ear; 2],
            tuned: [0.0; 3],
            inside: 0.0,
        };
        cabin.tune(CabinShape::DEFAULT.modes);
        cabin
    }

    fn tune(&mut self, modes: [f32; 3]) {
        if modes == self.tuned {
            return;
        }
        self.tuned = modes;
        for ear in &mut self.ears {
            for (mode, (&frequency, &(q, _))) in ear.modes.iter_mut().zip(modes.iter().zip(&MODES))
            {
                mode.tune(frequency, q);
            }
        }
    }

    /// The sound `(left, right)` as heard inside `shape`'s cabin to the share `inside`
    /// asks for (0 or 1; it crossfades).
    pub(super) fn process(
        &mut self,
        (left, right): (f32, f32),
        inside: f32,
        shape: &CabinShape,
    ) -> (f32, f32) {
        let rate = SAMPLE_RATE as f32;
        let step = 1.0 / (CROSSFADE * rate);
        let target = inside.clamp(0.0, 1.0) * shape.enclosed;
        self.inside += (target - self.inside).clamp(-step, step);
        if self.inside < 1e-4 && target <= 0.0 {
            return (left, right);
        }
        self.tune(shape.modes);
        let share = 1.0 - (-TAU * shape.transmission / rate).exp();
        let mut heard = [0.0; 2];
        for ((ear, x), (out, reflections)) in self
            .ears
            .iter_mut()
            .zip([left, right])
            .zip(heard.iter_mut().zip(&REFLECTIONS))
        {
            ear.transmitted[0] += share * (x - ear.transmitted[0]);
            ear.transmitted[1] += share * (ear.transmitted[0] - ear.transmitted[1]);
            let through = ear.transmitted[1] + shape.leak * x;
            let mut boom = 0.0;
            for (mode, &(_, gain)) in ear.modes.iter_mut().zip(&MODES) {
                boom += gain * mode.process(through);
            }
            let direct = through + boom;
            ear.line.push(direct);
            let mut reflected = 0.0;
            for &(delay, gain) in reflections {
                reflected += gain * ear.line.read(delay * rate);
            }
            *out = INSIDE_LEVEL * (direct + reflected);
        }
        let middle = 0.5 * (heard[0] + heard[1]);
        let side = 0.5 * WIDTH * (heard[0] - heard[1]);
        let inside = (middle + side, middle - side);
        let fade = self.inside * self.inside * (3.0 - 2.0 * self.inside);
        (
            left + (inside.0 - left) * fade,
            right + (inside.1 - right) * fade,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_road_reflection_comes_later_the_closer_and_higher() {
        let car = Vec3::ZERO;
        let behind = ground_delay(car, Vec3::new(0.0, 2.5, 9.0));
        // About 0.2 m further: 0.6 ms.
        assert!((0.0004..0.0009).contains(&behind), "{behind}");
        assert!(ground_delay(car, Vec3::new(0.0, 2.5, 30.0)) < 0.5 * behind);
        assert!(ground_delay(car, Vec3::new(0.0, 5.0, 9.0)) > behind);
    }

    /// RMS of a sine at `frequency` through the cabin, fully inside.
    fn through(frequency: f32) -> f32 {
        let mut cabin = Cabin::new();
        let rate = SAMPLE_RATE as f32;
        let mut sum = 0.0;
        let n = SAMPLE_RATE as usize;
        for i in 0..2 * n {
            let x = (TAU * frequency * i as f32 / rate).sin();
            let (left, right) = cabin.process((x, x), 1.0, &CabinShape::DEFAULT);
            if i >= n {
                sum += left * left + right * right;
            }
        }
        (sum / (2 * n) as f32).sqrt()
    }

    #[test]
    fn the_cabin_booms_and_keeps_out_the_highs() {
        let boom = through(72.0);
        let middle = through(400.0);
        let high = through(3000.0);
        assert!(boom > 1.5 * middle, "{boom} {middle}");
        assert!(high < 0.4 * middle, "{high} {middle}");
    }

    #[test]
    fn every_cabin_is_its_own() {
        let shape = |name: &str| {
            CabinShape::of(
                crate::catalogue::builtin()
                    .chassis
                    .iter()
                    .find(|c| c.name == name)
                    .unwrap(),
            )
        };
        for chassis in &crate::catalogue::builtin().chassis {
            let cabin = CabinShape::of(chassis);
            assert!(cabin.modes.is_sorted(), "{}", chassis.name);
            assert!(
                cabin.modes.iter().all(|m| (40.0..300.0).contains(m)),
                "{} {cabin:?}",
                chassis.name
            );
            assert!(
                cabin.leak > 0.0 && cabin.leak < 0.7,
                "{} {cabin:?}",
                chassis.name
            );
        }
        // A van booms lower than a city car.
        assert!(shape("Carrow").modes[0] < shape("Minim").modes[0]);
        // A saloon keeps out more than a stripped rally car.
        assert!(shape("Halden").leak < 0.5 * shape("Skarn").leak);
        // The engine right behind the driver comes through brighter than one up front.
        assert!(shape("Calder").transmission > shape("Harrow").transmission);
        // Without a roof, it's mostly heard as from outside.
        assert_eq!(shape("Linnet").enclosed, OPEN);
        assert_eq!(shape("Wisp").enclosed, OPEN);
        assert_eq!(shape("Halden").enclosed, 1.0);
    }

    #[test]
    fn outside_the_cabin_changes_nothing() {
        let mut cabin = Cabin::new();
        for i in 0..1000 {
            let x = (i as f32 * 0.1).sin();
            assert_eq!(cabin.process((x, -x), 0.0, &CabinShape::DEFAULT), (x, -x));
        }
    }
}
