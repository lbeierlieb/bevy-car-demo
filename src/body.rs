//! Procedural low-poly car bodies. A body is described by a few outlines (the deck line
//! over bonnet, belt and boot, the cabin above it, how the plan narrows at nose and tail)
//! and built by lofting cross-sections along the car: a lower body with wheel wells cut
//! into it, a greenhouse with glass, pillars and a roof, and details (lights, grille,
//! mirrors, wings) as boxes. Everything is one flat-shaded mesh with vertex colours.
//!
//! Outlines are given in metres from the nose (`x`, towards the tail) and above the
//! ground (`y`). The mesh is in the car's design frame: the origin on the ground midway
//! between the axles, forward along -z.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

/// Colours of the parts that aren't painted.
const DARK: [f32; 3] = [0.035, 0.035, 0.04];
const GLASS: [f32; 3] = [0.07, 0.09, 0.12];
const LAMP: [f32; 3] = [0.95, 0.93, 0.8];
const TAIL_LAMP: [f32; 3] = [0.75, 0.04, 0.03];
const METAL: [f32; 3] = [0.6, 0.62, 0.65];

/// Clearance between the tire and its wheel arch, and how far the wells' inner walls
/// stand off the tires (the front ones leave room to steer).
const ARCH_CLEARANCE: f32 = 0.05;
const WELL_CLEARANCE: [f32; 2] = [0.17, 0.04];
/// How far the underside rises towards nose and tail, and over what length.
const UNDERCUT: f32 = 0.08;
const UNDERCUT_LENGTH: f32 = 0.35;
/// Lengths over which the plan widens from the nose and narrows to the tail.
const NOSE_TAPER: f32 = 0.9;
const TAIL_TAPER: f32 = 0.8;
/// Width of the deck's flat top and of the cabin's base, relative to the body's.
const DECK_WIDTH: f32 = 0.88;
const CABIN_WIDTH: f32 = 0.85;
const PILLAR: f32 = 0.055;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tone {
    Paint,
    Dark,
    Glass,
    Lamp,
    TailLamp,
    Metal,
}

/// A box on the body: its centre (`x` from the nose, `y` above the ground, and how far
/// off the centreline), its size (along the car, up, across), and its colour. A box off
/// the centreline comes as a pair, one each side.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Detail {
    pub at: [f32; 3],
    pub size: [f32; 3],
    pub tone: Tone,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub length: f32,
    pub width: f32,
    /// From the nose to the front axle.
    pub front_overhang: f32,
    /// Height of the floor above the ground.
    pub floor: f32,
    /// The top of the lower body from nose to tail, as (x, y) points.
    pub deck: Vec<[f32; 2]>,
    /// The cabin's outline above the deck from the base of the windscreen to the base of
    /// the rear window, as (x, y, glazed) points: the side from a point to the next is
    /// glass if it's glazed (1), else a painted panel (0). Two points are the base and the
    /// top of an open car's windscreen; none, no windscreen at all.
    pub cabin: Vec<[f32; 3]>,
    /// Width at the nose and at the tail, relative to the full width.
    pub nose: f32,
    pub tail: f32,
    /// Width of the roof relative to the cabin's base.
    pub tumblehome: f32,
    /// The wheels stand outside the body, which has no wheel wells.
    pub open_wheels: bool,
    /// Dark openings in the deck (a cockpit, a pickup's bed): from x, to x, half-width.
    pub openings: Vec<[f32; 3]>,
    pub details: Vec<Detail>,
}

/// The wheels the body is built around: front and rear.
#[derive(Clone, Copy, Debug)]
pub struct Wheels {
    pub wheelbase: f32,
    pub track: [f32; 2],
    pub radius: [f32; 2],
    pub width: [f32; 2],
}

/// Collects flat-shaded, coloured triangles.
#[derive(Default)]
struct Builder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
}

