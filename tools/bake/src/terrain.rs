//! Terrain areas: closed ways tagged `carsim:terrain` (drawn in the map editor) that
//! change the lidar terrain before the roads are fitted to it.
//!
//! - `flatten`: to `carsim:height` (m above sea level).
//! - `raise`: by `carsim:offset` (m; negative lowers).
//! - `smooth`: averages the terrain over `carsim:radius` (m, default 6).
//! - `slope`: a plane through the heights (`ele`) set at its corners (least squares if
//!   more than three; one gives a level plane, two a ramp between them).
//!
//! Each fades in over `carsim:fade` metres (default 5) inside its outline, so the edge
//! meets the terrain around it. Areas are applied in way id order, each to the result
//! of the ones before.

use crate::Point;
use crate::osm::Tags;

const DEFAULT_FADE: f64 = 5.0;
const DEFAULT_RADIUS: f64 = 6.0;

pub enum Operation {
    Flatten(f64),
    Raise(f64),
    Smooth(f64),
    /// Height = a + b x + c z.
    Slope([f64; 3]),
}

pub struct Area {
    pub operation: Operation,
    pub fade: f64,
    /// The outline, not closed.
    pub outline: Vec<Point>,
}

impl Area {
    /// The area a closed way with these tags describes; `corners` are its points with
    /// their `ele` if set. `Err` says what's missing.
    pub fn new(tags: &Tags, corners: &[(Point, Option<f64>)]) -> Result<Self, String> {
        let number = |k: &str| {
            tags.get(k)
                .and_then(|v| v.trim_end_matches('m').trim().parse::<f64>().ok())
        };
        let op = tags.get("carsim:terrain").map(String::as_str).unwrap_or("");
        let operation = match op {
            "flatten" => Operation::Flatten(number("carsim:height").ok_or("no carsim:height")?),
            "raise" => Operation::Raise(number("carsim:offset").ok_or("no carsim:offset")?),
            "smooth" => Operation::Smooth(number("carsim:radius").unwrap_or(DEFAULT_RADIUS)),
            "slope" => {
                let set: Vec<(Point, f64)> =
                    corners.iter().filter_map(|&(p, e)| Some((p, e?))).collect();
                Operation::Slope(plane(&set).ok_or("no corner has an ele")?)
            }
            other => return Err(format!("unknown carsim:terrain={other}")),
        };
        let mut outline: Vec<Point> = corners.iter().map(|c| c.0).collect();
        if outline.len() > 1 && outline.first() == outline.last() {
            outline.pop();
        }
        if outline.len() < 3 {
            return Err("fewer than three corners".into());
        }
        Ok(Self {
            operation,
            fade: number("carsim:fade").unwrap_or(DEFAULT_FADE).max(0.0),
            outline,
        })
    }

    /// How much the operation counts at a point: 0 outside, rising smoothly to 1 at
    /// `fade` inside the outline.
    fn weight(&self, p: Point) -> f64 {
        if !inside(&self.outline, p) {
            return 0.0;
        }
        if self.fade <= 0.0 {
            return 1.0;
        }
        let t = (edge_distance(&self.outline, p) / self.fade).min(1.0);
        t * t * (3.0 - 2.0 * t)
    }

    /// Applies it to a height grid: `n` vertices per row, vertex (i, j) at
    /// `(origin.0 + i cell, origin.1 + j cell)`.
    pub fn apply(&self, heights: &mut [f64], n: usize, origin: Point, cell: f64) {
        let rows = heights.len() / n;
        let (mut lo, mut hi) = (self.outline[0], self.outline[0]);
        for &(x, z) in &self.outline {
            lo = (lo.0.min(x), lo.1.min(z));
            hi = (hi.0.max(x), hi.1.max(z));
        }
        let index = |v: f64, o: f64, count: usize| ((v - o) / cell).clamp(0.0, (count - 1) as f64);
        let (i0, i1) = (
            index(lo.0, origin.0, n).floor() as usize,
            index(hi.0, origin.0, n).ceil() as usize,
        );
        let (j0, j1) = (
            index(lo.1, origin.1, rows).floor() as usize,
            index(hi.1, origin.1, rows).ceil() as usize,
        );
        let smoothed = match self.operation {
            Operation::Smooth(radius) => {
                Some(blur(heights, n, rows, (i0, i1, j0, j1), radius, cell))
            }
            _ => None,
        };
        for j in j0..=j1 {
            for i in i0..=i1 {
                let p = (origin.0 + i as f64 * cell, origin.1 + j as f64 * cell);
                let w = self.weight(p);
                if w <= 0.0 {
                    continue;
                }
                let h = &mut heights[j * n + i];
                let target = match self.operation {
                    Operation::Flatten(level) => level,
                    Operation::Raise(offset) => *h + offset,
                    Operation::Smooth(_) => {
                        let (s, w0) = smoothed.as_ref().expect("smoothed above");
                        s[(j - j0) * w0 + (i - i0)]
                    }
                    Operation::Slope([a, b, c]) => a + b * p.0 + c * p.1,
                };
                *h += (target - *h) * w;
            }
        }
    }
}

