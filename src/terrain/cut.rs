//! The ground of a chunk with the roads cut out of it: where the ribbons and corners lie,
//! there's no terrain, and the terrain's edge there is the ribbon's edge, vertex for
//! vertex, so the two meet without a gap to see under the road or terrain coming up
//! through it.
//!
//! The grid's triangles that a road's outline crosses are cut along the outlines by a
//! constrained Delaunay triangulation of their corners, edges and diagonals and the
//! outlines (crossing each other where roads overlap); the pieces inside a road are
//! dropped. A piece's corners are the grid's (at the terrain's height) or on an outline
//! (at the drawn ribbon's height there). Bridges and waterways aren't cut.

use std::collections::HashSet;

use bevy::math::{DVec2, DVec3};
use bevy::prelude::*;
use spade::{ConstrainedDelaunayTriangulation, HasPosition, Point2, Triangulation};

use super::{FlatMesh, RoadKind, RoadRibbons, Strip, Terrain, ground_color};

/// Points this close to a road's outline are on it (m).
const ON_OUTLINE: f64 = 1e-4;

#[derive(Clone, Copy)]
struct Vertex {
    at: Point2<f64>,
    /// The terrain's height at a grid corner, the ribbon's at an outline's end; NaN where
    /// outlines cross (found on the outlines afterwards).
    height: f64,
}

impl HasPosition for Vertex {
    type Scalar = f64;

    fn position(&self) -> Point2<f64> {
        self.at
    }
}

/// What of the roads lies on a chunk: the triangles they cover it with (in the xz plane)
/// and their outlines ((x, drawn height, z), clipped to the chunk), each listed in the
/// cells it reaches into.
struct Cover {
    /// The chunk's lower corner, its cells' size and how many there are along a side.
    lo: DVec2,
    cell: f64,
    m: usize,
    pieces: Vec<[DVec2; 3]>,
    outlines: Vec<[DVec3; 2]>,
    piece_cells: Vec<Vec<u32>>,
    outline_cells: Vec<Vec<u32>>,
    /// The outlines so far, either way round (the triangulation fails on a constraint
    /// added twice once the first was split).
    seen: HashSet<[[u64; 2]; 2]>,
}

impl Cover {
    fn new(lo: DVec2, cell: f64, m: usize) -> Self {
        Self {
            lo,
            cell,
            m,
            pieces: Vec::new(),
            outlines: Vec::new(),
            piece_cells: vec![Vec::new(); m * m],
            outline_cells: vec![Vec::new(); m * m],
            seen: HashSet::new(),
        }
    }

    fn hi(&self) -> DVec2 {
        self.lo + self.cell * self.m as f64
    }

    /// The cells a box from `lo` to `hi` reaches into (none if it's off the chunk).
    fn cells(&self, lo: DVec2, hi: DVec2) -> impl Iterator<Item = usize> + use<> {
        let m = self.m;
        let (a, b) = ((lo - self.lo) / self.cell, (hi - self.lo) / self.cell);
        let off = b.x < 0.0 || b.y < 0.0 || a.x > m as f64 || a.y > m as f64;
        let index = |v: f64| (v.floor().max(0.0) as usize).min(m - 1);
        let (x0, x1, z0, z1) = (index(a.x), index(b.x), index(a.y), index(b.y));
        let (x1, z1) = if off { (0, 0) } else { (x1 + 1, z1 + 1) };
        (z0..z1).flat_map(move |z| (x0..x1).map(move |x| z * m + x))
    }

    /// The cell `p` is in.
    fn cell_of(&self, p: DVec2) -> usize {
        let v = (p - self.lo) / self.cell;
        let index = |v: f64| (v.floor().max(0.0) as usize).min(self.m - 1);
        index(v.y) * self.m + index(v.x)
    }