impl Builder {
    /// A triangle, front face counter-clockwise; degenerate ones are dropped.
    fn triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, color: [f32; 3]) {
        let normal = (b - a).cross(c - a);
        if normal.length_squared() < 1e-12 {
            return;
        }
        let normal = normal.normalize().to_array();
        let color = Color::srgb(color[0], color[1], color[2])
            .to_linear()
            .to_f32_array();
        for vertex in [a, b, c] {
            self.positions.push(vertex.to_array());
            self.normals.push(normal);
            self.colors.push(color);
        }
    }

    fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, color: [f32; 3]) {
        self.triangle(a, b, c, color);
        self.triangle(a, c, d, color);
    }

    /// A box with its centre at `centre`, its axes along `axes` (right-handed), each as
    /// long as the matching `size`.
    fn oriented_box(&mut self, centre: Vec3, axes: [Vec3; 3], size: Vec3, color: [f32; 3]) {
        let half = [
            axes[0] * size.x / 2.0,
            axes[1] * size.y / 2.0,
            axes[2] * size.z / 2.0,
        ];
        for axis in 0..3 {
            let (u, v) = (half[(axis + 1) % 3], half[(axis + 2) % 3]);
            for side in [1.0, -1.0] {
                let face = centre + half[axis] * side;
                // Counter-clockwise seen from outside: u × v points along the axis.
                let (u, v) = if side > 0.0 { (u, v) } else { (v, u) };
                self.quad(
                    face - u - v,
                    face + u - v,
                    face + u + v,
                    face - u + v,
                    color,
                );
            }
        }
    }

    fn cuboid(&mut self, centre: Vec3, size: Vec3, color: [f32; 3]) {
        self.oriented_box(centre, [Vec3::X, Vec3::Y, Vec3::Z], size, color);
    }

    /// A square bar from `a` to `b`.
    fn bar(&mut self, a: Vec3, b: Vec3, thickness: f32, color: [f32; 3]) {
        let along = (b - a).normalize_or_zero();
        if along == Vec3::ZERO {
            return;
        }
        let side = along.cross(Vec3::X).try_normalize().unwrap_or(Vec3::Y);
        let axes = [along, side, along.cross(side)];
        let size = Vec3::new(a.distance(b), thickness, thickness);
        self.oriented_box((a + b) / 2.0, axes, size, color);
    }

    /// Skins a loft over `sections`: each is the right half of a cross-section, from its
    /// bottom on the centreline around the outside to its top on the centreline, at one z
    /// (sections ordered towards +z). `colors` gives each half-segment's colour between a
    /// section and the next. Closed at both ends.
    fn loft(&mut self, sections: &[(f32, Vec<Vec2>)], colors: impl Fn(usize, usize) -> [f32; 3]) {
        let points = sections[0].1.len();
        // The whole ring, counter-clockwise seen from behind: up the right half, down
        // the mirrored left one. With it, the segment each ring edge belongs to.
        let ring = |(z, half): &(f32, Vec<Vec2>)| -> Vec<Vec3> {
            let right = half.iter().map(|p| Vec3::new(p.x, p.y, *z));
            let left = half[1..points - 1]
                .iter()
                .rev()
                .map(|p| Vec3::new(-p.x, p.y, *z));
            right.chain(left).collect()
        };
        let count = 2 * points - 2;
        let segment = |edge: usize| {
            if edge < points - 1 {
                edge
            } else {
                count - 1 - edge
            }
        };
        let rings: Vec<Vec<Vec3>> = sections.iter().map(ring).collect();
        for (i, pair) in rings.windows(2).enumerate() {
            let (a, b) = (&pair[0], &pair[1]);
            for edge in 0..count {
                let next = (edge + 1) % count;
                self.quad(a[edge], a[next], b[next], b[edge], colors(i, segment(edge)));
            }
        }
        for (ring, front, station) in [
            (&rings[0], true, 0),
            (&rings[rings.len() - 1], false, rings.len() - 2),
        ] {
            let centre = ring.iter().sum::<Vec3>() / count as f32;
            for edge in 0..count {
                let (p, q) = (ring[edge], ring[(edge + 1) % count]);
                let color = colors(station, segment(edge));
                if front {
                    self.triangle(centre, q, p, color);
                } else {
                    self.triangle(centre, p, q, color);
                }
            }
        }
    }

    fn build(self) -> Mesh {
        let indices = (0..self.positions.len() as u32).collect();
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::RENDER_WORLD,
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, self.colors)
        .with_inserted_indices(Indices::U32(indices))
    }
}

