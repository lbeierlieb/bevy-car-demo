//! Road height profiles: heights along every road at once, as close to the lidar as a
//! smooth road can be. Least squares over the whole network: each point is pulled towards
//! its lidar height (weighted by the length it stands for), and the change of slope from
//! one point to the next costs `stiffness`, which sets the shortest bumps the road keeps
//! (see `stiffness`). Roads share the height at the points where they meet, and a road
//! running on as another one (both ending there, or one passing straight through) also
//! bends smoothly across the joint.
//!
//! Solved road by road (each one exactly: its system is banded) with the others' heights
//! held, sweeping until nothing moves; then the lidar heights far off the profile are
//! trusted less (Huber) and it's solved again, so a dip where a bridge's ends are mapped
//! short doesn't pull the road down.
//!
//! Pinned heights (a node's `ele`, set in the map editor) are hard constraints: the road
//! passes through them exactly and bends smoothly towards them.

/// Huber's threshold: lidar heights further than this off the profile count less (m).
const ROBUST: f64 = 0.25;
const ROUNDS: usize = 4;
const SWEEPS: usize = 400;
/// A sweep moving no height more than this ends a round (m).
const SETTLED: f64 = 1e-3;
/// Over-relaxation of each road's solve (block SOR; anything between 1 and 2 converges).
const OVERRELAX: f64 = 1.7;
/// Shortest point spacing used for slopes (m), so near-duplicate points don't blow up.
const MIN_SPACING: f64 = 0.2;
/// The weight of a pinned height, against the lidar's metre-by-metre weights of about 1.
const PIN: f64 = 1e7;

/// The stiffness that lets through bumps longer than `wavelength` (m) and smooths shorter
/// ones away: with the data weighed per metre, the response is 1 / (1 + stiffness k⁴).
pub fn stiffness(wavelength: f64) -> f64 {
    (wavelength / std::f64::consts::TAU).powi(4)
}

#[derive(Default)]
pub struct Problem {
    /// How many heights there are.
    pub count: usize,
    /// (height, weight, lidar height)
    pub data: Vec<(usize, f64, f64)>,
    /// (heights a, b, c in a row, distances a–b and b–c, stiffness)
    pub bends: Vec<([usize; 3], [f64; 2], f64)>,
    /// The heights of each road in order, none twice (closed roads are split in two).
    pub chains: Vec<Vec<usize>>,
    /// Heights that must be exactly so: (height, value).
    pub pins: Vec<(usize, f64)>,
    /// Heights tied to a mix of others: (heights, coefficients, weight, target), costing
    /// weight × (coefficients · heights − target)².
    pub ties: Vec<([usize; 3], [f64; 3], f64, f64)>,
}

