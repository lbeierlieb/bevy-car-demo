//! Response curves drawn by points: how far a control is pushed (0 to 1) against what it
//! asks for (0 to 1). The points are joined by a monotone cubic (Fritsch–Carlson: smooth,
//! never overshooting between points), and that is baked into a table once, whenever the
//! points change, so reading a control is one lookup and a lerp.

use std::fmt;

/// The table's steps over the push.
const STEPS: usize = 256;
/// The most points a curve has.
pub const MOST_POINTS: usize = 9;
/// The least distance between two points' pushes.
pub const MIN_GAP: f32 = 0.02;

/// A curve: points from push 0 to push 1, and the table baked from them.
#[derive(Clone)]
pub struct Curve {
    points: Vec<[f32; 2]>,
    table: Box<[f32; STEPS + 1]>,
}

impl Curve {
    /// A curve through these points (sorted by push, the first moved to push 0 and the
    /// last to 1, everything kept within 0 to 1; fewer than two make it linear).
    pub fn new(points: &[[f32; 2]]) -> Self {
        let mut points: Vec<[f32; 2]> = points
            .iter()
            .filter(|p| p.iter().all(|v| v.is_finite()))
            .map(|&[x, y]| [x.clamp(0.0, 1.0), y.clamp(0.0, 1.0)])
            .collect();
        points.sort_by(|a, b| a[0].total_cmp(&b[0]));
        points.dedup_by(|b, a| b[0] - a[0] < MIN_GAP);
        points.truncate(MOST_POINTS);
        if points.len() < 2 {
            points = vec![[0.0, 0.0], [1.0, 1.0]];
        }
        points[0][0] = 0.0;
        let last = points.len() - 1;
        points[last][0] = 1.0;
        // Moving the ends may have squeezed their neighbours.
        if points.len() > 2 && (points[last][0] - points[last - 1][0] < MIN_GAP) {
            points.remove(last - 1);
        }
        if points.len() > 2 && (points[1][0] < MIN_GAP) {
            points.remove(1);
        }
        let table = bake(&points);
        Self { points, table }
    }

    pub fn linear() -> Self {
        Self::new(&[[0.0, 0.0], [1.0, 1.0]])
    }

    /// push^exponent, through five points (the curves before they were drawn).
    pub fn power(exponent: f32) -> Self {
        let exponent = exponent.max(0.1);
        let points: Vec<[f32; 2]> = (0..=4)
            .map(|i| {
                let x = i as f32 / 4.0;
                [x, (x.powf(exponent) * 1e4).round() / 1e4]
            })
            .collect();
        Self::new(&points)
    }

    pub fn points(&self) -> &[[f32; 2]] {
        &self.points
    }

    /// What a push (-1 to 1) asks for, the same either way.
    pub fn at(&self, push: f32) -> f32 {
        let x = push.abs().clamp(0.0, 1.0) * STEPS as f32;
        let i = (x as usize).min(STEPS - 1);
        let t = x - i as f32;
        let value = self.table[i] + (self.table[i + 1] - self.table[i]) * t;
        value.copysign(push)
    }

    /// Point `i` moved to `to`, kept between its neighbours (the ends stay at push 0
    /// and 1).
    pub fn set_point(&mut self, i: usize, [x, y]: [f32; 2]) {
        let Some(&old) = self.points.get(i) else {
            return;
        };
        let last = self.points.len() - 1;
        let x = if i == 0 || i == last {
            old[0]
        } else {
            x.clamp(
                self.points[i - 1][0] + MIN_GAP,
                self.points[i + 1][0] - MIN_GAP,
            )
        };
        self.points[i] = [x, y.clamp(0.0, 1.0)];
        self.table = bake(&self.points);
    }

    /// A point added in the middle of the widest gap, on the curve: its index.
    pub fn split_widest(&mut self) -> Option<usize> {
        if self.points.len() >= MOST_POINTS {
            return None;
        }
        let i = (1..self.points.len()).max_by(|&a, &b| {
            let gap = |i: usize| self.points[i][0] - self.points[i - 1][0];
            gap(a).total_cmp(&gap(b))
        })?;
        let x = (self.points[i - 1][0] + self.points[i][0]) / 2.0;
        self.insert([x, self.at(x)])
    }

