//! Collision boxes, and contacts between them and with the ground, resolved with impulses.
//!
//! A car's body is one box. Houses and tree trunks are boxes standing upright, kept in a
//! uniform grid so a car only tests the few near it. Boxes collide by the separating axis
//! test; the same contact works between two cars, each moving by half.

use bevy::prelude::*;

use super::rigid_body::RigidBody;

/// A box with its centre, its axes (unit, perpendicular) and half its extent along each.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obb {
    pub center: Vec3,
    pub axes: [Vec3; 3],
    pub half: Vec3,
}

/// Where two things touch: the point, the direction that pushes the first out of the
/// second (unit), and how far they overlap along it (m).
#[derive(Clone, Copy, Debug)]
pub struct Contact {
    pub point: Vec3,
    pub normal: Vec3,
    pub depth: f32,
}

/// How surfaces slide and bounce on each other.
#[derive(Clone, Copy, Debug)]
pub struct Surface {
    pub friction: f32,
    pub restitution: f32,
}

/// The body scraping over the ground.
pub const GROUND: Surface = Surface {
    friction: 0.6,
    restitution: 0.1,
};
/// Against walls and trunks, and other cars.
pub const OBSTACLE: Surface = Surface {
    friction: 0.3,
    restitution: 0.15,
};

/// Share of an overlap pushed out per step, and the overlap left alone so resting contacts
/// don't jitter (m).
const CORRECTION: f32 = 0.4;
const SLOP: f32 = 0.005;
/// Approach speed below which nothing bounces (m/s).
const BOUNCE_SPEED: f32 = 1.0;
/// Rounds of impulses over a set of contacts.
const ITERATIONS: usize = 6;

impl Obb {
    /// An upright box: `footprint` centre and length axis in the xz plane, half its length
    /// and width, from `bottom` to `top`.
    pub fn upright(center: Vec2, axis: Vec2, half_length: f32, half_width: f32, bottom: f32, top: f32) -> Self {
        let axis = axis.normalize_or(Vec2::X);
        Self {
            center: Vec3::new(center.x, (bottom + top) / 2.0, center.y),
            axes: [Vec3::new(axis.x, 0.0, axis.y), Vec3::Y, Vec3::new(-axis.y, 0.0, axis.x)],
            half: Vec3::new(half_length, (top - bottom) / 2.0, half_width),
        }
    }

    /// This box (given in a body's frame) where the body is now.
    pub fn placed(&self, position: Vec3, rotation: Quat) -> Self {
        Self {
            center: position + rotation * self.center,
            axes: self.axes.map(|axis| rotation * axis),
            half: self.half,
        }
    }

    /// Radius of the sphere around it.
    pub fn radius(&self) -> f32 {
        self.half.length()
    }

    /// The twelve edges, as pairs of corners.
    pub fn edges(&self) -> impl Iterator<Item = [Vec3; 2]> {
        let corners = self.corners();
        (0..8).flat_map(move |i| {
            [1, 2, 4]
                .into_iter()
                .filter(move |bit| i & bit == 0)
                .map(move |bit| [corners[i], corners[i | bit]])
        })
    }

    /// The eight corners.
    pub fn corners(&self) -> [Vec3; 8] {
        std::array::from_fn(|i| {
            let sign = |bit: usize| if i & bit == 0 { -1.0 } else { 1.0 };
            self.center
                + self.axes[0] * sign(1) * self.half.x
                + self.axes[1] * sign(2) * self.half.y
                + self.axes[2] * sign(4) * self.half.z
        })
    }

