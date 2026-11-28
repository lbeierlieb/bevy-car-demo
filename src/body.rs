//! Procedural low-poly car bodies. A body is described by a few outlines (the deck line
//! over bonnet, belt and boot, the cabin above it, how the plan narrows at nose and tail)
//! and built by lofting cross-sections along the car: a lower body with wheel wells cut
//! into it, a greenhouse with glass, pillars and a roof, and details (lights, grille,
//! mirrors, wings) as boxes. Everything is one flat-shaded mesh with vertex colours.
//!
//! The body is in pieces, the car's body parts (`Panel`): bumpers, bonnet, fenders, doors,
//! mirrors and the hatch are regions of the outlines (`Body::door`, `bumpers`,
//! `bumper_top` say where they part), the lip, skirts, spoiler and diffuser have shapes of
//! their own. A `Dress` says which are fitted and in what colour; a missing one leaves its
//! opening. The shell is hollow: the lower body has inner faces (painted in the engine bay,
//! trim in the cabin, which is open to the greenhouse), so an opening shows what's inside.
//!
//! Outlines are given in metres from the nose (`x`, towards the tail) and above the
//! ground (`y`). The mesh is in the car's design frame: the origin on the ground midway
//! between the axles, forward along -z.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use crate::items::Panel;

/// Colours of the parts that aren't painted.
pub(crate) const DARK: [f32; 3] = [0.035, 0.035, 0.04];
const GLASS: [f32; 3] = [0.07, 0.09, 0.12];
const LAMP: [f32; 3] = [0.95, 0.93, 0.8];
const TAIL_LAMP: [f32; 3] = [0.75, 0.04, 0.03];
pub(crate) const METAL: [f32; 3] = [0.6, 0.62, 0.65];
/// Inside: the cabin's trim (door cards, the sides), its carpet and headliner.
pub(crate) const TRIM: [f32; 3] = [0.1, 0.1, 0.11];
pub(crate) const CARPET: [f32; 3] = [0.06, 0.06, 0.065];
const HEADLINER: [f32; 3] = [0.42, 0.42, 0.4];
/// Painted panels' insides in the engine bay, darker than their outsides.
const BAY: f32 = 0.7;

/// Clearance between the tire and its wheel arch, and how far the wells' inner walls
/// stand off the tires (the front ones leave room to steer).
const ARCH_CLEARANCE: f32 = 0.05;
const WELL_CLEARANCE: [f32; 2] = [0.17, 0.04];
/// How far the underside rises towards nose and tail, and over what length.
pub(crate) const UNDERCUT: f32 = 0.08;
const UNDERCUT_LENGTH: f32 = 0.35;
/// Lengths over which the plan widens from the nose and narrows to the tail.
const NOSE_TAPER: f32 = 0.9;
const TAIL_TAPER: f32 = 0.8;
/// Width of the deck's flat top and of the cabin's base, relative to the body's.
const DECK_WIDTH: f32 = 0.88;
const CABIN_WIDTH: f32 = 0.85;
const PILLAR: f32 = 0.055;
/// A cabin ending within this of the tail has its rear window in the hatch (m).
const REAR_WINDOW_IN_HATCH: f32 = 0.4;

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
/// the centreline comes as a pair, one each side. It belongs to the piece it's on (a door's
/// rubbing strip goes with the door).
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
    /// glass if it's glazed (1), else a painted panel (0); 2 is a painted side with the
    /// windscreen or rear window glazed (a thick C pillar beside a hatch's glass). Two points are the base and the
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
    /// Where the doors' front and rear edges are (x); their glass runs from the
    /// windscreen's base to the rear edge.
    pub door: [f32; 2],
    /// A four-door's rear doors' edges (x), their glass from the front doors' rear edge
    /// (where a pillar stands) to theirs.
    pub rear_door: Option<[f32; 2]>,
    /// How far the front and the rear bumper reach in from the ends, and the height of
    /// their top edge.
    pub bumpers: [f32; 2],
    pub bumper_top: f32,
    /// Bulges in the bonnet (over a tall engine's intake): from x, to x, half-width and
    /// how high they rise over the deck.
    pub bulges: Vec<[f32; 4]>,
    /// The exhaust leaves at the sides, ahead of the rear wheels (else under the rear
    /// bumper).
    pub side_pipes: bool,
}

/// The wheels the body is built around: front and rear.
#[derive(Clone, Copy, Debug)]
pub struct Wheels {
    pub wheelbase: f32,
    pub track: [f32; 2],
    pub radius: [f32; 2],
    pub width: [f32; 2],
}

/// Which side of the car a piece is on (one in the middle for those there's one of).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Middle,
    Right,
}

impl Side {
    /// The side `across` metres right of the centreline is on (+x is right).
    fn of(across: f32) -> Self {
        if across < 0.0 {
            Side::Left
        } else {
            Side::Right
        }
    }

    /// The side a piece of `panel` on `side` is on: the paired ones keep it.
    fn of_panel(panel: Panel, side: Side) -> Side {
        match panel {
            Panel::Fender | Panel::Door | Panel::RearDoor | Panel::Mirror | Panel::SideSkirt => {
                side
            }
            _ => Side::Middle,
        }
    }
}

/// The body's pieces that are fitted and their colours (sRGB), and the shell's.
#[derive(Clone, Debug, PartialEq)]
pub struct Dress {
    pub shell: [f32; 3],
    pub pieces: Vec<(Panel, Side, [f32; 3])>,
    /// The rear spoiler is a wing on stands (else a lip on the roof's edge).
    pub wing: bool,
}