    /// A point added at `point` (not too near another): its index.
    pub fn insert(&mut self, [x, y]: [f32; 2]) -> Option<usize> {
        if self.points.len() >= MOST_POINTS {
            return None;
        }
        let i = self.points.iter().position(|p| p[0] > x)?;
        if i == 0 || x - self.points[i - 1][0] < MIN_GAP || self.points[i][0] - x < MIN_GAP {
            return None;
        }
        self.points.insert(i, [x, y.clamp(0.0, 1.0)]);
        self.table = bake(&self.points);
        Some(i)
    }

    /// Point `i` taken out (never an end). Whether it was.
    pub fn remove(&mut self, i: usize) -> bool {
        if i == 0 || i + 1 >= self.points.len() {
            return false;
        }
        self.points.remove(i);
        self.table = bake(&self.points);
        true
    }

    /// The points as text: `x,y x,y …`.
    pub fn to_text(&self) -> String {
        let points: Vec<String> = self
            .points
            .iter()
            .map(|[x, y]| format!("{x},{y}"))
            .collect();
        points.join(" ")
    }

    /// Points written by `to_text`; none if any is broken.
    pub fn from_words<'a>(words: impl Iterator<Item = &'a str>) -> Option<Self> {
        let points = words
            .map(|word| {
                let (x, y) = word.split_once(',')?;
                Some([x.parse().ok()?, y.parse().ok()?])
            })
            .collect::<Option<Vec<[f32; 2]>>>()?;
        (points.len() >= 2).then(|| Self::new(&points))
    }
}

impl PartialEq for Curve {
    fn eq(&self, other: &Self) -> bool {
        self.points == other.points
    }
}

impl fmt::Debug for Curve {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Curve({})", self.to_text())
    }
}

/// The monotone cubic through `points` (sorted, from push 0 to 1), sampled into the
/// table.
fn bake(points: &[[f32; 2]]) -> Box<[f32; STEPS + 1]> {
    let n = points.len();
    let secant: Vec<f32> = points
        .windows(2)
        .map(|w| (w[1][1] - w[0][1]) / (w[1][0] - w[0][0]))
        .collect();
    // Tangents: the secants' mean inside, flat at a peak or dip, and limited so the curve
    // stays monotone between two points.
    let mut tangent = vec![0.0; n];
    tangent[0] = secant[0];
    tangent[n - 1] = secant[n - 2];
    for i in 1..n - 1 {
        tangent[i] = if secant[i - 1] * secant[i] <= 0.0 {
            0.0
        } else {
            (secant[i - 1] + secant[i]) / 2.0
        };
    }
    for (i, &d) in secant.iter().enumerate() {
        if d == 0.0 {
            tangent[i] = 0.0;
            tangent[i + 1] = 0.0;
            continue;
        }
        let (a, b) = (tangent[i] / d, tangent[i + 1] / d);
        let length = a.hypot(b);
        if length > 3.0 {
            let scale = 3.0 / length;
            tangent[i] = scale * a * d;
            tangent[i + 1] = scale * b * d;
        }
    }
    let mut table = Box::new([0.0; STEPS + 1]);
    let mut segment = 0;
    for (k, value) in table.iter_mut().enumerate() {
        let x = k as f32 / STEPS as f32;
        while segment + 2 < n && x > points[segment + 1][0] {
            segment += 1;
        }
        let ([x0, y0], [x1, y1]) = (points[segment], points[segment + 1]);
        let h = x1 - x0;
        let t = ((x - x0) / h).clamp(0.0, 1.0);
        let (t2, t3) = (t * t, t * t * t);
        *value = ((2.0 * t3 - 3.0 * t2 + 1.0) * y0
            + (t3 - 2.0 * t2 + t) * h * tangent[segment]
            + (-2.0 * t3 + 3.0 * t2) * y1
            + (t3 - t2) * h * tangent[segment + 1])
            .clamp(0.0, 1.0);
    }
    table
}