    /// Points to test against the ground: the corners, the middles of the edges, and the
    /// middle of each face, so a bump or a crest under the floor counts too.
    pub fn surface_points(&self) -> impl Iterator<Item = Vec3> + '_ {
        (0..27).filter(|&i| i != 13).map(move |i| {
            let s = |k: usize| ((i / k) % 3) as f32 - 1.0;
            self.center
                + self.axes[0] * s(1) * self.half.x
                + self.axes[1] * s(3) * self.half.y
                + self.axes[2] * s(9) * self.half.z
        })
    }

    /// The point of the box closest to `p`.
    pub fn closest(&self, p: Vec3) -> Vec3 {
        let d = p - self.center;
        let mut out = self.center;
        for (axis, half) in self.axes.iter().zip(self.half.to_array()) {
            out += *axis * d.dot(*axis).clamp(-half, half);
        }
        out
    }

    /// Half the box's extent along `axis` (unit).
    fn reach(&self, axis: Vec3) -> f32 {
        (0..3).map(|i| self.half[i] * self.axes[i].dot(axis).abs()).sum()
    }

    /// How this box overlaps `other`, if it does: the contact pushes this one out.
    pub fn contact(&self, other: &Obb) -> Option<Contact> {
        let between = self.center - other.center;
        // The best axis so far: its score, overlap and normal.
        let mut best: Option<(f32, f32, Vec3)> = None;
        let mut test = |axis: Vec3, preference: f32| -> bool {
            let length = axis.length();
            if length < 1e-4 {
                return true;
            }
            let axis = axis / length;
            let overlap = self.reach(axis) + other.reach(axis) - between.dot(axis).abs();
            if overlap < 0.0 {
                return false;
            }
            // Edge axes must be clearly better than a face to win, which keeps the normal
            // from flipping between nearly equal ones.
            if best.is_none_or(|(score, _, _)| overlap * preference < score) {
                let normal = if between.dot(axis) < 0.0 { -axis } else { axis };
                best = Some((overlap * preference, overlap, normal));
            }
            true
        };
        for axis in self.axes.iter().chain(&other.axes) {
            if !test(*axis, 1.0) {
                return None;
            }
        }
        for a in &self.axes {
            for b in &other.axes {
                if !test(a.cross(*b), 1.05) {
                    return None;
                }
            }
        }
        let (_, depth, normal) = best?;
        // A point in the overlap: projecting back and forth between two convex shapes
        // closes in on where they meet.
        let mut point = (self.center + other.center) / 2.0;
        for _ in 0..4 {
            point = self.closest(other.closest(point));
        }
        Some(Contact {
            point,
            normal,
            depth,
        })
    }
}

/// Where the ground pushes on the box: the test points below it.
pub fn ground_contacts(
    shape: &Obb,
    ground: impl Fn(f32, f32) -> (f32, Vec3),
    out: &mut Vec<Contact>,
) {
    let lowest = shape.center.y - shape.reach(Vec3::Y);
    let (height, _) = ground(shape.center.x, shape.center.z);
    // Well clear of the ground under its middle: nothing can touch (slopes aside).
    if lowest > height + shape.radius() {
        return;
    }
    for point in shape.surface_points() {
        let (height, normal) = ground(point.x, point.z);
        let depth = (height - point.y) * normal.y;
        if depth > 0.0 {
            out.push(Contact {
                point,
                normal,
                depth,
            });
        }
    }
}

/// One contact during a round of impulses.
struct Pending {
    contact: Contact,
    /// Normal speed it should part at (m/s), and the impulses so far along the normal and
    /// sideways.
    target: f32,
    normal_impulse: f32,
    friction_impulse: Vec3,
}

/// Resolves `contacts` between `body` and something that doesn't move (the ground, a
/// house), or `other` body, whose contacts point out of `other` into `body`.
pub fn resolve(body: &mut RigidBody, mut other: Option<&mut RigidBody>, contacts: &[Contact], surface: Surface, dt: f32) {
    if contacts.is_empty() {
        return;
    }
    let relative = |a: &RigidBody, b: &Option<&mut RigidBody>, p: Vec3| {
        a.velocity_at(p) - b.as_ref().map_or(Vec3::ZERO, |b| b.velocity_at(p))
    };
    let mut pending: Vec<Pending> = contacts
        .iter()
        .map(|&contact| {
            let approach = relative(body, &other, contact.point).dot(contact.normal);
            let bounce = if approach < -BOUNCE_SPEED {
                -surface.restitution * approach
            } else {
                0.0
            };
            // Pushing out what overlaps, a share per step.
            let push = CORRECTION * (contact.depth - SLOP).max(0.0) / dt;
            Pending {
                contact,
                target: bounce.max(push.min(2.0)),
                normal_impulse: 0.0,
                friction_impulse: Vec3::ZERO,
            }
        })
        .collect();
    for _ in 0..ITERATIONS {
        for p in &mut pending {
            let Contact { point, normal, .. } = p.contact;
            let v = relative(body, &other, point);
            // Along the normal: never pulling, in all.
            let wanted = (p.target - v.dot(normal)) / inverse_mass(body, &other, point, normal);
            let total = (p.normal_impulse + wanted).max(0.0);
            let impulse = total - p.normal_impulse;
            p.normal_impulse = total;
            apply(body, &mut other, normal * impulse, point);
            // Sideways: stopping the sliding, as far as friction reaches.
            let v = relative(body, &other, point);
            let slide = v - normal * v.dot(normal);
            let speed = slide.length();
            if speed > 1e-6 {
                let direction = slide / speed;
                let wanted = -direction * speed / inverse_mass(body, &other, point, direction);
                let limit = surface.friction * p.normal_impulse;
                let total = (p.friction_impulse + wanted).clamp_length_max(limit);
                let impulse = total - p.friction_impulse;
                p.friction_impulse = total;
                apply(body, &mut other, impulse, point);
            }
        }
    }
}