impl Dress {
    /// Every piece a stock car has (bumpers, bonnet, fenders, doors, mirrors, hatch) in
    /// `color` (rear doors too, which only a four-door body draws).
    pub fn stock(color: [f32; 3]) -> Self {
        let mut pieces = Vec::new();
        for panel in [
            Panel::FrontBumper,
            Panel::Bonnet,
            Panel::Hatch,
            Panel::RearBumper,
        ] {
            pieces.push((panel, Side::Middle, color));
        }
        for panel in [Panel::Fender, Panel::Door, Panel::RearDoor, Panel::Mirror] {
            for side in [Side::Left, Side::Right] {
                pieces.push((panel, side, color));
            }
        }
        Self {
            shell: color,
            pieces,
            wing: false,
        }
    }

    /// The colour of the piece of `panel` on `side`, if it's fitted.
    pub fn get(&self, panel: Panel, side: Side) -> Option<[f32; 3]> {
        let side = Side::of_panel(panel, side);
        (self.pieces.iter())
            .find(|(p, s, _)| *p == panel && *s == side)
            .map(|piece| piece.2)
    }

    /// The colour of `piece` (the shell's for none), if it's there.
    fn color(&self, piece: Option<(Panel, Side)>) -> Option<[f32; 3]> {
        match piece {
            Some((panel, side)) => self.get(panel, side),
            None => Some(self.shell),
        }
    }

    /// Without `panel` (on every side).
    pub fn without(mut self, panel: Panel) -> Self {
        self.pieces.retain(|piece| piece.0 != panel);
        self
    }

    /// With `panel` (on both sides for a paired one) in `color`.
    pub fn with(mut self, panel: Panel, color: [f32; 3]) -> Self {
        for side in [Side::Left, Side::Right] {
            let side = Side::of_panel(panel, side);
            if self.get(panel, side).is_none() {
                self.pieces.push((panel, side, color));
            }
        }
        self
    }

    /// A key for caching meshes of this dress.
    pub fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let bits = |color: &[f32; 3]| color.map(f32::to_bits);
        bits(&self.shell).hash(&mut hasher);
        self.wing.hash(&mut hasher);
        for (panel, side, color) in &self.pieces {
            (*panel as u8, side, bits(color)).hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// A face of a loft: a segment between two of its sections on one side (+1 right, -1
/// left), or a strip across one of its ends.
#[derive(Clone, Copy)]
enum Face {
    Side {
        station: usize,
        segment: usize,
        side: f32,
    },
    Cap {
        front: bool,
        segment: usize,
    },
}

/// A face's colour outside and inside (none: that side isn't drawn).
type Skin = (Option<[f32; 3]>, Option<[f32; 3]>);

/// Collects flat-shaded, coloured triangles.
#[derive(Default)]
pub(crate) struct Builder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
}

impl Builder {
    /// A triangle, front face counter-clockwise; degenerate ones are dropped.
    pub(crate) fn triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, color: [f32; 3]) {
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

    pub(crate) fn quad(&mut self, a: Vec3, b: Vec3, c: Vec3, d: Vec3, color: [f32; 3]) {
        self.triangle(a, b, c, color);
        self.triangle(a, c, d, color);
    }

    /// A quad's outside and inside: the same triangles, turned.
    fn skinned(&mut self, [a, b, c, d]: [Vec3; 4], (outside, inside): Skin) {
        if let Some(color) = outside {
            self.quad(a, b, c, d, color);
        }
        if let Some(color) = inside {
            self.triangle(a, c, b, color);
            self.triangle(a, d, c, color);
        }
    }

    /// A quad turned to face `out`.
    pub(crate) fn quad_facing(&mut self, [a, b, c, d]: [Vec3; 4], out: Vec3, color: [f32; 3]) {
        if (b - a).cross(c - a).dot(out) >= 0.0 {
            self.quad(a, b, c, d, color);
        } else {
            self.quad(d, c, b, a, color);
        }
    }

    /// A box with its centre at `centre`, its axes along `axes` (right-handed), each as
    /// long as the matching `size`.
    pub(crate) fn oriented_box(
        &mut self,
        centre: Vec3,
        axes: [Vec3; 3],
        size: Vec3,
        color: [f32; 3],
    ) {
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

    pub(crate) fn cuboid(&mut self, centre: Vec3, size: Vec3, color: [f32; 3]) {
        self.oriented_box(centre, [Vec3::X, Vec3::Y, Vec3::Z], size, color);
    }

    /// A square bar from `a` to `b`.
    pub(crate) fn bar(&mut self, a: Vec3, b: Vec3, thickness: f32, color: [f32; 3]) {
        let along = (b - a).normalize_or_zero();
        if along == Vec3::ZERO {
            return;
        }
        let side = along.cross(Vec3::X).try_normalize().unwrap_or(Vec3::Y);
        let axes = [along, side, along.cross(side)];
        let size = Vec3::new(a.distance(b), thickness, thickness);
        self.oriented_box((a + b) / 2.0, axes, size, color);
    }

    /// A cylinder from `a` to `b` of `radius`, with `sides` facets and capped ends.
    pub(crate) fn cylinder(
        &mut self,
        a: Vec3,
        b: Vec3,
        radius: f32,
        sides: usize,
        color: [f32; 3],
    ) {
        let along = (b - a).normalize_or_zero();
        if along == Vec3::ZERO {
            return;
        }
        let u = along.any_orthonormal_vector();
        let v = along.cross(u);
        let rim = |k: usize| {
            let angle = std::f32::consts::TAU * k as f32 / sides as f32;
            (u * angle.cos() + v * angle.sin()) * radius
        };
        for k in 0..sides {
            let (p, q) = (rim(k), rim(k + 1));
            self.quad(a + p, a + q, b + q, b + p, color);
            self.triangle(a, a + q, a + p, color);
            self.triangle(b, b + p, b + q, color);
        }
    }

    /// Skins a loft over `sections`: each is the right half of a cross-section, from its
    /// bottom on the centreline around the outside to its top on the centreline, at one z
    /// (sections ordered towards +z). `skin` gives each face's colours: the segments
    /// between one section and the next, and strips across the ends, which close it.
    fn loft(&mut self, sections: &[(f32, Vec<Vec2>)], skin: impl Fn(Face) -> Skin) {
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
                (edge, 1.0)
            } else {
                (count - 1 - edge, -1.0)
            }
        };
        let rings: Vec<Vec<Vec3>> = sections.iter().map(ring).collect();
        for (station, pair) in rings.windows(2).enumerate() {
            let (a, b) = (&pair[0], &pair[1]);
            for edge in 0..count {
                let next = (edge + 1) % count;
                let (segment, side) = segment(edge);
                let face = Face::Side {
                    station,
                    segment,
                    side,
                };
                self.skinned([a[edge], a[next], b[next], b[edge]], skin(face));
            }
        }
        // The ends: a strip across from each point of the half to the next and their
        // mirror images (the halves rise monotonically, so the strips cover the end).
        for (front, (z, half)) in [(true, &sections[0]), (false, &sections[sections.len() - 1])] {
            for segment in 0..points - 1 {
                let at = |p: Vec2, side: f32| Vec3::new(side * p.x, p.y, *z);
                let (p, q) = (half[segment], half[segment + 1]);
                let mut quad = [at(p, 1.0), at(q, 1.0), at(q, -1.0), at(p, -1.0)];
                if front {
                    quad.reverse();
                }
                self.skinned(quad, skin(Face::Cap { front, segment }));
            }
        }
    }

