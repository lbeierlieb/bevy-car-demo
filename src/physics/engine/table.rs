//! Tables over a grid, interpolated linearly along each axis.

/// Most axes a table can have.
const MAX_AXES: usize = 8;

/// Values on a grid of any number of axes (each ascending). Lookups interpolate linearly
/// along every axis and clamp to the grid's edges; an axis of length one is constant.
#[derive(Clone, Debug)]
pub struct Table {
    axes: Vec<Vec<f32>>,
    /// Row-major: the last axis varies fastest.
    values: Vec<f32>,
}

impl Table {
    /// Fills a table by evaluating `f` at every grid point.
    pub fn bake(axes: Vec<Vec<f32>>, mut f: impl FnMut(&[f32]) -> f32) -> Self {
        assert!(axes.len() <= MAX_AXES && axes.iter().all(|axis| !axis.is_empty()));
        let count = axes.iter().map(Vec::len).product();
        let mut point = vec![0.0; axes.len()];
        let values = (0..count)
            .map(|i| {
                let mut rest = i;
                for (x, axis) in point.iter_mut().zip(&axes).rev() {
                    *x = axis[rest % axis.len()];
                    rest /= axis.len();
                }
                f(&point)
            })
            .collect();
        Self { axes, values }
    }

    pub fn get(&self, point: &[f32]) -> f32 {
        debug_assert_eq!(point.len(), self.axes.len());
        // Per axis: the grid index below the point, the one above, and the weight of the
        // one above.
        let mut cells = [(0, 0, 0.0); MAX_AXES];
        for (cell, (axis, &x)) in cells.iter_mut().zip(self.axes.iter().zip(point)) {
            let i = axis
                .partition_point(|&a| a <= x)
                .clamp(1, axis.len().max(2) - 1)
                - 1;
            *cell = match axis.get(i + 1) {
                Some(&next) => (i, i + 1, ((x - axis[i]) / (next - axis[i])).clamp(0.0, 1.0)),
                None => (0, 0, 0.0),
            };
        }
        let cells = &cells[..self.axes.len()];
        (0..1usize << cells.len())
            .map(|corner| {
                let (mut index, mut weight) = (0, 1.0);
                for (bit, (axis, &(lo, hi, t))) in self.axes.iter().zip(cells).enumerate() {
                    let up = corner >> bit & 1 == 1;
                    index = index * axis.len() + if up { hi } else { lo };
                    weight *= if up { t } else { 1.0 - t };
                }
                if weight == 0.0 {
                    0.0
                } else {
                    weight * self.values[index]
                }
            })
            .sum()
    }
}

/// A curve through `points` (ascending in x), linear in between and flat beyond the ends.
pub fn curve(points: &[(f32, f32)], x: f32) -> f32 {
    let i = points.partition_point(|&(px, _)| px <= x);
    match (i.checked_sub(1).map(|i| points[i]), points.get(i)) {
        (Some((x0, y0)), Some(&(x1, y1))) => y0 + (y1 - y0) * (x - x0) / (x1 - x0),
        (Some((_, y)), None) | (None, Some(&(_, y))) => y,
        (None, None) => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interpolates_and_clamps() {
        let table = Table::bake(vec![vec![0.0, 1.0, 3.0], vec![5.0], vec![0.0, 10.0]], |p| {
            p[0] * 2.0 + p[2]
        });
        // Linear functions come out exactly, between and on grid points.
        for (x, z) in [(0.5, 5.0), (2.0, 2.5), (3.0, 10.0), (0.0, 0.0)] {
            assert!((table.get(&[x, 5.0, z]) - (x * 2.0 + z)).abs() < 1e-5);
        }
        // Outside the grid, and along the constant axis, the edge values hold.
        assert!((table.get(&[-1.0, 99.0, 20.0]) - 10.0).abs() < 1e-5);
        assert!((table.get(&[4.0, -3.0, -5.0]) - 6.0).abs() < 1e-5);
    }

    #[test]
    fn curves_hold_beyond_the_ends() {
        let points = [(1.0, 10.0), (2.0, 20.0), (4.0, 0.0)];
        assert_eq!(curve(&points, 0.0), 10.0);
        assert_eq!(curve(&points, 1.5), 15.0);
        assert_eq!(curve(&points, 3.0), 10.0);
        assert_eq!(curve(&points, 9.0), 0.0);
    }
}