/// Ready-made curves to start from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    /// As pushed: a wheel and pedals.
    Linear,
    /// A little finer near the rest position (push^1.25): the gas on a trigger.
    Gentle,
    /// push^1.5: the brake on a trigger.
    Progressive,
    /// push^1.8: steering with a stick.
    Stick,
    /// push^2.6: a twitchy stick, or a very quick car.
    Precise,
    /// Fine at both ends, quick in the middle.
    Smooth,
    /// Quick off the rest position, easing to the end: short-travel triggers or pedals.
    Quick,
}

impl Preset {
    pub const ALL: [Preset; 7] = [
        Preset::Linear,
        Preset::Gentle,
        Preset::Progressive,
        Preset::Stick,
        Preset::Precise,
        Preset::Smooth,
        Preset::Quick,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Preset::Linear => "Linear",
            Preset::Gentle => "Gentle",
            Preset::Progressive => "Progressive",
            Preset::Stick => "Stick",
            Preset::Precise => "Precise",
            Preset::Smooth => "Smooth (S)",
            Preset::Quick => "Quick",
        }
    }

    pub fn curve(self) -> Curve {
        match self {
            Preset::Linear => Curve::linear(),
            Preset::Gentle => Curve::power(1.25),
            Preset::Progressive => Curve::power(1.5),
            Preset::Stick => Curve::power(1.8),
            Preset::Precise => Curve::power(2.6),
            Preset::Smooth => Curve::new(&[[0.0, 0.0], [0.25, 0.08], [0.75, 0.92], [1.0, 1.0]]),
            Preset::Quick => Curve::new(&[[0.0, 0.0], [0.3, 0.48], [0.7, 0.9], [1.0, 1.0]]),
        }
    }

    /// The preset this curve is, if any.
    pub fn of(curve: &Curve) -> Option<Preset> {
        Self::ALL.into_iter().find(|p| p.curve() == *curve)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_curve_goes_through_its_points_and_never_overshoots() {
        for preset in Preset::ALL {
            let curve = preset.curve();
            for &[x, y] in curve.points() {
                assert!((curve.at(x) - y).abs() < 2e-3, "{preset:?} at {x}");
            }
            // Rising points: a rising curve.
            let mut last = 0.0;
            for k in 0..=100 {
                let v = curve.at(k as f32 / 100.0);
                assert!(v >= last - 1e-6, "{preset:?} falls at {k}");
                last = v;
            }
            assert_eq!(curve.at(-0.5), -curve.at(0.5));
            assert_eq!(curve.at(1.0), 1.0);
            assert_eq!(curve.at(0.0), 0.0);
        }
        // A power curve's points lie on it, and between them it stays close.
        let stick = Preset::Stick.curve();
        for k in 0..=20 {
            let x = k as f32 / 20.0;
            assert!((stick.at(x) - x.powf(1.8)).abs() < 0.02, "{x}");
        }
    }

    #[test]
    fn points_stay_in_order_and_ends_stay_put() {
        let mut curve = Preset::Stick.curve();
        curve.set_point(0, [0.5, 0.1]);
        assert_eq!(curve.points()[0], [0.0, 0.1]);
        curve.set_point(1, [0.9, 2.0]);
        assert_eq!(curve.points()[1], [0.5 - MIN_GAP, 1.0]);
        let i = curve.split_widest().unwrap();
        assert_eq!(curve.points().len(), 6);
        assert!(curve.remove(i));
        assert!(!curve.remove(0));
        assert!(!curve.remove(curve.points().len() - 1));
        let back = Curve::from_words(curve.to_text().split_whitespace()).unwrap();
        assert_eq!(back, curve);
        // Never more than the most points.
        while curve.split_widest().is_some() {}
        assert_eq!(curve.points().len(), MOST_POINTS);
    }
}