    pub(crate) fn build(self) -> Mesh {
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

    /// How high a bulge in the bonnet rises over the deck at `x`, along its flat top (half
    /// as wide as `half` and less); 0 where there's none.
    fn bulge_rise(from: f32, to: f32, rise: f32, x: f32) -> f32 {
        let t = ((x - from) / (0.5 * (to - from))).clamp(0.0, 1.0);
        rise * (t * (2.0 - t))
    }

    /// The bonnet's top at `x`, `across` from the middle: the deck, or a bulge's flat top
    /// where one is.
    pub fn bonnet_top(&self, x: f32, across: f32) -> f32 {
        let bulge = (self.bulges.iter())
            .filter(|&&[from, to, half, _]| (from..=to).contains(&x) && across.abs() <= 0.7 * half)
            .map(|&[from, to, _, rise]| 0.005 + Self::bulge_rise(from, to, rise, x))
            .fold(0.0, f32::max);
        self.deck_height(x) + bulge
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

    /// The x of a point `z` in the design frame.
    pub fn x(&self, z: f32, wheels: &Wheels) -> f32 {
        z + self.front_overhang + wheels.wheelbase / 2.0
    }

    /// Where a greenhouse (a closed cabin) starts and ends, if there's one: between them
    /// the lower body is open to it.
    pub fn greenhouse(&self) -> Option<(f32, f32)> {
        match &self.cabin[..] {
            [first, _, .., last] => Some((first[0], last[0])),
            _ => None,
        }
    }

    /// Where the lower body is open on top: under a greenhouse, or an open car's cockpit
    /// (its first opening).
    pub fn cockpit(&self) -> Option<(f32, f32)> {
        (self.greenhouse()).or_else(|| self.openings.first().map(|o| (o[0], o[1])))
    }

    /// Half the width of the cockpit's opening at `x`: the greenhouse's base, or an open
    /// car's cockpit.
    pub fn cockpit_half_width(&self, x: f32) -> f32 {
        let width = self.cabin_half_width(x);
        match (self.greenhouse(), self.openings.first()) {
            (None, Some(opening)) => opening[2].min(width),
            _ => width,
        }
    }

    /// Where the wheel arches are: each axle's x and the arch's radius (none with open
    /// wheels).
    fn arches(&self, wheels: &Wheels) -> Option<[(f32, f32); 2]> {
        let axles = [self.front_overhang, self.front_overhang + wheels.wheelbase];
        (!self.open_wheels)
            .then(|| [0, 1].map(|axle| (axles[axle], wheels.radius[axle] + ARCH_CLEARANCE)))
    }

    /// Which doors `x` is between the edges of: the front ones' or a four-door's rear
    /// ones'.
    fn door_at(&self, x: f32) -> Option<Panel> {
        if (self.door[0]..self.door[1]).contains(&x) {
            Some(Panel::Door)
        } else {
            (self.rear_door)
                .filter(|rear| (rear[0]..rear[1]).contains(&x))
                .map(|_| Panel::RearDoor)
        }
    }

    /// Where the side glass that goes with the doors ends: the rear doors' rear edge, or
    /// the front ones'.
    fn door_glass_end(&self) -> f32 {
        self.rear_door.map_or(self.door[1], |rear| rear[1])
    }

    /// Whether the rear window goes with the hatch (a hatchback's, the cabin reaching the
    /// tail) rather than the shell (a saloon's, with a boot lid behind it).
    fn rear_window_in_hatch(&self) -> bool {
        self.cabin
            .last()
            .is_some_and(|last| last[0] > self.length - REAR_WINDOW_IN_HATCH)
    }

    /// The piece a detail at (x, y, across) belongs to (none: the shell).
    fn piece_at(&self, x: f32, y: f32, across: f32) -> Option<(Panel, Side)> {
        let side = Side::of(across);
        let outer = across.abs() > 0.6 * self.half_width(x);
        let cockpit = self.cockpit();
        if y < self.bumper_top && x < self.bumpers[0] {
            Some((Panel::FrontBumper, Side::Middle))
        } else if y < self.bumper_top && x > self.length - self.bumpers[1] {
            Some((Panel::RearBumper, Side::Middle))
        } else if let Some(door) = self.door_at(x).filter(|_| outer) {
            Some((door, side))
        } else if outer && y > self.bumper_top && x > self.bumpers[0] && x < self.door[0] {
            Some((Panel::Fender, side))
        } else if !outer && y > self.bumper_top && cockpit.is_some_and(|(_, end)| x > end) {
            Some((Panel::Hatch, Side::Middle))
        } else if !outer
            && y > self.bumper_top
            && x > self.bumpers[0]
            && cockpit.is_some_and(|(start, _)| x < start)
        {
            Some((Panel::Bonnet, Side::Middle))
        } else {
            None
        }
    }

    /// The body with every stock piece on, in `paint`.
    pub fn mesh_stock(&self, wheels: &Wheels, paint: Color) -> Mesh {
        let paint = paint.to_srgba();
        self.mesh(wheels, &Dress::stock([paint.red, paint.green, paint.blue]))
    }

    /// The body as a mesh in the design frame, with the pieces `dress` has.
    pub fn mesh(&self, wheels: &Wheels, dress: &Dress) -> Mesh {
        let mut builder = Builder::default();
        self.lower_body(&mut builder, wheels, dress);
        match self.cabin.len() {
            0 | 1 => {}
            2 => self.windscreen(&mut builder, wheels, dress.shell),
            _ => self.greenhouse_mesh(&mut builder, wheels, dress),
        }
        let at = |x: f32, y: f32, across: f32| Vec3::new(across, y, self.z(x, wheels));
        // An open car's first opening is its cockpit, open to the lower body.
        let skip = usize::from(self.greenhouse().is_none() && self.cockpit().is_some());
        for opening in &self.openings[skip.min(self.openings.len())..] {
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
                let Some(paint) = dress.color(self.piece_at(x, y, side * across)) else {
                    continue;
                };
                let color = match detail.tone {
                    Tone::Paint => paint,
                    Tone::Dark => DARK,
                    Tone::Glass => GLASS,
                    Tone::Lamp => LAMP,
                    Tone::TailLamp => TAIL_LAMP,
                    Tone::Metal => METAL,
                };
                builder.cuboid(at(x, y, side * across), size, color);
            }
        }
        self.add_ons(&mut builder, wheels, dress);
        self.bulges_mesh(&mut builder, wheels, dress);
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

    /// Lights, grille and number plate, placed by the body's outlines.
    fn fittings(&self) -> Vec<Detail> {
        let nose_top = self.deck_height(0.06);
        let nose_width = self.half_width(0.03);
        let tail_top = self.deck_height(self.length - 0.1);
        let tail_width = self.half_width(self.length - 0.03);
        let nose_bottom = self.floor + UNDERCUT;
        vec![
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
        ]
    }

    /// The body's half cross-section at `x` (in the arch of `in_arch`), from the floor's
    /// middle round the side to the deck's: floor, well wall, sill, lower side (up to the
    /// bumpers' top), upper side, shoulder and deck.
    fn section(&self, x: f32, in_arch: Option<usize>, wheels: &Wheels) -> Vec<Vec2> {
        let w = self.half_width(x);
        let ends = (1.0 - x / UNDERCUT_LENGTH)
            .max(1.0 - (self.length - x) / UNDERCUT_LENGTH)
            .max(0.0);
        let floor = self.floor + UNDERCUT * ends;
        let top = self.deck_height(x);
        // Within an arch, the body starts above the wheel.
        let (inner, bottom) = match (in_arch, self.arches(wheels)) {
            (Some(axle), Some(arches)) => {
                let (at, r) = arches[axle];
                let dx = x - at;
                let over = wheels.radius[axle] + (r * r - dx * dx).max(0.0).sqrt();
                let wall =
                    wheels.track[axle] / 2.0 - wheels.width[axle] / 2.0 - WELL_CLEARANCE[axle];
                (wall.min(w - 0.02), over.max(floor))
            }
            _ => (0.8 * w, floor),
        };
        // The wings rise over the wheels where the deck is lower than they are.
        let shoulder = (top - 0.05).max(bottom + 0.09);
        let crease = self.bumper_top.clamp(bottom + 0.05, shoulder);
        vec![
            Vec2::new(0.0, floor),
            Vec2::new(inner, floor),
            Vec2::new(inner, bottom),
            Vec2::new(w, bottom + 0.05),
            Vec2::new(w, crease),
            Vec2::new(w, shoulder),
            Vec2::new(DECK_WIDTH * w, top.max(shoulder - 0.03)),
            Vec2::new(0.0, top + 0.02),
        ]
    }

    /// The arch (if any) `x` is within.
    fn arch_at(&self, x: f32, wheels: &Wheels) -> Option<usize> {
        let arches = self.arches(wheels)?;
        (0..2).find(|&axle| (x - arches[axle].0).abs() < arches[axle].1)
    }

    /// The colours of the lower body's `segment` (see `section`) at `x` on `side`.
    fn lower_skin(&self, x: f32, segment: usize, side: Side, dress: &Dress) -> Skin {
        let cabin = self.cockpit();
        let in_cabin = cabin.is_some_and(|(start, _)| x > start);
        let piece = match segment {
            0 => return (Some(DARK), Some(CARPET)),
            1 | 2 => return (Some(DARK), Some(DARK)),
            3 if x < self.bumpers[0] => Some((Panel::FrontBumper, Side::Middle)),
            3 if x > self.length - self.bumpers[1] => Some((Panel::RearBumper, Side::Middle)),
            3..=5 if self.door_at(x).is_some() => self.door_at(x).map(|door| (door, side)),
            3..=5 if x < self.door[0] => Some((Panel::Fender, side)),
            3..=5 => None,
            _ => match cabin {
                Some((start, _)) if x < start => Some((Panel::Bonnet, Side::Middle)),
                Some((_, end)) if x > end => Some((Panel::Hatch, Side::Middle)),
                // Open to the cabin.
                Some(_) => return (None, None),
                None => None,
            },
        };
        match dress.color(piece) {
            Some(color) => (
                Some(color),
                Some(if in_cabin && segment < 6 {
                    TRIM
                } else {
                    color.map(|c| c * BAY)
                }),
            ),
            None => (None, None),
        }
    }

    /// The body below the windows: floor, wheel wells, sides, wings and deck, and inside
    /// the bulkhead between the engine bay and the cabin.
    fn lower_body(&self, builder: &mut Builder, wheels: &Wheels, dress: &Dress) {
        let cabin = self.cockpit();
        // Stations: where the deck bends, a few to round the plan off, where the pieces
        // part, and around the wheel arches. Each is (x, the axle whose arch it's in).
        let mut xs: Vec<f32> = self.deck.iter().map(|p| p[0]).collect();
        xs.extend([0.3, 0.6, self.length - 0.3, self.length - 0.55]);
        xs.extend(self.door);
        xs.extend(self.rear_door.into_iter().flatten());
        xs.extend([self.bumpers[0], self.length - self.bumpers[1]]);
        if let Some((start, end)) = cabin {
            xs.extend([start, end]);
        }
        let mut stations: Vec<(f32, Option<usize>)> = xs
            .into_iter()
            .map(|x| (x, self.arch_at(x, wheels)))
            .collect();
        if let Some(arches) = self.arches(wheels) {
            // The arches' ends are doubled: a station on the floor and one at the arch
            // make the wells' end walls.
            for (axle, &(at, arch)) in arches.iter().enumerate() {
                for step in [-1.0, -0.92, -0.7, -0.38, 0.0, 0.38, 0.7, 0.92, 1.0] {
                    stations.push((at + step * arch, Some(axle)));
                }
                stations.push((at - arch, None));
                stations.push((at + arch, None));
            }
        }
        stations.retain(|&(x, _)| (0.0..=self.length).contains(&x));
        // Along the car; at an arch's front end the floor station comes first, at its
        // rear end the arch's.
        let arches = self.arches(wheels);
        stations.sort_by(|a, b| {
            let rank = |s: &(f32, Option<usize>)| match (s.1, arches) {
                (Some(axle), Some(arches)) if s.0 > arches[axle].0 => 0,
                (Some(_), _) => 2,
                (None, _) => 1,
            };
            (a.0, rank(a))
                .partial_cmp(&(b.0, rank(b)))
                .expect("finite stations")
        });
        stations.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-4 && a.1 == b.1);

        let sections: Vec<(f32, Vec<Vec2>)> = (stations.iter())
            .map(|&(x, in_arch)| (self.z(x, wheels), self.section(x, in_arch, wheels)))
            .collect();
        builder.loft(&sections, |face| match face {
            Face::Side {
                station,
                segment,
                side,
            } => {
                let x = (stations[station].0 + stations[station + 1].0) / 2.0;
                self.lower_skin(x, segment, Side::of(side), dress)
            }
            Face::Cap { front, segment } => {
                let piece = match (front, segment) {
                    (_, 0..=2) => return (Some(DARK), Some(DARK)),
                    (true, 3) => Some((Panel::FrontBumper, Side::Middle)),
                    (false, 3) => Some((Panel::RearBumper, Side::Middle)),
                    (true, _) => None,
                    (false, _) => Some((Panel::Hatch, Side::Middle)),
                };
                let color = dress.color(piece);
                let inside = color.map(|c| if front { c.map(|c| c * BAY) } else { TRIM });
                (color, inside)
            }
        });
        let Some((start, end)) = cabin else {
            return;
        };
        // The sills along the cabin: the deck's edge, in to the greenhouse's foot.
        for pair in sections.windows(2) {
            let ([z0, z1], [s0, s1]) = ([pair[0].0, pair[1].0], [&pair[0].1, &pair[1].1]);
            let x = (self.x(z0, wheels) + self.x(z1, wheels)) / 2.0;
            if x < start || x > end {
                continue;
            }
            for side in [1.0f32, -1.0] {
                let inner = |z: f32, edge: Vec2| {
                    let width = self.cockpit_half_width(self.x(z, wheels));
                    Vec3::new(side * width, edge.y + 0.002, z)
                };
                let outer = |z: f32, edge: Vec2| Vec3::new(side * edge.x, edge.y, z);
                let mut quad = [
                    outer(z0, s0[6]),
                    inner(z0, s0[6]),
                    inner(z1, s1[6]),
                    outer(z1, s1[6]),
                ];
                if side < 0.0 {
                    quad.reverse();
                }
                let piece = self.door_at(x).map(|door| (door, Side::of(side)));
                if let Some(color) = dress.color(piece) {
                    builder.skinned(quad, (Some(color), Some(TRIM)));
                }
            }
        }
        // The bulkhead at the windscreen's base, dark towards the bay, trim inside; an open
        // car's behind the seats too, trim towards them.
        let mut walls = vec![(start, (Some(DARK), Some(TRIM)))];
        if self.greenhouse().is_none() {
            walls.push((end, (Some(TRIM), Some(DARK))));
        }
        for (x, skin) in walls {
            let half = self.section(x, self.arch_at(x, wheels), wheels);
            let z = self.z(x, wheels);
            let at = |p: Vec2, side: f32| Vec3::new(side * p.x, p.y, z);
            for pair in half.windows(2) {
                let quad = [
                    at(pair[1], 1.0),
                    at(pair[0], 1.0),
                    at(pair[0], -1.0),
                    at(pair[1], -1.0),
                ];
                builder.skinned(quad, skin);
            }
        }
    }

    /// The bonnet's bulges (with the bonnet): a flat-topped ridge rising from the deck
    /// over the first half of its length, standing full height at its rear end.
    fn bulges_mesh(&self, builder: &mut Builder, wheels: &Wheels, dress: &Dress) {
        let Some(color) = dress.get(Panel::Bonnet, Side::Middle) else {
            return;
        };
        let at = |x: f32, y: f32, across: f32| Vec3::new(across, y, self.z(x, wheels));
        for &[from, to, half, rise] in &self.bulges {
            let mut xs: Vec<f32> = (0..=8)
                .map(|k| from + (to - from) * k as f32 / 8.0)
                .collect();
            xs.extend(
                self.deck
                    .iter()
                    .map(|p| p[0])
                    .filter(|&x| x > from && x < to),
            );
            xs.sort_by(f32::total_cmp);
            xs.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
            let up = |x: f32| Self::bulge_rise(from, to, rise, x);
            // Each station's corners: base right, top right, top left, base left.
            let corners = |x: f32| {
                let base = self.deck_height(x) + 0.005;
                let top = base + up(x);
                [
                    at(x, base, half),
                    at(x, top, 0.7 * half),
                    at(x, top, -0.7 * half),
                    at(x, base, -half),
                ]
            };
            for pair in xs.windows(2) {
                let (a, b) = (corners(pair[0]), corners(pair[1]));
                builder.quad_facing([a[0], a[1], b[1], b[0]], Vec3::X, color);
                builder.quad_facing([a[1], a[2], b[2], b[1]], Vec3::Y, color);
                builder.quad_facing([a[2], a[3], b[3], b[2]], -Vec3::X, color);
            }
            let end = corners(to);
            builder.quad_facing(end, Vec3::Z, color);
        }
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

    /// The cabin's outline with points added at the doors' rear edges, where their glass
    /// ends.
    fn cabin_points(&self) -> Vec<[f32; 3]> {
        let mut cabin = self.cabin.clone();
        for edge in std::iter::once(self.door[1]).chain(self.rear_door.map(|rear| rear[1])) {
            if let Some(k) =
                (1..cabin.len()).find(|&k| cabin[k - 1][0] < edge && edge < cabin[k][0])
            {
                let roof = height(&self.cabin, edge);
                cabin.insert(k, [edge, roof, cabin[k - 1][2]]);
            }
        }
        cabin
    }

    /// The cabin above the deck: glass (or panels), a painted roof, and pillars. It's
    /// open below, to the lower body; the doors' glass goes with the doors, the rear
    /// window with the hatch.
    fn greenhouse_mesh(&self, builder: &mut Builder, wheels: &Wheels, dress: &Dress) {
        let cabin = self.cabin_points();
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
        // The side glazed (1), and the windscreen or rear window (1 or 2).
        let glazed = |station: usize| (0.5..1.5).contains(&cabin[station][2]);
        let window = |station: usize| cabin[station][2] > 0.5;
        // The doors' glass: the front doors' up to their rear edge, then the rear doors'.
        let door_glass = |station: usize| {
            let x = cabin[station][0];
            if x < self.door[1] - 1e-4 {
                Some(Panel::Door)
            } else if x < self.door_glass_end() - 1e-4 {
                Some(Panel::RearDoor)
            } else {
                None
            }
        };
        // A pillar between the front and the rear doors' glass.
        let b_pillar = |station: usize| {
            self.rear_door.is_some() && (cabin[station][0] - self.door[1]).abs() < 1e-4
        };
        let hatch = dress.get(Panel::Hatch, Side::Middle);
        let rear_window = if self.rear_window_in_hatch() {
            hatch
        } else {
            Some(dress.shell)
        };
        builder.loft(&sections, |face| match face {
            // The sides (the doors' glass, then the quarters), then the top: the roof
            // between the windscreen and the rear window.
            Face::Side {
                station,
                segment: 1,
                side,
            } => {
                let piece = door_glass(station).map(|door| (door, Side::of(side)));
                match (dress.color(piece), glazed(station)) {
                    (None, _) => (None, None),
                    (Some(_), true) => (Some(GLASS), None),
                    (Some(color), false) => (Some(color), Some(TRIM)),
                }
            }
            Face::Side {
                station,
                segment: 2,
                ..
            } => match station {
                0 if window(0) => (Some(GLASS), None),
                _ if station + 1 == last && window(station) => (rear_window.map(|_| GLASS), None),
                _ => (Some(dress.shell), Some(HEADLINER)),
            },
            // Open to the lower body.
            Face::Side { .. } => (None, None),
            Face::Cap { front: true, .. } => (Some(dress.shell), None),
            Face::Cap { front: false, .. } => (rear_window, None),
        });
        // Pillars along the windscreen and the rear window, posts in between (but within
        // the doors' glass), and the roof's edges, all standing a little proud of the
        // glass.
        let proud = |p: Vec3, side: f32| Vec3::new(side * (p.x + 0.008), p.y, p.z);
        let paint = dress.shell;
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
                    if door_glass(station).is_none() || b_pillar(station) {
                        builder.bar(proud(top, side), proud(base, side), PILLAR, paint);
                    }
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

    /// The pieces with shapes of their own: mirrors on the doors (with a door), the lip under the front
    /// bumper, skirts along the sills, the spoiler over the hatch's window and the
    /// diffuser under the rear bumper (each only on what it's fitted to).
    fn add_ons(&self, builder: &mut Builder, wheels: &Wheels, dress: &Dress) {
        let at = |x: f32, y: f32, across: f32| Vec3::new(across, y, self.z(x, wheels));
        let size = |along: f32, up: f32, across: f32| Vec3::new(across, up, along);
        if let Some(base) = self.cabin.first() {
            // Mirrors on the doors, at the foot of the side windows.
            let x = base[0] + 0.3;
            for side in [Side::Left, Side::Right] {
                if let (Some(color), Some(_)) =
                    (dress.get(Panel::Mirror, side), dress.get(Panel::Door, side))
                {
                    let across = self.cabin_half_width(x) + 0.09;
                    let across = if side == Side::Left { -across } else { across };
                    let centre = at(x, self.deck_height(x) + 0.07, across);
                    builder.cuboid(centre, size(0.09, 0.08, 0.16), color);
                }
            }
        }
        let has = |panel| dress.get(panel, Side::Middle);
        if let (Some(color), Some(_)) = (has(Panel::FrontLip), has(Panel::FrontBumper)) {
            let length = self.bumpers[0];
            let y = self.floor + 0.55 * UNDERCUT;
            let width = 2.0 * 0.92 * self.half_width(length / 2.0);
            builder.cuboid(
                at(length / 2.0 - 0.02, y, 0.0),
                size(length, 0.03, width),
                color,
            );
        }
        if let Some(arches) = self.arches(wheels) {
            let (from, to) = (arches[0].0 + arches[0].1, arches[1].0 - arches[1].1);
            let x = (from + to) / 2.0;
            for side in [Side::Left, Side::Right] {
                if let Some(color) = dress.get(Panel::SideSkirt, side) {
                    let across = self.half_width(x) + 0.008;
                    let across = if side == Side::Left { -across } else { across };
                    let centre = at(x, self.floor + 0.05, across);
                    builder.cuboid(centre, size(to - from - 0.04, 0.13, 0.05), color);
                }
            }
        }
        if let (Some(color), Some(_), true) =
            (has(Panel::RearSpoiler), has(Panel::Hatch), dress.wing)
        {
            // A wing on two stands from the hatch's top edge, end plates at its tips.
            let x = self.length - 0.22;
            let top = self.deck_height(x);
            let width = 2.0 * 0.86 * self.half_width(x);
            let rise = 0.3;
            for across in [-0.3 * width, 0.3 * width] {
                builder.cuboid(
                    at(x, top + rise / 2.0, across),
                    size(0.12, rise, 0.02),
                    DARK,
                );
            }
            let tilt = Quat::from_rotation_x(-0.2);
            let axes = [tilt * Vec3::X, tilt * Vec3::Y, tilt * Vec3::Z];
            let centre = at(x + 0.02, top + rise + 0.02, 0.0);
            builder.oriented_box(centre, axes, size(0.3, 0.025, width), color);
            for across in [-width / 2.0, width / 2.0] {
                builder.cuboid(
                    at(x + 0.02, top + rise, across),
                    size(0.34, 0.14, 0.012),
                    color,
                );
            }
        } else if let (Some(color), Some(_), Some(cabin)) = (
            has(Panel::RearSpoiler),
            has(Panel::Hatch),
            (self.greenhouse())
                .filter(|_| self.rear_window_in_hatch())
                .map(|_| self.cabin[self.cabin.len() - 2]),
        ) {
            // On the roof's rear edge, over the rear window, tipped down a little.
            let x = cabin[0];
            let width = 2.0 * self.cabin_half_width(x) * self.tumblehome;
            let tilt = Quat::from_rotation_x(0.12);
            let axes = [tilt * Vec3::X, tilt * Vec3::Y, tilt * Vec3::Z];
            let centre = at(x + 0.04, cabin[1] + 0.035, 0.0);
            builder.oriented_box(centre, axes, size(0.26, 0.035, width), color);
        } else if let (Some(color), Some(_)) = (has(Panel::RearSpoiler), has(Panel::Hatch)) {
            // Without a roof (or on a saloon's boot), a lip on the boot lid's rear edge,
            // rising to the back.
            let x = self.length - 0.16;
            let width = 2.0 * 0.8 * self.half_width(x);
            let tilt = Quat::from_rotation_x(-0.25);
            let axes = [tilt * Vec3::X, tilt * Vec3::Y, tilt * Vec3::Z];
            let centre = at(x, self.deck_height(x) + 0.035, 0.0);
            builder.oriented_box(centre, axes, size(0.2, 0.03, width), color);
        }
        if let (Some(color), Some(_)) = (has(Panel::Diffuser), has(Panel::RearBumper)) {
            let length = self.bumpers[1];
            let x = self.length - length / 2.0;
            let y = self.floor + 0.5 * UNDERCUT;
            let width = 2.0 * 0.8 * self.half_width(x);
            builder.cuboid(at(x, y, 0.0), size(length, 0.025, width), color);
            for k in [-1.5f32, -0.5, 0.5, 1.5] {
                let across = k * width / 4.5;
                builder.cuboid(
                    at(x + 0.04, y - 0.04, across),
                    size(length - 0.08, 0.08, 0.015),
                    color,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn positions(mesh: &Mesh) -> Vec<Vec3> {
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions");
        };
        positions.iter().map(|p| Vec3::from_array(*p)).collect()
    }

    #[test]
    fn bodies_are_closed_and_within_their_size() {
        for chassis in &crate::catalogue::builtin().chassis {
            let (body, wheels) = (&chassis.body, chassis.wheels());
            let positions = positions(&body.mesh_stock(&wheels, Color::WHITE));
            let triangles = positions.len() / 3;
            assert!(
                (300..4000).contains(&triangles),
                "{}: {triangles} triangles",
                chassis.name
            );
            // Within the stated size (mirrors and wings may stand out a little), nose
            // at the front, on the ground side of the roof.
            let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
            let mut sum = Vec3::ZERO;
            for &p in &positions {
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
        // The painted outside faces away from the body's middle (all but a little: the
        // mirrors' and spoiler's sides facing in), and the inner faces are another colour.
        for chassis in &crate::catalogue::builtin().chassis {
            let (body, wheels) = (&chassis.body, chassis.wheels());
            let mesh = body.mesh_stock(&wheels, Color::WHITE);
            let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) =
                mesh.attribute(Mesh::ATTRIBUTE_COLOR)
            else {
                panic!("no colours");
            };
            let (mut out, mut all) = (0.0, 0.0);
            for (t, color) in positions(&mesh).chunks(3).zip(colors.chunks(3)) {
                if color[0] != [1.0; 4] {
                    continue;
                }
                let normal = (t[1] - t[0]).cross(t[2] - t[0]);
                let centroid = (t[0] + t[1] + t[2]) / 3.0;
                // Away from the middle of the section it's in, but for the ends.
                let x = body.x(centroid.z, &wheels);
                let top = match body.greenhouse() {
                    Some((start, end)) if (start..end).contains(&x) => height(&body.cabin, x),
                    _ => body.deck_height(x),
                };
                let middle = (body.floor + top) / 2.0;
                let from = Vec3::new(
                    centroid.x / body.half_width(x),
                    (centroid.y - middle) / (top - body.floor),
                    0.0,
                );
                let area = normal.length();
                all += area;
                if normal.dot(from) >= 0.0 {
                    out += area;
                }
            }
            assert!(out > 0.93 * all, "{}: {out} of {all}", chassis.name);
        }
    }

    #[test]
    fn every_piece_has_its_own_faces() {
        // Each piece taken off takes triangles with it, and only its own colour's: on the
        // hatch, and on every four-door (rear doors too).
        let catalogue = crate::catalogue::builtin();
        let closed = (catalogue.chassis.iter().enumerate())
            .filter(|(c, chassis)| *c == 0 || chassis.body.rear_door.is_some());
        for (_, chassis) in closed {
            let (body, wheels) = (&chassis.body, chassis.wheels());
            let red = [0.8, 0.1, 0.1];
            let full = Dress::stock([0.5; 3])
                .with(Panel::FrontLip, red)
                .with(Panel::SideSkirt, red)
                .with(Panel::RearSpoiler, red)
                .with(Panel::Diffuser, red);
            let count = |dress: &Dress| positions(&body.mesh(&wheels, dress)).len() / 3;
            let all = count(&full);
            for panel in Panel::ALL {
                let fewer = count(&full.clone().without(panel));
                if panel == Panel::RearDoor && body.rear_door.is_none() {
                    assert_eq!(fewer, all, "{}", chassis.name);
                } else {
                    assert!(fewer < all, "{} {panel:?}: {fewer} of {all}", chassis.name);
                }
            }
            // A piece in another colour changes only its own faces' colours.
            let mut green = full.clone();
            for piece in &mut green.pieces {
                if piece.0 == Panel::Bonnet {
                    piece.2 = [0.1, 0.8, 0.1];
                }
            }
            assert_eq!(count(&green), all);
            assert_ne!(green.key(), full.key());
        }
    }

    #[test]
    fn the_shell_is_open_where_a_piece_is_missing() {
        // Without the bonnet, a ray straight down onto the bay hits the inside (the
        // floor's carpet side or a well) under the deck instead of the bonnet.
        let catalogue = crate::catalogue::builtin();
        let chassis = &catalogue.chassis[0];
        let (body, wheels) = (&chassis.body, chassis.wheels());
        let highest = |dress: &Dress, x: f32| {
            let z = body.z(x, &wheels);
            let p = positions(&body.mesh(&wheels, dress));
            let mut best = f32::MIN;
            for t in p.chunks(3) {
                let (a, b, c) = (t[0], t[1], t[2]);
                // Barycentric test in the x-z plane at (0, z).
                let d = (b.x - a.x) * (c.z - a.z) - (c.x - a.x) * (b.z - a.z);
                if d.abs() < 1e-9 {
                    continue;
                }
                let u = ((0.0 - a.x) * (c.z - a.z) - (c.x - a.x) * (z - a.z)) / d;
                let v = ((b.x - a.x) * (z - a.z) - (0.0 - a.x) * (b.z - a.z)) / d;
                if u >= 0.0 && v >= 0.0 && u + v <= 1.0 {
                    best = best.max(a.y + u * (b.y - a.y) + v * (c.y - a.y));
                }
            }
            best
        };
        let x = body.front_overhang - 0.2;
        let stock = Dress::stock([0.5; 3]);
        let on = highest(&stock, x);
        let off = highest(&stock.clone().without(Panel::Bonnet), x);
        assert!((on - body.deck_height(x)).abs() < 0.05, "{on}");
        assert!(off < on - 0.2, "{off} under {on}");
    }
}