    fn piece(&mut self, [a, b, c]: [Vec3; 3]) {
        let tri = [a, b, c].map(|v| DVec2::new(v.x as f64, v.z as f64));
        let lo = tri[0].min(tri[1]).min(tri[2]) - 1e-6;
        let hi = tri[0].max(tri[1]).max(tri[2]) + 1e-6;
        let k = self.pieces.len() as u32;
        let cells: Vec<usize> = self.cells(lo, hi).collect();
        if cells.is_empty() {
            return;
        }
        for c in cells {
            self.piece_cells[c].push(k);
        }
        self.pieces.push(tri);
    }

    /// An outline from `a` to `b`, clipped to the chunk; where it's clipped, its end lies
    /// exactly on the chunk's edge, the same in the chunk next to it.
    fn outline(&mut self, a: Vec3, b: Vec3) {
        let key = |v: Vec3| [v.x.to_bits() as u64, v.z.to_bits() as u64];
        let (ka, kb) = (key(a), key(b));
        if ka == kb || !self.seen.insert([ka.min(kb), ka.max(kb)]) {
            return;
        }
        let (a, b) = (a.as_dvec3(), b.as_dvec3());
        let (lo, hi) = (self.lo, self.hi());
        let d = b - a;
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        // Per side: (start, change, bound, whether it's a lower bound), and which side
        // clipped each end, to put that end exactly on it.
        let mut cut: [Option<(usize, f64)>; 2] = [None, None];
        for (axis, (start, change, low, high)) in [(a.x, d.x, lo.x, hi.x), (a.z, d.z, lo.y, hi.y)]
            .into_iter()
            .enumerate()
        {
            if change.abs() < 1e-12 {
                if start < low || start > high {
                    return;
                }
                continue;
            }
            let (ta, tb) = ((low - start) / change, (high - start) / change);
            let (enter, leave, at_enter, at_leave) = if ta < tb {
                (ta, tb, low, high)
            } else {
                (tb, ta, high, low)
            };
            if enter > t0 {
                t0 = enter;
                cut[0] = Some((axis, at_enter));
            }
            if leave < t1 {
                t1 = leave;
                cut[1] = Some((axis, at_leave));
            }
        }
        if t0 >= t1 {
            return;
        }
        let mut ends = [a + d * t0, a + d * t1];
        for (end, cut) in ends.iter_mut().zip(cut) {
            match cut {
                Some((0, v)) => end.x = v,
                Some((_, v)) => end.z = v,
                None => {}
            }
        }
        if t0 == 0.0 {
            ends[0] = a;
        }
        if t1 == 1.0 {
            ends[1] = b;
        }
        let [p, q] = ends.map(|e| e.xz());
        let reach = DVec2::splat(2.0 * ON_OUTLINE);
        let k = self.outlines.len() as u32;
        for c in self.cells(p.min(q) - reach, p.max(q) + reach) {
            self.outline_cells[c].push(k);
        }
        self.outlines.push(ends);
    }

    /// Whether `p` is inside a road (or a corner).
    fn inside(&self, p: DVec2) -> bool {
        self.piece_cells[self.cell_of(p)].iter().any(|&k| {
            let [a, b, c] = self.pieces[k as usize];
            let det = (b - a).perp_dot(c - a);
            if det.abs() < 1e-12 {
                return false;
            }
            let u = (p - a).perp_dot(c - a) / det;
            let v = (b - a).perp_dot(p - a) / det;
            u >= 0.0 && v >= 0.0 && u + v <= 1.0
        })
    }

    /// The drawn height of the highest outline `p` lies on, if any.
    fn on_outline(&self, p: DVec2) -> Option<f64> {
        let mut best: Option<f64> = None;
        for &k in &self.outline_cells[self.cell_of(p)] {
            let [a, b] = self.outlines[k as usize];
            let (a2, d) = (a.xz(), b.xz() - a.xz());
            let t = ((p - a2).dot(d) / d.length_squared().max(1e-18)).clamp(0.0, 1.0);
            if (a2 + d * t).distance(p) < ON_OUTLINE {
                let h = a.y + (b.y - a.y) * t;
                best = Some(best.map_or(h, |b| b.max(h)));
            }
        }
        best
    }
}