/// Height of a polyline of (x, y, ..) points at `x`, held level beyond its ends.
fn height<const N: usize>(line: &[[f32; N]], x: f32) -> f32 {
    let first = line[0];
    if x <= first[0] {
        return first[1];
    }
    for pair in line.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        if x <= b[0] {
            return a[1] + (b[1] - a[1]) * (x - a[0]) / (b[0] - a[0]).max(1e-6);
        }
    }
    line[line.len() - 1][1]
}

impl Body {
    /// Half the body's width at `x`: narrower at the nose and the tail, rounded off.
    pub fn half_width(&self, x: f32) -> f32 {
        let ease = |t: f32| (t.clamp(0.0, 1.0) * std::f32::consts::FRAC_PI_2).sin();
        let nose = self.nose + (1.0 - self.nose) * ease(x / NOSE_TAPER);
        let tail = self.tail + (1.0 - self.tail) * ease((self.length - x) / TAIL_TAPER);
        self.width / 2.0 * nose.min(tail)
    }

    /// Height of the deck (the lower body's top) at `x`.
    pub fn deck_height(&self, x: f32) -> f32 {
        height(&self.deck, x)
    }

    /// Half the cabin's width at its base at `x`.
    pub fn cabin_half_width(&self, x: f32) -> f32 {
        CABIN_WIDTH * self.half_width(x)
    }

    /// The highest point of the roof (or of the deck, on an open car).
    pub fn height(&self) -> f32 {
        let points = self.cabin.iter().map(|p| p[1]);
        points
            .chain(self.deck.iter().map(|p| p[1]))
            .fold(0.0, f32::max)
    }

    /// z in the design frame of a point `x` behind the nose.
    pub fn z(&self, x: f32, wheels: &Wheels) -> f32 {
        x - self.front_overhang - wheels.wheelbase / 2.0
    }

    /// The body as a mesh in the design frame, painted `paint`.
    pub fn mesh(&self, wheels: &Wheels, paint: Color) -> Mesh {
        let paint = paint.to_srgba();
        let paint = [paint.red, paint.green, paint.blue];
        let tone = |tone: Tone| match tone {
            Tone::Paint => paint,
            Tone::Dark => DARK,
            Tone::Glass => GLASS,
            Tone::Lamp => LAMP,
            Tone::TailLamp => TAIL_LAMP,
            Tone::Metal => METAL,
        };
        let mut builder = Builder::default();
        self.lower_body(&mut builder, wheels, paint);
        match self.cabin.len() {
            0 | 1 => {}
            2 => self.windscreen(&mut builder, wheels, paint),
            _ => self.greenhouse(&mut builder, wheels, paint),
        }
        let at = |x: f32, y: f32, across: f32| Vec3::new(across, y, self.z(x, wheels));
        for opening in &self.openings {
            let [from, to, half_width] = *opening;
            let mut xs: Vec<f32> = self.deck.iter().map(|p| p[0]).collect();
            xs.retain(|&x| x > from && x < to);
            xs.insert(0, from);
            xs.push(to);
            for pair in xs.windows(2) {
                let y = |x: f32| self.deck_height(x) + 0.028;
                let (a, b) = (pair[0], pair[1]);
                builder.quad(
                    at(a, y(a), half_width),
                    at(a, y(a), -half_width),
                    at(b, y(b), -half_width),
                    at(b, y(b), half_width),
                    DARK,
                );
            }
        }
        let fitted = self.fittings();
        for detail in self.details.iter().chain(&fitted) {
            let [x, y, across] = detail.at;
            let size = Vec3::new(detail.size[2], detail.size[1], detail.size[0]);
            let sides: &[f32] = if across == 0.0 { &[1.0] } else { &[1.0, -1.0] };
            for side in sides {
                builder.cuboid(at(x, y, side * across), size, tone(detail.tone));
            }
        }
        builder.build()
    }