/// The heights in the box (i0..=i1, j0..=j1) averaged over `radius` (a box filter run
/// twice, about a Gaussian), and the box's width.
fn blur(
    heights: &[f64],
    n: usize,
    rows: usize,
    (i0, i1, j0, j1): (usize, usize, usize, usize),
    radius: f64,
    cell: f64,
) -> (Vec<f64>, usize) {
    let r = ((radius / cell / 2.0).round() as usize).max(1);
    // Work on the box grown by the filter's reach, so the edges average real terrain.
    let reach = 2 * r;
    let (a0, a1) = (i0.saturating_sub(reach), (i1 + reach).min(n - 1));
    let (b0, b1) = (j0.saturating_sub(reach), (j1 + reach).min(rows - 1));
    let (w, h) = (a1 - a0 + 1, b1 - b0 + 1);
    let mut grid: Vec<f64> = (0..w * h)
        .map(|k| heights[(b0 + k / w) * n + a0 + k % w])
        .collect();
    for _ in 0..2 {
        // Along rows, then along columns.
        let mut next = vec![0.0; w * h];
        for y in 0..h {
            for x in 0..w {
                let (lo, hi) = (x.saturating_sub(r), (x + r).min(w - 1));
                next[y * w + x] =
                    (lo..=hi).map(|k| grid[y * w + k]).sum::<f64>() / (hi - lo + 1) as f64;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let (lo, hi) = (y.saturating_sub(r), (y + r).min(h - 1));
                grid[y * w + x] =
                    (lo..=hi).map(|k| next[k * w + x]).sum::<f64>() / (hi - lo + 1) as f64;
            }
        }
    }
    let width = i1 - i0 + 1;
    let out = (0..width * (j1 - j0 + 1))
        .map(|k| grid[(j0 - b0 + k / width) * w + (i0 - a0 + k % width)])
        .collect();
    (out, width)
}

/// The plane a + b x + c z through heights at points: least squares for three or more
/// (level along whatever direction the points don't span), a ramp for two, level for one.
fn plane(points: &[(Point, f64)]) -> Option<[f64; 3]> {
    let n = points.len() as f64;
    if points.is_empty() {
        return None;
    }
    let (mx, mz, mh) = points
        .iter()
        .fold((0.0, 0.0, 0.0), |(x, z, h), &((px, pz), ph)| {
            (x + px / n, z + pz / n, h + ph / n)
        });
    // Centred sums for the normal equations.
    let (mut sxx, mut sxz, mut szz, mut sxh, mut szh) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for &((x, z), h) in points {
        let (dx, dz, dh) = (x - mx, z - mz, h - mh);
        sxx += dx * dx;
        sxz += dx * dz;
        szz += dz * dz;
        sxh += dx * dh;
        szh += dz * dh;
    }
    let det = sxx * szz - sxz * sxz;
    let (b, c) = if det > 1e-6 * (sxx + szz).powi(2).max(1e-12) {
        ((sxh * szz - szh * sxz) / det, (szh * sxx - sxh * sxz) / det)
    } else if sxx + szz > 1e-9 {
        // On a line (the sums' matrix has rank one): slope along it only.
        let (ux, uz) = if sxx >= szz { (sxx, sxz) } else { (sxz, szz) };
        let len = ux.hypot(uz);
        let (ux, uz) = (ux / len, uz / len);
        let along = (sxh * ux + szh * uz) / (sxx * ux * ux + 2.0 * sxz * ux * uz + szz * uz * uz);
        (along * ux, along * uz)
    } else {
        (0.0, 0.0)
    };
    Some([mh - b * mx - c * mz, b, c])
}