impl RoadRibbons {
    /// The ground of one chunk, flat-shaded, with the roads on the ground cut out.
    pub fn ground_mesh(&self, terrain: &Terrain, index: usize, mesh: &mut FlatMesh) {
        let cover = self.cover(terrain, index);
        if cover.outlines.is_empty() {
            terrain.ground_mesh(index, mesh);
            return;
        }
        // The triangulation can fail on unlucky input (asserting): every chunk of the
        // map in the repo is cut by a test, but a map just baked in the editor isn't, so
        // natively a chunk that fails stays as it is. (On the web, panics abort.)
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut cut = FlatMesh::default();
            let done = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                self.cut(terrain, index, &cover, &mut cut)
            }));
            match done {
                Ok(()) => mesh.append(cut),
                Err(_) => {
                    warn!("couldn't cut the roads out of chunk {index}");
                    terrain.ground_mesh(index, mesh);
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        self.cut(terrain, index, &cover, mesh);
    }

    /// The ground of a chunk with roads on it (`cover`), cut along their outlines.
    fn cut(&self, terrain: &Terrain, index: usize, cover: &Cover, mesh: &mut FlatMesh) {
        let chunk = &terrain.chunks[index];
        let (s, m) = (chunk.step, cover.m);
        let per_side = terrain.chunks_x();
        let (ix0, iz0) = (
            index % per_side * terrain.chunk_cells,
            index / per_side * terrain.chunk_cells,
        );
        let corner = |jx: usize, jz: usize| (ix0 + jx * s, iz0 + jz * s);
        let color = |c: usize, up: f32| {
            let (ix, iz) = corner(c % m, c / m);
            ground_color(chunk.classes[c], up, ix * 31 + iz * 17)
        };
        let touched: Vec<bool> = cover.outline_cells.iter().map(|o| !o.is_empty()).collect();

        // Untouched cells: as they are, unless they're inside a road.
        for c in (0..m * m).filter(|&c| !touched[c]) {
            let (jx, jz) = (c % m, c / m);
            let centre = cover.lo + (DVec2::new(jx as f64, jz as f64) + 0.5) * cover.cell;
            if cover.inside(centre) {
                continue;
            }
            let (ix, iz) = corner(jx, jz);
            let (a, b) = (terrain.vertex(ix, iz), terrain.vertex(ix + s, iz));
            let (cc, d) = (terrain.vertex(ix, iz + s), terrain.vertex(ix + s, iz + s));
            for tri in [[a, d, b], [a, cc, d]] {
                let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize();
                mesh.triangle(tri, color(c, normal.y));
            }
        }

        // The touched cells: their corners, sides and diagonals, cut by the outlines.
        let mut slot = vec![u32::MAX; (m + 1) * (m + 1)];
        let mut vertices = Vec::new();
        let mut edges = Vec::new();
        let mut grid_vertex = |vertices: &mut Vec<Vertex>, lx: usize, lz: usize| {
            let k = lz * (m + 1) + lx;
            if slot[k] == u32::MAX {
                slot[k] = vertices.len() as u32;
                let (ix, iz) = corner(lx, lz);
                let v = terrain.vertex(ix, iz);
                vertices.push(Vertex {
                    at: Point2::new(v.x as f64, v.z as f64),
                    height: v.y as f64,
                });
            }
            slot[k] as usize
        };
        for c in (0..m * m).filter(|&c| touched[c]) {
            let (jx, jz) = (c % m, c / m);
            let a = grid_vertex(&mut vertices, jx, jz);
            let b = grid_vertex(&mut vertices, jx + 1, jz);
            let cc = grid_vertex(&mut vertices, jx, jz + 1);
            let d = grid_vertex(&mut vertices, jx + 1, jz + 1);
            edges.extend([[a, b], [a, cc], [b, d], [cc, d], [a, d]]);
        }
        let mut cdt = ConstrainedDelaunayTriangulation::<Vertex>::bulk_load_cdt(vertices, edges)
            .expect("grid cells triangulate");
        for &[a, b] in &cover.outlines {
            let insert = |cdt: &mut ConstrainedDelaunayTriangulation<Vertex>, v: DVec3| {
                cdt.insert(Vertex {
                    at: Point2::new(v.x, v.z),
                    height: v.y,
                })
            };
            let (Ok(a), Ok(b)) = (insert(&mut cdt, a), insert(&mut cdt, b)) else {
                continue;
            };
            if a != b {
                cdt.add_constraint_and_split(a, b, |at| Vertex {
                    at,
                    height: f64::NAN,
                });
            }
        }

        let mut heights = vec![f32::NAN; cdt.num_vertices()];
        let mut height = |k: usize, data: &Vertex| {
            if heights[k].is_nan() {
                let p = DVec2::new(data.at.x, data.at.y);
                heights[k] = cover
                    .on_outline(p)
                    .or(data.height.is_finite().then_some(data.height))
                    .map_or_else(|| terrain.land(p.x as f32, p.y as f32).0, |h| h as f32);
            }
            heights[k]
        };
        for face in cdt.inner_faces() {
            let vs = face.vertices();
            let ps = vs.map(|v| DVec2::new(v.position().x, v.position().y));
            let centre = (ps[0] + ps[1] + ps[2]) / 3.0;
            let c = cover.cell_of(centre);
            // Points a hair apart are one in f32: what's left between them is nothing.
            let flat = ps.map(|p| p.as_vec2());
            if !touched[c] || flat[0] == flat[1] || flat[1] == flat[2] || flat[2] == flat[0] {
                continue;
            }
            if cover.inside(centre) {
                continue;
            }
            let tri = [0, 1, 2].map(|k| {
                let h = height(vs[k].fix().index(), vs[k].data());
                Vec3::new(flat[k].x, h, flat[k].y)
            });
            let normal = (tri[1] - tri[0]).cross(tri[2] - tri[0]).normalize_or_zero();
            mesh.triangle(tri, color(c, normal.y.abs()));
        }
    }

    /// The roads on the ground and the corners over a chunk, as drawn.
    fn cover(&self, terrain: &Terrain, index: usize) -> Cover {
        let chunk = &terrain.chunks[index];
        let (lo, hi) = terrain.chunk_bounds(index);
        let m = terrain.chunk_cells / chunk.step;
        let mut cover = Cover::new(lo.as_dvec2(), (terrain.cell * chunk.step as f32) as f64, m);
        let per_side = terrain.chunks_x();
        let rows = terrain.chunks.len() / per_side;
        let (cx, cz) = (index % per_side, index / per_side);
        let near = (cz.saturating_sub(1)..=(cz + 1).min(rows - 1)).flat_map(|z| {
            (cx.saturating_sub(1)..=(cx + 1).min(per_side - 1)).map(move |x| z * per_side + x)
        });
        let overlaps = |a: Vec2, b: Vec2| a.x <= hi.x && a.y <= hi.y && b.x >= lo.x && b.y >= lo.y;
        for c in near {
            for &(r, i) in &self.by_chunk[c] {
                let road = &terrain.roads[r];
                if road.bridge || road.kind == RoadKind::Water {
                    continue;
                }
                // Mitred ends are stretched up to 1/0.7.
                let reach = Vec2::splat(road.widest() / 2.0 / 0.7 + 0.5);
                let (p, q) = (road.points[i], road.points[i + 1]);
                if !overlaps(p.min(q) - reach, p.max(q) + reach) {
                    continue;
                }
                let strip = Strip::new(terrain, r, i);
                let (fine, across) = (strip.fine, strip.across);
                let edges: Vec<Vec<Vec3>> = (0..=fine)
                    .map(|k| strip.row(k, [0, across].into_iter()))
                    .collect();
                for pair in edges.windows(2) {
                    let ([l0, r0], [l1, r1]) = ([pair[0][0], pair[0][1]], [pair[1][0], pair[1][1]]);
                    cover.piece([l0, r0, r1]);
                    cover.piece([l0, r1, l1]);
                    cover.outline(l0, l1);
                    cover.outline(r0, r1);
                }
                // The road's ends, across, but where it runs on into another road's
                // end (its ribbon ends on the same mitre).
                let n = road.points.len();
                let first = (i == 0 && road.joins[0].is_none()).then_some(0);
                let last = (i + 2 == n && road.joins[1].is_none()).then_some(fine);
                for k in [first, last].into_iter().flatten() {
                    let row = strip.row(k, 0..=across);
                    for w in row.windows(2) {
                        cover.outline(w[0], w[1]);
                    }
                }
            }
            for &k in &self.corners[c] {
                let corner = &terrain.corners[k];
                let fan: Vec<[Vec3; 3]> = corner.fan().collect();
                let (Some(first), Some(last)) = (fan.first(), fan.last()) else {
                    continue;
                };
                let (a, b) = fan
                    .iter()
                    .flatten()
                    .fold((Vec2::MAX, Vec2::MIN), |(a, b), v| {
                        (a.min(v.xz()), b.max(v.xz()))
                    });
                if !overlaps(a, b) {
                    continue;
                }
                cover.outline(first[0], first[1]);
                cover.outline(last[2], last[0]);
                for tri in &fan {
                    cover.piece(*tri);
                    cover.outline(tri[1], tri[2]);
                }
            }
        }
        cover
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::time::Instant;

    /// A point in the xz plane, by its coordinates' bits.
    type Key = (u32, u32);

    /// Builds the ground of a block of chunks and checks that it has no holes: every
    /// triangle's edge is another triangle's too, but on the block's border and along the
    /// roads' outlines (where the ribbons take over), and no triangle is inside a road.
    fn check_block(terrain: &Terrain, ribbons: &RoadRibbons, centre: Vec2, radius: usize) {
        let per_side = terrain.chunks_x();
        let middle = terrain.chunk_at(centre);
        let (mx, mz) = (middle % per_side, middle / per_side);
        let key = |v: [f32; 3]| (v[0].to_bits(), v[2].to_bits());
        let mut edges: HashMap<(Key, Key), u32> = HashMap::new();
        let mut covers = Vec::new();
        let (mut slowest, mut total) = (0.0f64, 0.0);
        for z in mz - radius..=mz + radius {
            for x in mx - radius..=mx + radius {
                let index = z * per_side + x;
                let mut mesh = FlatMesh::default();
                let start = Instant::now();
                ribbons.ground_mesh(terrain, index, &mut mesh);
                let took = start.elapsed().as_secs_f64();
                (slowest, total) = (slowest.max(took), total + took);
                let cover = ribbons.cover(terrain, index);
                for tri in mesh.positions.chunks(3) {
                    let centre = DVec2::new(
                        (tri[0][0] + tri[1][0] + tri[2][0]) as f64 / 3.0,
                        (tri[0][2] + tri[1][2] + tri[2][2]) as f64 / 3.0,
                    );
                    let area = ((tri[1][0] - tri[0][0]) * (tri[2][2] - tri[0][2])
                        - (tri[2][0] - tri[0][0]) * (tri[1][2] - tri[0][2]))
                        .abs()
                        / 2.0;
                    assert!(
                        area < 1e-3 || !cover.inside(centre),
                        "terrain {tri:?} (area {area}) is inside a road"
                    );
                    for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                        let (a, b) = (key(tri[a]), key(tri[b]));
                        *edges.entry((a.min(b), a.max(b))).or_default() += 1;
                    }
                }
                covers.push(cover);
            }
        }
        let size = terrain.chunk_cells as f32 * terrain.cell;
        let half = terrain.half_size();
        let lo = Vec2::new((mx - radius) as f32, (mz - radius) as f32) * size - half;
        let hi = lo + size * (2 * radius + 1) as f32;
        let on_border = |p: Vec2| p.x == lo.x || p.x == hi.x || p.y == lo.y || p.y == hi.y;
        let point = |(x, z): (u32, u32)| Vec2::new(f32::from_bits(x), f32::from_bits(z));
        // Within a millimetre: the mesh is in f32, a quarter millimetre apart out here.
        let on_outline = |p: Vec2| {
            let p = p.as_dvec2();
            covers.iter().flat_map(|c| &c.outlines).any(|&[a, b]| {
                let (a, d) = (a.xz(), b.xz() - a.xz());
                let t = ((p - a).dot(d) / d.length_squared().max(1e-18)).clamp(0.0, 1.0);
                (a + d * t).distance(p) < 1e-3
            })
        };
        let (mut open, mut cut) = (Vec::new(), 0);
        for (&(a, b), &count) in &edges {
            assert!(
                count <= 2,
                "edge {:?} in {count} triangles",
                (point(a), point(b))
            );
            if count == 2 {
                continue;
            }
            let (a, b) = (point(a), point(b));
            // Along the block's border, and between chunks of different resolutions (the
            // finer one's vertices lie on the coarser one's edges).
            let seam = |v: f32, h: f32| ((v + h) / size).fract() == 0.0;
            let along_seam = (a.x == b.x && seam(a.x, half.x)) || (a.y == b.y && seam(a.y, half.y));
            if (on_border(a) && on_border(b)) || along_seam {
                continue;
            }
            if on_outline(a) && on_outline(b) && on_outline((a + b) / 2.0) {
                cut += 1;
                continue;
            }
            open.push((a, b));
        }
        let chunks = (2 * radius + 1).pow(2);
        eprintln!(
            "{chunks} chunks around {centre}: {} edges, {cut} along roads, slowest {:.1} ms, mean {:.1} ms",
            edges.len(),
            slowest * 1e3,
            total * 1e3 / chunks as f64
        );
        assert!(
            open.is_empty(),
            "{} open edges, e.g. {:?}",
            open.len(),
            &open[..open.len().min(5)]
        );
    }

    #[test]
    fn the_ground_is_closed_up_to_the_road_edges() {
        let terrain = Terrain::baked();
        let ribbons = RoadRibbons::new(&terrain);
        // Gerach (the spawn) and Baunach's middle.
        check_block(&terrain, &ribbons, Vec2::new(-2984.5, -2007.2), 2);
        check_block(&terrain, &ribbons, Vec2::new(-87.0, 2286.0), 2);
    }

    /// Cuts every chunk of the map (about 8 s): that the triangulation never fails (the
    /// web build would abort), and how long the slowest chunk takes.
    #[test]
    fn every_chunk_is_cut() {
        let terrain = Terrain::baked();
        let ribbons = RoadRibbons::new(&terrain);
        let (mut slowest, mut total) = ((0.0f64, 0), 0.0);
        let mut failed = Vec::new();
        for index in 0..terrain.chunk_count() {
            let start = Instant::now();
            let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let cover = ribbons.cover(&terrain, index);
                if !cover.outlines.is_empty() {
                    ribbons.cut(&terrain, index, &cover, &mut FlatMesh::default());
                }
            }));
            if ok.is_err() {
                failed.push(index);
            }
            let took = start.elapsed().as_secs_f64();
            total += took;
            if took > slowest.0 {
                slowest = (took, index);
            }
        }
        eprintln!(
            "{} chunks in {:.1} s, slowest {:.1} ms (chunk {})",
            terrain.chunk_count(),
            total,
            slowest.0 * 1e3,
            slowest.1
        );
        assert!(
            failed.is_empty(),
            "{} chunks failed: {failed:?}",
            failed.len()
        );
    }

    /// Points of a chunk covered by neither the ground nor a road (looked down on), every
    /// `step` metres.
    fn holes(terrain: &Terrain, ribbons: &RoadRibbons, centre: Vec2, step: f32) -> Vec<Vec2> {
        let per_side = terrain.chunks_x();
        let middle = terrain.chunk_at(centre);
        let (mx, mz) = (middle % per_side, middle / per_side);
        let mut tris: Vec<[Vec2; 3]> = Vec::new();
        for z in mz - 1..=mz + 1 {
            for x in mx - 1..=mx + 1 {
                let index = z * per_side + x;
                let (mut ground, mut roads, mut structure) = Default::default();
                ribbons.ground_mesh(terrain, index, &mut ground);
                ribbons.mesh(terrain, index, &mut roads, &mut structure);
                for mesh in [&ground, &roads] {
                    for t in mesh.positions.chunks(3) {
                        tris.push([0, 1, 2].map(|k| Vec2::new(t[k][0], t[k][2])));
                    }
                }
            }
        }
        // Binned by 1 m cells over the middle chunk.
        let (lo, hi) = terrain.chunk_bounds(middle);
        let n = (hi.x - lo.x) as usize;
        let mut bins: Vec<Vec<u32>> = vec![Vec::new(); n * n];
        for (k, t) in tris.iter().enumerate() {
            let a = (t[0].min(t[1]).min(t[2]) - lo).floor();
            let b = (t[0].max(t[1]).max(t[2]) - lo).floor();
            for z in
                (a.y.max(0.0) as usize)..=(b.y.min(n as f32 - 1.0).max(-1.0) as usize).min(n - 1)
            {
                for x in (a.x.max(0.0) as usize)
                    ..=(b.x.min(n as f32 - 1.0).max(-1.0) as usize).min(n - 1)
                {
                    if b.x >= 0.0 && b.y >= 0.0 && a.x < n as f32 && a.y < n as f32 {
                        bins[z * n + x].push(k as u32);
                    }
                }
            }
        }
        let inside = |p: Vec2, [a, b, c]: [Vec2; 3]| {
            let det = (b - a).perp_dot(c - a);
            if det.abs() < 1e-9 {
                return false;
            }
            let u = (p - a).perp_dot(c - a) / det;
            let v = (b - a).perp_dot(p - a) / det;
            // About a millimetre of slack: out here, f32 is a quarter millimetre apart.
            u >= -2e-3 && v >= -2e-3 && u + v <= 1.0 + 2e-3
        };
        let mut holes = Vec::new();
        let count = ((hi.x - lo.x) / step) as usize;
        for j in 0..count {
            for i in 0..count {
                // Off the grid lines, where triangles meet.
                let p = lo + (Vec2::new(i as f32, j as f32) + 0.37) * step;
                let cell =
                    ((p.y - lo.y) as usize).min(n - 1) * n + ((p.x - lo.x) as usize).min(n - 1);
                if !bins[cell].iter().any(|&k| inside(p, tris[k as usize])) {
                    holes.push(p);
                }
            }
        }
        holes
    }

    #[test]
    fn no_holes_between_the_ground_and_the_roads() {
        let terrain = Terrain::baked();
        let ribbons = RoadRibbons::new(&terrain);
        for centre in [
            Vec2::new(-2642.0, -2212.0),
            Vec2::new(-2941.0, -2070.0),
            Vec2::new(-2984.5, -2007.2),
            Vec2::new(-87.0, 2286.0),
        ] {
            let holes = holes(&terrain, &ribbons, centre, 0.1);
            assert!(
                holes.is_empty(),
                "{} holes around {centre}, e.g. {:?}",
                holes.len(),
                &holes[..holes.len().min(8)]
            );
        }
    }
}