/// How much an impulse along `n` at `point` changes how fast the two bodies part there.
fn inverse_mass(body: &RigidBody, other: &Option<&mut RigidBody>, point: Vec3, n: Vec3) -> f32 {
    body.inverse_mass_at(point, n) + other.as_ref().map_or(0.0, |b| b.inverse_mass_at(point, n))
}

fn apply(body: &mut RigidBody, other: &mut Option<&mut RigidBody>, impulse: Vec3, point: Vec3) {
    body.apply_impulse(impulse, point);
    if let Some(other) = other {
        other.apply_impulse(-impulse, point);
    }
}

/// A box standing upright, stored compactly (there are hundreds of thousands of trees):
/// its middle in the xz plane, half its extent along its length and across, its bottom
/// and top, and the angle of its length from the x axis about y.
#[derive(Clone, Copy, Debug)]
pub struct Upright {
    pub center: Vec2,
    pub half: Vec2,
    pub bottom: f32,
    pub top: f32,
    pub angle: f32,
}

impl Upright {
    pub fn obb(&self) -> Obb {
        let axis = Vec2::from_angle(self.angle);
        Obb::upright(self.center, axis, self.half.x, self.half.y, self.bottom, self.top)
    }

    /// Lower and upper corners of its shadow on the xz plane.
    fn extent(&self) -> (Vec2, Vec2) {
        let (sin, cos) = self.angle.sin_cos();
        let reach = Vec2::new(
            self.half.x * cos.abs() + self.half.y * sin.abs(),
            self.half.x * sin.abs() + self.half.y * cos.abs(),
        );
        (self.center - reach, self.center + reach)
    }

    /// Radius of the sphere around it, and that sphere's centre.
    fn sphere(&self) -> (Vec3, f32) {
        let height = (self.top - self.bottom) / 2.0;
        let center = Vec3::new(self.center.x, self.bottom + height, self.center.y);
        (center, self.half.extend(height).length())
    }
}