/// Whether a point is inside a polygon (even-odd).
pub fn inside(polygon: &[Point], (x, z): Point) -> bool {
    let mut inside = false;
    let mut j = polygon.len() - 1;
    for i in 0..polygon.len() {
        let ((xi, zi), (xj, zj)) = (polygon[i], polygon[j]);
        if (zi > z) != (zj > z) && x < (xj - xi) * (z - zi) / (zj - zi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Distance from a point to a polygon's outline.
fn edge_distance(polygon: &[Point], p: Point) -> f64 {
    let mut best = f64::MAX;
    for i in 0..polygon.len() {
        let (a, b) = (polygon[i], polygon[(i + 1) % polygon.len()]);
        let (dx, dz) = (b.0 - a.0, b.1 - a.1);
        let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dz) / (dx * dx + dz * dz).max(1e-12))
            .clamp(0.0, 1.0);
        best = best.min((p.0 - a.0 - dx * t).hypot(p.1 - a.1 - dz * t));
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(tags: &[(&str, &str)], ele: [Option<f64>; 4]) -> Area {
        let tags: Tags = tags
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let corners = [(0.0, 0.0), (40.0, 0.0), (40.0, 40.0), (0.0, 40.0)];
        let corners: Vec<(Point, Option<f64>)> = corners.into_iter().zip(ele).collect();
        Area::new(&tags, &corners).unwrap()
    }

    /// A 60 × 60 m grid of 2 m cells from (-10, -10), sloping 1 in 10 along x.
    fn grid() -> Vec<f64> {
        (0..31 * 31)
            .map(|k| ((k % 31) as f64 * 2.0 - 10.0) / 10.0)
            .collect()
    }

    fn at(heights: &[f64], x: f64, z: f64) -> f64 {
        heights[((z + 10.0) / 2.0) as usize * 31 + ((x + 10.0) / 2.0) as usize]
    }

    #[test]
    fn flattening_levels_the_inside_and_leaves_the_outside() {
        let mut h = grid();
        square(
            &[("carsim:terrain", "flatten"), ("carsim:height", "7")],
            [None; 4],
        )
        .apply(&mut h, 31, (-10.0, -10.0), 2.0);
        assert_eq!(at(&h, 20.0, 20.0), 7.0);
        assert_eq!(at(&h, -6.0, 20.0), -0.6);
        // In the fade, between the two.
        let edge = at(&h, 2.0, 20.0);
        assert!(edge > 0.2 && edge < 7.0, "{edge}");
    }

    #[test]
    fn raising_smoothing_and_sloping() {
        let mut h = grid();
        square(
            &[("carsim:terrain", "raise"), ("carsim:offset", "-1.5")],
            [None; 4],
        )
        .apply(&mut h, 31, (-10.0, -10.0), 2.0);
        assert!((at(&h, 20.0, 20.0) - 0.5).abs() < 1e-9);

        // A plane through the corners: 10 m at x = 0, 14 m at x = 40.
        let mut h = grid();
        let ele = [Some(10.0), Some(14.0), Some(14.0), Some(10.0)];
        square(&[("carsim:terrain", "slope"), ("carsim:fade", "0")], ele).apply(
            &mut h,
            31,
            (-10.0, -10.0),
            2.0,
        );
        assert!((at(&h, 20.0, 20.0) - 12.0).abs() < 1e-9);
        assert!((at(&h, 30.0, 10.0) - 13.0).abs() < 1e-9);

        // Two corners: a ramp between them.
        let p = plane(&[((0.0, 0.0), 1.0), ((10.0, 10.0), 3.0)]).unwrap();
        let h = |x: f64, z: f64| p[0] + p[1] * x + p[2] * z;
        assert!((h(5.0, 5.0) - 2.0).abs() < 1e-9 && (h(10.0, 0.0) - 2.0).abs() < 1e-9);

        // Smoothing a spike.
        let mut h = grid();
        h[15 * 31 + 15] += 10.0;
        square(
            &[("carsim:terrain", "smooth"), ("carsim:fade", "0")],
            [None; 4],
        )
        .apply(&mut h, 31, (-10.0, -10.0), 2.0);
        let top = at(&h, 20.0, 20.0);
        assert!(top > 2.0 && top < 2.6, "{top}");
    }
}