    /// The lamps' lenses in the design frame, a little proud of the body's, to draw them
    /// lit over it at night: the head lamps' mesh, the tail lamps' and the head lamps'
    /// centres (for their light).
    pub fn lamps(&self, wheels: &Wheels) -> (Mesh, Mesh, Vec<Vec3>) {
        let at = |x: f32, y: f32, across: f32| Vec3::new(across, y, self.z(x, wheels));
        let (mut heads, mut tails) = (Builder::default(), Builder::default());
        let mut centres = Vec::new();
        for detail in self.details.iter().chain(&self.fittings()) {
            let [x, y, across] = detail.at;
            let builder = match detail.tone {
                Tone::TailLamp => &mut tails,
                // The lamp at the tail is the number plate's.
                Tone::Lamp if x < self.length / 2.0 => &mut heads,
                _ => continue,
            };
            let size = Vec3::new(detail.size[2], detail.size[1], detail.size[0]) * 1.06 + 0.006;
            let sides: &[f32] = if across == 0.0 { &[1.0] } else { &[1.0, -1.0] };
            for side in sides {
                let centre = at(x, y, side * across);
                builder.cuboid(centre, size, [1.0; 3]);
                if detail.tone == Tone::Lamp {
                    centres.push(centre);
                }
            }
        }
        (heads.build(), tails.build(), centres)
    }

    /// Lights, grille, number plate and mirrors, placed by the body's outlines.
    fn fittings(&self) -> Vec<Detail> {
        let nose_top = self.deck_height(0.06);
        let nose_width = self.half_width(0.03);
        let tail_top = self.deck_height(self.length - 0.1);
        let tail_width = self.half_width(self.length - 0.03);
        let nose_bottom = self.floor + UNDERCUT;
        let mut fitted = vec![
            Detail {
                at: [0.03, nose_top - 0.07, 0.68 * nose_width],
                size: [0.1, 0.08, 0.5 * nose_width],
                tone: Tone::Lamp,
            },
            Detail {
                at: [0.0, (nose_bottom + nose_top) / 2.0 - 0.05, 0.0],
                size: [0.05, 0.45 * (nose_top - nose_bottom), 1.1 * nose_width],
                tone: Tone::Dark,
            },
            Detail {
                at: [self.length - 0.03, tail_top - 0.1, 0.7 * tail_width],
                size: [0.1, 0.09, 0.5 * tail_width],
                tone: Tone::TailLamp,
            },
            Detail {
                at: [self.length, tail_top - 0.3, 0.0],
                size: [0.03, 0.1, 0.42],
                tone: Tone::Lamp,
            },
            Detail {
                at: [self.length - 0.02, self.floor + 0.06, 0.5 * tail_width],
                size: [0.12, 0.07, 0.09],
                tone: Tone::Metal,
            },
        ];
        if let Some(base) = self.cabin.first() {
            // Mirrors on the doors, at the foot of the side windows.
            let x = base[0] + 0.3;
            fitted.push(Detail {
                at: [
                    x,
                    self.deck_height(x) + 0.07,
                    self.cabin_half_width(x) + 0.09,
                ],
                size: [0.09, 0.08, 0.16],
                tone: Tone::Paint,
            });
        }
        fitted
    }

    /// The body below the windows: floor, wheel wells, sides, wings and deck.
    fn lower_body(&self, builder: &mut Builder, wheels: &Wheels, paint: [f32; 3]) {
        let axles = [self.front_overhang, self.front_overhang + wheels.wheelbase];
        let arch = |axle: usize| wheels.radius[axle] + ARCH_CLEARANCE;
        // Stations: where the deck bends, a few to round the plan off, and around the
        // wheel arches. Each is (x, the axle whose arch it's in).
        let mut stations: Vec<(f32, Option<usize>)> = self
            .deck
            .iter()
            .map(|p| p[0])
            .chain([0.3, 0.6, self.length - 0.3, self.length - 0.55])
            .map(|x| (x, None))
            .collect();
        if !self.open_wheels {
            // Nothing else within the arches, whose ends are doubled: a station on the
            // floor and one at the arch make the wells' end walls.
            stations.retain(|&(x, _)| (0..2).all(|axle| (x - axles[axle]).abs() > arch(axle)));
            for (axle, &at) in axles.iter().enumerate() {
                for step in [-1.0, -0.92, -0.7, -0.38, 0.0, 0.38, 0.7, 0.92, 1.0] {
                    stations.push((at + step * arch(axle), Some(axle)));
                }
                stations.push((at - arch(axle), None));
                stations.push((at + arch(axle), None));
            }
        }
        stations.retain(|&(x, _)| (0.0..=self.length).contains(&x));
        // Along the car; at an arch's front end the floor station comes first, at its
        // rear end the arch's.
        stations.sort_by(|a, b| {
            let rank = |s: &(f32, Option<usize>)| match s.1 {
                Some(axle) if s.0 > axles[axle] => 0,
                Some(_) => 2,
                None => 1,
            };
            (a.0, rank(a))
                .partial_cmp(&(b.0, rank(b)))
                .expect("finite stations")
        });
        stations.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-4 && a.1 == b.1);