/// Things that don't move, in a uniform grid over the xz plane: each cell lists the boxes
/// reaching into it.
#[derive(Default)]
pub struct Obstacles {
    pub boxes: Vec<Upright>,
    origin: Vec2,
    cell: f32,
    columns: usize,
    rows: usize,
    /// Where each cell's list starts in `items` (one more than there are cells).
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl Obstacles {
    /// The grid over `boxes`, in cells of `cell` metres.
    pub fn new(boxes: Vec<Upright>, cell: f32) -> Self {
        let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for b in &boxes {
            let (a, c) = b.extent();
            (lo, hi) = (lo.min(a), hi.max(c));
        }
        if boxes.is_empty() {
            (lo, hi) = (Vec2::ZERO, Vec2::ZERO);
        }
        let columns = ((hi.x - lo.x) / cell).floor() as usize + 1;
        let rows = ((hi.y - lo.y) / cell).floor() as usize + 1;
        let mut grid = Self {
            boxes,
            origin: lo,
            cell,
            columns,
            rows,
            starts: Vec::new(),
            items: Vec::new(),
        };
        // Count, then fill: one flat list, no list per cell.
        let mut counts = vec![0u32; columns * rows + 1];
        for b in &grid.boxes {
            grid.for_cells(b.extent(), |i| counts[i] += 1);
        }
        let mut start = 0;
        for count in &mut counts {
            (start, *count) = (start + *count, start);
        }
        let mut next = counts.clone();
        let mut items = vec![0; start as usize];
        for (k, b) in grid.boxes.iter().enumerate() {
            grid.for_cells(b.extent(), |i| {
                items[next[i] as usize] = k as u32;
                next[i] += 1;
            });
        }
        grid.items = items;
        grid.starts = counts;
        grid
    }

    fn for_cells(&self, (lo, hi): (Vec2, Vec2), mut each: impl FnMut(usize)) {
        let cell = |v: Vec2| {
            let c = ((v - self.origin) / self.cell).floor();
            (
                (c.x.max(0.0) as usize).min(self.columns - 1),
                (c.y.max(0.0) as usize).min(self.rows - 1),
            )
        };
        if self.boxes.is_empty() {
            return;
        }
        let ((x0, z0), (x1, z1)) = (cell(lo), cell(hi));
        for z in z0..=z1 {
            for x in x0..=x1 {
                each(z * self.columns + x);
            }
        }
    }

    /// The obstacles in the grid cells within `reach` of `center` (xz), each once.
    pub fn near(&self, center: Vec2, reach: f32) -> impl Iterator<Item = &Upright> {
        let mut seen: Vec<u32> = Vec::new();
        self.for_cells((center - reach, center + reach), |i| {
            let range = self.starts[i] as usize..self.starts[i + 1] as usize;
            seen.extend_from_slice(&self.items[range]);
        });
        seen.sort_unstable();
        seen.dedup();
        seen.into_iter().map(|k| &self.boxes[k as usize])
    }

    /// Contacts of `shape` with the obstacles near it (pushing `shape` out).
    pub fn contacts(&self, shape: &Obb, out: &mut Vec<Contact>) {
        let reach = shape.radius();
        for b in self.near(shape.center.xz(), reach) {
            let (center, radius) = b.sphere();
            if center.distance_squared(shape.center) > (reach + radius).powi(2) {
                continue;
            }
            out.extend(shape.contact(&b.obb()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(center: Vec3) -> Obb {
        Obb {
            center,
            axes: [Vec3::X, Vec3::Y, Vec3::Z],
            half: Vec3::splat(0.5),
        }
    }

    #[test]
    fn boxes_overlap_along_the_shallowest_axis() {
        let a = cube(Vec3::new(0.9, 0.1, 0.0));
        let contact = a.contact(&cube(Vec3::ZERO)).unwrap();
        assert!((contact.depth - 0.1).abs() < 1e-5, "{contact:?}");
        assert!(contact.normal.abs_diff_eq(Vec3::X, 1e-5), "{contact:?}");
        assert!((contact.point.x - 0.45).abs() < 0.06, "{contact:?}");
        assert!(cube(Vec3::new(1.01, 0.0, 0.0)).contact(&cube(Vec3::ZERO)).is_none());
        // Turned 45° about y, the corner reaches further.
        let turned = Obb {
            axes: [Vec3::X, Vec3::Y, Vec3::Z].map(|a| Quat::from_rotation_y(0.785) * a),
            ..cube(Vec3::new(1.1, 0.0, 0.0))
        };
        assert!(turned.contact(&cube(Vec3::ZERO)).is_some());
    }

    #[test]
    fn a_trunk_against_the_middle_of_a_face_is_found() {
        // A car-sized box, and a thin trunk poking into the middle of its front.
        let car = Obb {
            center: Vec3::new(0.0, 0.6, 0.0),
            axes: [Vec3::X, Vec3::Y, Vec3::Z],
            half: Vec3::new(0.9, 0.5, 2.2),
        };
        let trunk = Obb::upright(Vec2::new(0.0, -2.35), Vec2::X, 0.2, 0.2, 0.0, 8.0);
        let contact = car.contact(&trunk).unwrap();
        assert!(contact.normal.abs_diff_eq(Vec3::Z, 1e-4), "{contact:?}");
        assert!((contact.depth - 0.05).abs() < 1e-4, "{contact:?}");
        assert!(contact.point.x.abs() < 0.21 && (contact.point.z + 2.2).abs() < 0.1, "{contact:?}");
    }

    #[test]
    fn the_grid_finds_what_is_near() {
        let upright = |x: f32, z: f32, length: f32, width: f32, angle: f32| Upright {
            center: Vec2::new(x, z),
            half: Vec2::new(length, width),
            bottom: 0.0,
            top: 6.0,
            angle,
        };
        let mut boxes: Vec<Upright> =
            (0..100).map(|i| upright(i as f32 * 10.0, 0.0, 0.3, 0.3, 0.0)).collect();
        // One long house across many cells, and one turned a quarter.
        boxes.push(upright(500.0, 30.0, 200.0, 5.0, 0.0));
        boxes.push(upright(-50.0, 0.0, 30.0, 2.0, std::f32::consts::FRAC_PI_2));
        let obstacles = Obstacles::new(boxes, 8.0);
        let mut out = Vec::new();
        obstacles.contacts(&cube(Vec3::new(300.6, 1.0, 0.0)), &mut out);
        assert_eq!(out.len(), 1);
        out.clear();
        obstacles.contacts(&cube(Vec3::new(650.0, 1.0, 25.2)), &mut out);
        assert_eq!(out.len(), 1);
        out.clear();
        obstacles.contacts(&cube(Vec3::new(305.0, 1.0, 0.0)), &mut out);
        assert!(out.is_empty());
        obstacles.contacts(&cube(Vec3::new(-50.0, 1.0, 28.0)), &mut out);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn a_box_dropped_on_the_ground_comes_to_rest() {
        let mut body = RigidBody::new(Vec3::new(0.0, 2.0, 0.0), Quat::from_rotation_z(0.3), 1000.0, Vec3::new(400.0, 500.0, 200.0));
        let shape = Obb {
            center: Vec3::ZERO,
            axes: [Vec3::X, Vec3::Y, Vec3::Z],
            half: Vec3::new(0.9, 0.6, 2.2),
        };
        let dt = 1.0 / 120.0;
        let mut contacts = Vec::new();
        for _ in 0..600 {
            body.add_force_at_center(Vec3::new(0.0, -9.81 * body.mass, 0.0));
            body.integrate_velocity(dt);
            contacts.clear();
            ground_contacts(&shape.placed(body.position, body.rotation), |_, _| (0.0, Vec3::Y), &mut contacts);
            resolve(&mut body, None, &contacts, GROUND, dt);
            body.integrate_position(dt);
        }
        assert!(body.velocity.length() < 0.05 && body.angular_velocity.length() < 0.05, "{body:?}");
        // Flat on its bottom, at its half height.
        assert!((body.position.y - 0.6).abs() < 0.03, "{}", body.position);
    }

    #[test]
    fn two_bodies_bounce_apart_keeping_their_momentum() {
        let inertia = Vec3::splat(300.0);
        let mut a = RigidBody::new(Vec3::new(-1.0, 0.0, 0.0), Quat::IDENTITY, 1000.0, inertia);
        let mut b = RigidBody::new(Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY, 1500.0, inertia);
        a.velocity = Vec3::new(10.0, 0.0, 0.0);
        let shape = cube(Vec3::ZERO);
        let shape = Obb { half: Vec3::new(1.0, 0.5, 0.8), ..shape };
        let dt = 1.0 / 120.0;
        for _ in 0..120 {
            if let Some(contact) = shape.placed(a.position, a.rotation).contact(&shape.placed(b.position, b.rotation)) {
                resolve(&mut a, Some(&mut b), &[contact], OBSTACLE, dt);
            }
            a.integrate(dt);
            b.integrate(dt);
        }
        let momentum = a.velocity * a.mass + b.velocity * b.mass;
        assert!(momentum.abs_diff_eq(Vec3::new(10_000.0, 0.0, 0.0), 1.0), "{momentum}");
        assert!(b.velocity.x > a.velocity.x, "{} {}", a.velocity, b.velocity);
        assert!(b.position.x - a.position.x > 1.99);
    }
}
