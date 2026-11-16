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
        self.corners(point)
            .map(|(index, weight)| weight * self.values[index])
            .sum()
    }

    /// The grid points around `point` that a lookup there blends, as indices into
    /// `values`, with their weights (only those with a weight).
    fn corners(&self, point: &[f32]) -> impl Iterator<Item = (usize, f32)> {
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
        let dims = self.axes.len();
        (0..1usize << dims).filter_map(move |corner| {
            let (mut index, mut weight) = (0, 1.0);
            for (bit, (axis, &(lo, hi, t))) in self.axes.iter().zip(&cells[..dims]).enumerate() {
                let up = corner >> bit & 1 == 1;
                index = index * axis.len() + if up { hi } else { lo };
                weight *= if up { t } else { 1.0 - t };
            }
            (weight > 0.0).then_some((index, weight))
        })
    }

    pub fn axis(&self, axis: usize) -> &[f32] {
        &self.axes[axis]
    }

    /// The value at a grid point, by its index along each axis.
    pub fn cell(&self, cell: &[usize]) -> f32 {
        self.values[self.flat(cell)]
    }

    pub fn set_cell(&mut self, cell: &[usize], value: f32) {
        let index = self.flat(cell);
        self.values[index] = value;
    }

    /// Scales the grid points a lookup at `point` blends, each by `factor` in proportion
    /// to its weight there: a correction where the engine runs.
    pub fn scale_at(&mut self, point: &[f32], factor: f32) {
        let corners: Vec<_> = self.corners(point).collect();
        for (index, weight) in corners {
            self.values[index] *= 1.0 + weight * (factor - 1.0);
        }
    }

    fn flat(&self, cell: &[usize]) -> usize {
        debug_assert_eq!(cell.len(), self.axes.len());
        cell.iter()
            .zip(&self.axes)
            .fold(0, |flat, (&i, axis)| flat * axis.len() + i)
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
    fn edits_cells_and_corrects_around_a_point() {
        let mut table = Table::bake(vec![vec![0.0, 1.0], vec![0.0, 1.0, 2.0]], |p| p[0] + p[1]);
        assert_eq!(table.cell(&[1, 2]), 3.0);
        table.set_cell(&[0, 1], 5.0);
        assert_eq!(table.get(&[0.0, 1.0]), 5.0);
        // Halfway between two points, each takes half the correction.
        table.scale_at(&[1.0, 1.5], 1.2);
        assert!((table.cell(&[1, 1]) - 2.0 * 1.1).abs() < 1e-6);
        assert!((table.cell(&[1, 2]) - 3.0 * 1.1).abs() < 1e-6);
        assert_eq!(table.cell(&[0, 1]), 5.0);
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