impl Problem {
    pub fn solve(&self) -> Vec<f64> {
        let n = self.count;
        // Start from the lidar (the average where several roads see a height).
        let mut x = vec![0.0; n];
        let mut seen = vec![0.0_f64; n];
        for &(i, _, r) in &self.data {
            x[i] += r;
            seen[i] += 1.0;
        }
        for (x, seen) in x.iter_mut().zip(&seen) {
            *x /= seen.max(1.0);
        }
        let mut pinned = vec![None; n];
        for &(i, value) in &self.pins {
            pinned[i] = Some(value);
            x[i] = value;
        }
        // Each bend as coefficients on its heights, a weight and a target (none), and the
        // ties after them.
        let bends: Vec<([usize; 3], [f64; 3], f64, f64)> = self
            .bends
            .iter()
            .map(|&(v, [da, db], stiffness)| {
                let (da, db) = (da.max(MIN_SPACING), db.max(MIN_SPACING));
                let (a, c) = (1.0 / da, 1.0 / db);
                (v, [a, -(a + c), c], stiffness * 2.0 / (da + db), 0.0)
            })
            .chain(self.ties.iter().copied())
            .collect();
        let mut data_of = vec![Vec::new(); n];
        for (t, &(i, _, _)) in self.data.iter().enumerate() {
            data_of[i].push(t);
        }
        let mut bends_of = vec![Vec::new(); n];
        for (t, (v, _, _, _)) in bends.iter().enumerate() {
            for &i in v {
                if !bends_of[i].contains(&t) {
                    bends_of[i].push(t);
                }
            }
        }
        let mut local = vec![usize::MAX; n];
        let chains = self.banded_chains(&bends, &bends_of, &mut local);
        let mut trust = vec![1.0; self.data.len()];
        let mut stamp = vec![usize::MAX; bends.len()];
        let mut stamps = 0;
        let mut sweeps = 0;
        let mut rounds = Vec::new();
        for round in 0..ROUNDS {
            if round > 0 {
                for (t, &(i, _, r)) in self.data.iter().enumerate() {
                    let off = (x[i] - r).abs();
                    trust[t] = if off <= ROBUST { 1.0 } else { ROBUST / off };
                }
            }
            for _ in 0..SWEEPS {
                let mut moved: f64 = 0.0;
                for chain in &chains {
                    let m = chain.len();
                    for (j, &i) in chain.iter().enumerate() {
                        local[i] = j;
                    }
                    // The chain's banded system: diagonal, and one and two off it.
                    let (mut d, mut e1, mut e2, mut rhs) =
                        (vec![1e-9; m], vec![0.0; m], vec![0.0; m], vec![0.0; m]);
                    stamps += 1;
                    for (j, &i) in chain.iter().enumerate() {
                        if let Some(value) = pinned[i] {
                            d[j] += PIN;
                            rhs[j] += PIN * value;
                        }
                        for &t in &data_of[i] {
                            let (_, w, r) = self.data[t];
                            d[j] += w * trust[t];
                            rhs[j] += w * trust[t] * r;
                        }
                        for &t in &bends_of[i] {
                            if stamp[t] == stamps {
                                continue;
                            }
                            stamp[t] = stamps;
                            let (v, c, k, target) = bends[t];
                            for p in 0..3 {
                                let lp = local[v[p]];
                                if lp == usize::MAX {
                                    continue;
                                }
                                rhs[lp] += k * c[p] * target;
                                for q in 0..3 {
                                    let lq = local[v[q]];
                                    let value = k * c[p] * c[q];
                                    if lq != usize::MAX {
                                        match lp as isize - lq as isize {
                                            0 => d[lp] += value,
                                            1 => e1[lp] += value,
                                            2 => e2[lp] += value,
                                            _ => {} // the upper half mirrors the lower
                                        }
                                    } else {
                                        rhs[lp] -= value * x[v[q]];
                                    }
                                }
                            }
                        }
                    }
                    let solved = solve_banded(&d, &e1, &e2, &rhs);
                    for (j, &i) in chain.iter().enumerate() {
                        let step = OVERRELAX * (solved[j] - x[i]);
                        moved = moved.max(step.abs());
                        x[i] += step;
                        local[i] = usize::MAX;
                    }
                }
                sweeps += 1;
                if moved < SETTLED {
                    break;
                }
            }
            rounds.push(sweeps);
            sweeps = 0;
        }
        for (x, pin) in x.iter_mut().zip(&pinned) {
            if let Some(value) = pin {
                *x = *value;
            }
        }
        let mut off: Vec<f64> = self
            .data
            .iter()
            .filter(|d| d.1 > 0.01)
            .map(|&(i, _, r)| (x[i] - r).abs())
            .collect();
        off.sort_by(f64::total_cmp);
        let share = |q: f64| off[((off.len() - 1) as f64 * q) as usize];
        eprintln!(
            "road profiles: {} heights, sweeps {rounds:?}; off the lidar: median {:.2} m, 95 % {:.2} m, 99 % {:.2} m, max {:.1} m",
            x.len(),
            share(0.5),
            share(0.95),
            share(0.99),
            share(1.0)
        );
        x
    }
}

impl Problem {
    /// The chains split so each is solved exactly as a banded system: no height twice in
    /// one, and no bend coupling two of its heights more than two apart (where mapped
    /// roads overlap). `local` must be all `usize::MAX`, and is left so.
    fn banded_chains(
        &self,
        bends: &[([usize; 3], [f64; 3], f64, f64)],
        bends_of: &[Vec<usize>],
        local: &mut [usize],
    ) -> Vec<Vec<usize>> {
        let mut todo: Vec<Vec<usize>> = Vec::new();
        for chain in &self.chains {
            // A new piece wherever a height comes round again.
            let mut piece: Vec<usize> = Vec::new();
            for &i in chain {
                if piece.contains(&i) {
                    let last = *piece.last().unwrap();
                    todo.push(std::mem::replace(&mut piece, vec![last]));
                }
                piece.push(i);
            }
            todo.push(piece);
        }
        let mut done = Vec::new();
        while let Some(chain) = todo.pop() {
            for (j, &i) in chain.iter().enumerate() {
                local[i] = j;
            }
            let far = chain.iter().find_map(|&i| {
                bends_of[i].iter().find_map(|&t| {
                    let at: Vec<usize> = bends[t].0.iter().map(|&v| local[v]).collect();
                    let at: Vec<usize> = at.into_iter().filter(|&l| l != usize::MAX).collect();
                    let (lo, hi) = (*at.iter().min()?, *at.iter().max()?);
                    (hi - lo > 2).then_some((lo + hi) / 2)
                })
            });
            for &i in &chain {
                local[i] = usize::MAX;
            }
            match far {
                Some(cut) => {
                    todo.push(chain[..=cut].to_vec());
                    todo.push(chain[cut..].to_vec());
                }
                None => done.push(chain),
            }
        }
        done
    }
}