        let sections: Vec<(f32, Vec<Vec2>)> = stations
            .iter()
            .map(|&(x, in_arch)| {
                let w = self.half_width(x);
                let ends = (1.0 - x / UNDERCUT_LENGTH)
                    .max(1.0 - (self.length - x) / UNDERCUT_LENGTH)
                    .max(0.0);
                let floor = self.floor + UNDERCUT * ends;
                let top = self.deck_height(x);
                // Within an arch, the body starts above the wheel.
                let (inner, bottom) = match in_arch {
                    Some(axle) => {
                        let r = arch(axle);
                        let dx = x - axles[axle];
                        let over = wheels.radius[axle] + (r * r - dx * dx).max(0.0).sqrt();
                        let wall = wheels.track[axle] / 2.0
                            - wheels.width[axle] / 2.0
                            - WELL_CLEARANCE[axle];
                        (wall.min(w - 0.02), over.max(floor))
                    }
                    None => (0.8 * w, floor),
                };
                // The wings rise over the wheels where the deck is lower than they are.
                let shoulder = (top - 0.05).max(bottom + 0.09);
                let half = vec![
                    Vec2::new(0.0, floor),
                    Vec2::new(inner, floor),
                    Vec2::new(inner, bottom),
                    Vec2::new(w, bottom + 0.05),
                    Vec2::new(w, shoulder),
                    Vec2::new(DECK_WIDTH * w, top.max(shoulder - 0.03)),
                    Vec2::new(0.0, top + 0.02),
                ];
                (self.z(x, wheels), half)
            })
            .collect();
        builder.loft(
            &sections,
            |_, segment| {
                if segment < 3 { DARK } else { paint }
            },
        );
    }

    /// An open car's windscreen: glass seen from the front only (the driver looks through
    /// it from behind), in a frame.
    fn windscreen(&self, builder: &mut Builder, wheels: &Wheels, paint: [f32; 3]) {
        let corner = |point: [f32; 3], side: f32, top: bool| {
            let width = self.cabin_half_width(point[0]) * if top { self.tumblehome } else { 1.0 };
            Vec3::new(side * width, point[1], self.z(point[0], wheels))
        };
        let (base, top) = (self.cabin[0], self.cabin[1]);
        let [left, right] =
            [-1.0, 1.0].map(|side| (corner(base, side, false), corner(top, side, true)));
        builder.quad(right.0, left.0, left.1, right.1, GLASS);
        builder.bar(left.0, left.1, PILLAR, paint);
        builder.bar(right.0, right.1, PILLAR, paint);
        builder.bar(left.1, right.1, PILLAR, paint);
    }

    /// The cabin above the deck: glass (or panels), a painted roof, and pillars.
    fn greenhouse(&self, builder: &mut Builder, wheels: &Wheels, paint: [f32; 3]) {
        let cabin = &self.cabin;
        let last = cabin.len() - 1;
        let tallest = cabin
            .iter()
            .map(|p| p[1] - self.deck_height(p[0]))
            .fold(0.0, f32::max);
        // Corners of the cabin at each of its points: at the base and at the roof.
        let corners: Vec<(Vec3, Vec3)> = cabin
            .iter()
            .map(|p| {
                let (x, roof) = (p[0], p[1]);
                let base = self.cabin_half_width(x);
                let deck = self.deck_height(x);
                let rise = ((roof - deck) / tallest).clamp(0.0, 1.0);
                let top = base * (1.0 - (1.0 - self.tumblehome) * rise);
                let z = self.z(x, wheels);
                (
                    Vec3::new(base, deck - 0.02, z),
                    Vec3::new(top, roof.max(deck - 0.02), z),
                )
            })
            .collect();
        let sections: Vec<(f32, Vec<Vec2>)> = corners
            .iter()
            .map(|(base, top)| {
                let half = vec![
                    Vec2::new(0.0, base.y),
                    base.xy(),
                    top.xy(),
                    Vec2::new(0.0, top.y + 0.015),
                ];
                (base.z, half)
            })
            .collect();
        let glazed = |station: usize| cabin[station][2] > 0.5;
        builder.loft(&sections, |station, segment| match segment {
            // The sides, then the top: the roof between the windscreen and the rear
            // window.
            1 if glazed(station) => GLASS,
            2 if glazed(station) && (station == 0 || station + 1 == last) => GLASS,
            _ => paint,
        });
        // Pillars along the windscreen and the rear window, posts in between, and the
        // roof's edges, all standing a little proud of the glass.
        let proud = |p: Vec3, side: f32| Vec3::new(side * (p.x + 0.008), p.y, p.z);
        for side in [1.0, -1.0] {
            for station in 0..last {
                let (base, top) = corners[station];
                let (next_base, next_top) = corners[station + 1];
                if !glazed(station) {
                    continue;
                }
                if station == 0 {
                    builder.bar(proud(base, side), proud(next_top, side), PILLAR, paint);
                } else {
                    builder.bar(proud(top, side), proud(base, side), PILLAR, paint);
                    let end = if station + 1 == last {
                        next_base
                    } else {
                        next_top
                    };
                    builder.bar(proud(top, side), proud(end, side), PILLAR, paint);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bodies_are_closed_and_within_their_size() {
        for chassis in &crate::catalogue::builtin().chassis {
            let (body, wheels) = (&chassis.body, chassis.wheels());
            let mesh = body.mesh(&wheels, Color::WHITE);
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!("no positions");
            };
            let triangles = positions.len() / 3;
            assert!(
                (300..2500).contains(&triangles),
                "{}: {triangles} triangles",
                chassis.name
            );
            // Within the stated size (mirrors and wings may stand out a little), nose
            // at the front, on the ground side of the roof.
            let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let mut sum = Vec3::ZERO;
            for position in positions {
                let p = Vec3::from_array(*position);
                assert!(p.is_finite());
                (low, high) = (low.min(p), high.max(p));
                sum += p;
            }
            let nose = body.z(0.0, &wheels);
            assert!((low.z - nose).abs() < 0.08, "{}: nose", chassis.name);
            assert!(
                (high.z - low.z - body.length).abs() < 0.15,
                "{}: {} long",
                chassis.name,
                high.z - low.z
            );
            assert!(
                (high.x - low.x) < body.width + 0.45,
                "{}: {} wide",
                chassis.name,
                high.x - low.x
            );
            assert!(
                low.y > 0.08 && high.y < body.height() + 0.45,
                "{}",
                chassis.name
            );
            // Mirrored about the centreline.
            assert!((sum.x / positions.len() as f32).abs() < 1e-3);
            // The body covers the wheels, unless they're meant to stand outside it.
            let outside = wheels.track[0] / 2.0 + wheels.width[0] / 2.0;
            let covered = body.half_width(body.front_overhang) >= outside - 0.02;
            assert_eq!(covered, !body.open_wheels, "{}", chassis.name);
        }
    }

    #[test]
    fn faces_point_outwards() {
        // The signed volume of a closed, outward-facing mesh is positive: the lower
        // body and the greenhouse are closed, and the details are closed boxes.
        for chassis in &crate::catalogue::builtin().chassis {
            let mesh = chassis.body.mesh(&chassis.wheels(), Color::WHITE);
            let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            else {
                panic!("no positions");
            };
            let volume: f32 = positions
                .chunks(3)
                .map(|t| {
                    let [a, b, c] = [t[0], t[1], t[2]].map(Vec3::from_array);
                    a.dot(b.cross(c)) / 6.0
                })
                .sum();
            let body = &chassis.body;
            let block = body.length * body.width * body.height();
            assert!(
                volume > 0.25 * block && volume < 0.9 * block,
                "{}: {volume} m³ of {block}",
                chassis.name
            );
        }
    }
}