/// Solves a symmetric positive definite system with two bands below the diagonal `d`:
/// `e1[i]` is the entry (i, i - 1), `e2[i]` (i, i - 2). LDLᵀ.
fn solve_banded(d: &[f64], e1: &[f64], e2: &[f64], rhs: &[f64]) -> Vec<f64> {
    let m = d.len();
    let (mut a, mut b, mut dd) = (vec![0.0; m], vec![0.0; m], vec![0.0; m]);
    for i in 0..m {
        if i >= 2 {
            b[i] = e2[i] / dd[i - 2];
        }
        if i >= 1 {
            let back = if i >= 2 {
                b[i] * dd[i - 2] * a[i - 1]
            } else {
                0.0
            };
            a[i] = (e1[i] - back) / dd[i - 1];
        }
        dd[i] = d[i]
            - if i >= 1 { a[i] * a[i] * dd[i - 1] } else { 0.0 }
            - if i >= 2 { b[i] * b[i] * dd[i - 2] } else { 0.0 };
    }
    let mut y = vec![0.0; m];
    for i in 0..m {
        y[i] = rhs[i]
            - if i >= 1 { a[i] * y[i - 1] } else { 0.0 }
            - if i >= 2 { b[i] * y[i - 2] } else { 0.0 };
    }
    let mut x = vec![0.0; m];
    for i in (0..m).rev() {
        x[i] = y[i] / dd[i]
            - if i + 1 < m { a[i + 1] * x[i + 1] } else { 0.0 }
            - if i + 2 < m { b[i + 2] * x[i + 2] } else { 0.0 };
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banded_solve_matches_the_matrix() {
        let (d, e1, e2) = (
            [5.0, 6.0, 7.0, 6.5],
            [0.0, 1.0, -2.0, 0.5],
            [0.0, 0.0, 0.7, 1.1],
        );
        let rhs = [1.0, -2.0, 3.0, 0.5];
        let x = solve_banded(&d, &e1, &e2, &rhs);
        let at = |i: usize, j: usize| match i.abs_diff(j) {
            0 => d[i],
            1 => e1[i.max(j)],
            2 => e2[i.max(j)],
            _ => 0.0,
        };
        for (i, want) in rhs.iter().enumerate() {
            let row: f64 = (0..4).map(|j| at(i, j) * x[j]).sum();
            assert!((row - want).abs() < 1e-9, "row {i}: {row}");
        }
    }

    #[test]
    fn a_profile_smooths_bumps_and_bridges_gaps() {
        // A straight road 200 m long, 1 m apart, rising 1 in 20, with a 0.5 m bump 4 m
        // long at 50 m and no data from 120 to 160 m (a bridge).
        let mut problem = Problem {
            count: 201,
            chains: vec![(0..201).collect()],
            ..Default::default()
        };
        for i in 0..201 {
            let s = i as f64;
            let bump = if (48.0..52.0).contains(&s) { 0.5 } else { 0.0 };
            let w = if (120.0..160.0).contains(&s) {
                1e-4
            } else {
                1.0
            };
            problem.data.push((i, w, s / 20.0 + bump));
            if i > 0 && i < 200 {
                problem
                    .bends
                    .push(([i - 1, i, i + 1], [1.0, 1.0], stiffness(40.0)));
            }
        }
        let x = problem.solve();
        assert!((x[50] - 2.5).abs() < 0.1, "bump left {}", x[50] - 2.5);
        assert!((x[140] - 7.0).abs() < 0.05, "gap at {}", x[140]);
        assert!((x[100] - 5.0).abs() < 0.02, "slope at {}", x[100]);
    }

    #[test]
    fn pinned_heights_hold_and_the_road_bends_towards_them() {
        // A flat road 200 m long, pinned 2 m up in the middle.
        let mut problem = Problem {
            count: 201,
            chains: vec![(0..201).collect()],
            pins: vec![(100, 2.0)],
            ..Default::default()
        };
        for i in 0..201 {
            problem.data.push((i, 1.0, 0.0));
            if i > 0 && i < 200 {
                problem
                    .bends
                    .push(([i - 1, i, i + 1], [1.0, 1.0], stiffness(40.0)));
            }
        }
        let x = problem.solve();
        assert_eq!(x[100], 2.0);
        assert!(x[95] > 1.5 && x[95] < 2.0, "near the pin {}", x[95]);
        assert!(x[0].abs() < 0.05, "far from it {}", x[0]);
    }
}
