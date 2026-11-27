//! What the tires throw up: smoke where they slide or spin on the road, and off it dust,
//! mud, stones, bits of turf and water. Local and cosmetic: worked out for every car
//! drawn near the camera from what's seen of it (`Looks`, `Motion` and where it's drawn),
//! so other players' cars and traffic smoke and splash as ours does, with nothing sent.
//!
//! A wheel's slip is the speed its tread moves over the ground: the wheel's own speed
//! there (from where it was drawn the frame before) less what it rolls (from its spin).
//! Smoke comes from sliding alone; off the road rolling throws things up too, more the
//! faster, and sliding much more.
//!
//! The particles are camera-facing quads in one mesh, rebuilt every frame back to front
//! (one draw, blended): soft puffs (smoke, dust, mist) that grow and fade, and small
//! chunks (mud, stones, turf, drops) that fly and fall until they reach the ground they
//! left.

use std::collections::HashMap;
use std::f32::consts::TAU;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::graphics::{Graphics, Particles};
use crate::looks::{Looks, Motion};
use crate::physics::surface::Surface;
use crate::terrain::Terrain;

/// Cars farther from the camera (m) throw nothing up.
const RANGE: f32 = 150.0;
/// The most particles alive at once, for each setting.
const MOST: [usize; 2] = [600, 2000];
/// Emission rates are scaled by this on the low setting.
const LOW_RATE: f32 = 0.4;
/// How far over the ground a tire's bottom may be and still touch it (m).
const TOUCH: f32 = 0.1;
/// A wheel moving faster than this between frames (m/s) was put somewhere: no slip.
const JUMP: f32 = 90.0;
/// How quickly a wheel's slip follows (s): drawn positions of remote cars jitter.
const SLIP_LAG: f32 = 0.08;
/// Gravity on chunks (m/s²).
const GRAVITY: f32 = 9.81;

/// Smoke: from this sliding speed (m/s), full at the second; puffs a second at full.
const SMOKE_SLIDE: (f32, f32) = (3.0, 12.0);
const SMOKE_RATE: f32 = 60.0;
/// Off the road: rolling throws up from the first speed, fully at the second (m/s);
/// sliding from the first, fully at the second.
const ROLL_SPEED: (f32, f32) = (2.0, 22.0);
const DIRT_SLIDE: (f32, f32) = (0.8, 8.0);

pub struct SprayPlugin;

impl Plugin for SprayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Spray>()
            .add_systems(Startup, setup)
            .add_systems(
                PostUpdate,
                (emit, fly, draw)
                    .chain()
                    .in_set(crate::name_tags::PlaceTags)
                    .before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Shape {
    /// Soft, growing and fading.
    Puff,
    /// Small and solid, flying under gravity.
    Chunk,
}

#[derive(Clone, Copy, Debug)]
struct Particle {
    shape: Shape,
    position: Vec3,
    velocity: Vec3,
    /// Linear colour and opacity at the start.
    color: [f32; 4],
    age: f32,
    life: f32,
    /// Size (m) at the start, and what a puff grows to.
    size: (f32, f32),
    /// Turn of the quad in the view (rad) and how fast it turns.
    angle: f32,
    turn: f32,
    /// How quickly the air slows it (1/s), and how it rises (m/s², smoke's buoyancy).
    drag: f32,
    lift: f32,
    /// A chunk dies under this height (the ground it left).
    floor: f32,
}

/// What a wheel was doing the frame before.
#[derive(Clone, Copy, Default)]
struct WheelTrack {
    center: Vec3,
    spin: f32,
    /// Its tread's speed over the ground (m/s, world), smoothed.
    slip: Vec3,
    /// Particles owed but not yet emitted (fractions carry over).
    owed: [f32; 2],
}

#[derive(Clone, Copy, Default)]
struct Track {
    wheels: [WheelTrack; 4],
    seen: bool,
}

#[derive(Resource, Default)]
struct Spray {
    particles: Vec<Particle>,
    tracks: HashMap<Entity, Track>,
    seed: u32,
}

impl Spray {
    /// A number in 0..1 (xorshift).
    fn random(&mut self) -> f32 {
        let mut x = self.seed.max(1);
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.seed = x;
        (x >> 8) as f32 / (1 << 24) as f32
    }

    /// A number in -1..1.
    fn signed(&mut self) -> f32 {
        self.random() * 2.0 - 1.0
    }

    fn around(&mut self, spread: f32) -> Vec3 {
        Vec3::new(self.signed(), self.signed(), self.signed()) * spread
    }
}

/// The particle mesh's entity.
#[derive(Component)]
struct Cloud;

/// The texture: a soft puff on the left half, a round blob on the right.
const TEXTURE: u32 = 64;

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let material = StandardMaterial {
        base_color_texture: Some(images.add(texture())),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        double_sided: true,
        ..default()
    };
    commands.spawn((
        Cloud,
        Mesh3d(meshes.add(empty_mesh())),
        MeshMaterial3d(materials.add(material)),
        Transform::default(),
        Visibility::Hidden,
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

/// One quad nobody sees: a mesh without vertices is warned about every frame.
fn empty_mesh() -> Mesh {
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 4])
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 0.0, 1.0]; 4])
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0f32; 2]; 4])
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, vec![[0.0f32; 4]; 4])
    .with_inserted_indices(Indices::U32(vec![0, 1, 2, 0, 2, 3]))
}

/// Value noise on a small lattice, wrapping, for the puff's clumps.
fn noise(x: f32, y: f32, cells: u32) -> f32 {
    let hash = |i: u32, j: u32| {
        let mut h = (i % cells).wrapping_mul(374_761_393) ^ (j % cells).wrapping_mul(668_265_263);
        h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
        (h ^ (h >> 16)) as f32 / u32::MAX as f32
    };
    let (fx, fy) = (x * cells as f32, y * cells as f32);
    let (i, j) = (fx.floor() as u32, fy.floor() as u32);
    let (tx, ty) = (fx.fract(), fy.fract());
    let (sx, sy) = (tx * tx * (3.0 - 2.0 * tx), ty * ty * (3.0 - 2.0 * ty));
    let top = hash(i, j) + (hash(i + 1, j) - hash(i, j)) * sx;
    let bottom = hash(i, j + 1) + (hash(i + 1, j + 1) - hash(i, j + 1)) * sx;
    top + (bottom - top) * sy
}

fn texture() -> Image {
    let size = TEXTURE;
    let mut data = Vec::with_capacity((size * 2 * size * 4) as usize);
    for y in 0..size {
        for x in 0..size * 2 {
            let blob = x >= size;
            let u = ((x % size) as f32 + 0.5) / size as f32;
            let v = (y as f32 + 0.5) / size as f32;
            let r = ((u - 0.5).hypot(v - 0.5) * 2.0).min(1.0);
            let alpha = if blob {
                // Solid with a slightly soft rim.
                ((1.0 - r) / 0.25).clamp(0.0, 1.0)
            } else {
                let clumps = 0.55 * noise(u, v, 4) + 0.3 * noise(u, v, 8) + 0.15 * noise(u, v, 16);
                let falloff = (1.0 - r).max(0.0).powf(1.6);
                (falloff * (0.55 + 0.9 * clumps)).clamp(0.0, 1.0)
            };
            data.extend_from_slice(&[255, 255, 255, (alpha * 255.0).round() as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: size * 2,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn smoothstep((from, to): (f32, f32), x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Linear colour from sRGB bytes.
fn rgb(r: u8, g: u8, b: u8, alpha: f32) -> [f32; 4] {
    let c = Color::srgb_u8(r, g, b).to_linear();
    [c.red, c.green, c.blue, alpha]
}

/// How much a wheel throws up of each of the two kinds a ground has, a second: the
/// first is puffs, the second chunks.
fn rates(surface: Surface, speed: f32, sliding: f32) -> [f32; 2] {
    let roll = smoothstep(ROLL_SPEED, speed);
    let slide = smoothstep(DIRT_SLIDE, sliding);
    let smoke = smoothstep(SMOKE_SLIDE, sliding) * SMOKE_RATE;
    match surface {
        Surface::Asphalt => [smoke, 0.0],
        Surface::Cobbles => [0.7 * smoke, 0.0],
        Surface::Gravel => [14.0 * roll + 25.0 * slide, 10.0 * roll + 50.0 * slide],
        Surface::Dirt => [10.0 * roll + 18.0 * slide, 14.0 * roll + 60.0 * slide],
        Surface::Field | Surface::Forest => [3.0 * roll + 6.0 * slide, 22.0 * roll + 80.0 * slide],
        Surface::Grass => [1.0 * roll + 3.0 * slide, 14.0 * roll + 50.0 * slide],
        Surface::Water => [12.0 * roll + 10.0 * slide, 60.0 * roll + 40.0 * slide],
    }
}

/// What a wheel at `at` on `surface` throws up: a puff or a chunk, the wheel moving at
/// `velocity`, its tread sliding at `slip` over the ground (`up` its normal).
fn particle(
    spray: &mut Spray,
    surface: Surface,
    shape: Shape,
    at: Vec3,
    velocity: Vec3,
    slip: Vec3,
    up: Vec3,
) -> Particle {
    let shade = 0.85 + 0.3 * spray.random();
    let tint = |c: [f32; 4]| [c[0] * shade, c[1] * shade, c[2] * shade, c[3]];
    let angle = spray.random() * TAU;
    let turn = spray.signed() * 0.6;
    let speed = velocity.length();
    match shape {
        Shape::Puff => {
            let (color, life, size, drag, lift, rise) = match surface {
                Surface::Asphalt | Surface::Cobbles => {
                    (rgb(218, 218, 220, 0.5), 3.5, (0.7, 5.0), 1.2, 0.25, 0.6)
                }
                Surface::Water => (rgb(235, 240, 245, 0.28), 0.9, (0.4, 1.8), 2.5, 0.0, 0.8),
                Surface::Gravel => (rgb(190, 180, 160, 0.3), 2.2, (0.5, 3.2), 1.5, 0.1, 0.6),
                _ => (rgb(150, 125, 90, 0.3), 2.0, (0.5, 3.0), 1.5, 0.1, 0.6),
            };
            Particle {
                shape,
                position: at + up * (size.0 * 0.4) + spray.around(0.15),
                // Kicked back with the tread, left behind by the car.
                velocity: velocity * 0.35 + slip * 0.25 + up * rise + spray.around(0.6),
                color: tint(color),
                age: 0.0,
                life: life * (0.75 + 0.5 * spray.random()),
                size: (size.0, size.1 * (0.8 + 0.4 * spray.random())),
                angle,
                turn,
                drag,
                lift,
                floor: f32::NEG_INFINITY,
            }
        }
        Shape::Chunk => {
            let (color, size, life) = match surface {
                Surface::Gravel => (rgb(140, 134, 124, 1.0), 0.03, 1.5),
                Surface::Grass => {
                    if spray.random() < 0.6 {
                        (rgb(92, 120, 52, 1.0), 0.045, 1.5)
                    } else {
                        (rgb(105, 82, 55, 1.0), 0.04, 1.5)
                    }
                }
                Surface::Water => (rgb(200, 215, 225, 0.5), 0.035, 1.0),
                Surface::Forest => (rgb(80, 62, 42, 1.0), 0.045, 1.5),
                _ => (rgb(110, 85, 58, 1.0), 0.045, 1.5),
            };
            // Off the tread behind the wheel, up and back: with the slip, and more the
            // faster the wheel goes.
            let throw = slip * 0.9 + up * (1.0 + 0.2 * speed.min(25.0) + 0.25 * slip.length());
            Particle {
                shape,
                position: at + up * 0.05 + spray.around(0.1),
                velocity: velocity * 0.55
                    + throw * (0.6 + 0.6 * spray.random())
                    + spray.around(1.0 + 0.1 * speed),
                color: tint(color),
                age: 0.0,
                life: life * (0.7 + 0.6 * spray.random()),
                size: {
                    let s = size * (0.5 + spray.random());
                    (s, s)
                },
                angle,
                turn: turn * 8.0,
                drag: 0.3,
                lift: 0.0,
                floor: at.y - 0.05,
            }
        }
    }
}

/// Every car drawn near the camera throws up what its wheels do.
fn emit(
    time: Res<Time>,
    graphics: Res<Graphics>,
    terrain: Option<Res<Terrain>>,
    mut spray: ResMut<Spray>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    cars: Query<(Entity, &Looks, &Motion, &Transform, &InheritedVisibility)>,
) {
    let dt = time.delta_secs();
    let spray = &mut *spray;
    for track in spray.tracks.values_mut() {
        track.seen = false;
    }
    let (Some(terrain), Ok(camera)) = (terrain, cameras.single()) else {
        spray.tracks.clear();
        return;
    };
    let (most, scale) = match graphics.particles {
        Particles::Off => {
            spray.tracks.clear();
            spray.particles.clear();
            return;
        }
        Particles::Low => (MOST[0], LOW_RATE),
        Particles::High => (MOST[1], 1.0),
    };
    let eye = camera.translation();
    for (entity, looks, motion, transform, visible) in &cars {
        if !visible.get() || transform.translation.distance(eye) > RANGE {
            continue;
        }
        let fresh = !spray.tracks.contains_key(&entity);
        let mut track = spray.tracks.get(&entity).copied().unwrap_or_default();
        track.seen = true;
        for (corner, wheel) in motion.wheels.iter().enumerate() {
            let pose = looks.corner_pose(corner, wheel);
            let radius = looks.axles[corner / 2].radius;
            let center = transform.transform_point(pose.wheel_center);
            let was = track.wheels[corner];
            let mut now = WheelTrack {
                center,
                spin: wheel.spin,
                ..was
            };
            let moved = (center - was.center) / dt.max(1e-4);
            let ground = terrain.ground(center.x, center.z, center.y);
            let touching = center.y - radius - ground.height < TOUCH;
            if fresh || dt <= 0.0 || moved.length() > JUMP || !touching {
                now.slip = Vec3::ZERO;
                now.owed = [0.0; 2];
                track.wheels[corner] = now;
                continue;
            }
            let up = ground.normal;
            let flat = |v: Vec3| v - up * v.dot(up);
            let velocity = flat(moved);
            let forward = flat(transform.rotation * pose.upright * Vec3::NEG_Z).normalize_or_zero();
            let rolling = (wheel.spin - was.spin) / dt * radius;
            let slip = velocity - forward * rolling;
            now.slip = was.slip + (slip - was.slip) * (dt / SLIP_LAG).min(1.0);
            let sliding = now.slip.length();
            let speed = velocity.length().max(rolling.abs());
            let rate = rates(ground.surface, speed, sliding);
            let bottom = Vec3::new(center.x, ground.height, center.z);
            for (kind, shape) in [Shape::Puff, Shape::Chunk].into_iter().enumerate() {
                now.owed[kind] += rate[kind] * scale * dt;
                while now.owed[kind] >= 1.0 {
                    now.owed[kind] -= 1.0;
                    if spray.particles.len() >= most {
                        continue;
                    }
                    // Spread over the frame along the way the wheel went.
                    let back = moved * dt * spray.random();
                    let p = particle(
                        spray,
                        ground.surface,
                        shape,
                        bottom - back,
                        velocity,
                        now.slip,
                        up,
                    );
                    spray.particles.push(p);
                }
            }
            track.wheels[corner] = now;
        }
        spray.tracks.insert(entity, track);
    }
    spray.tracks.retain(|_, track| track.seen);
}

/// Moves the particles on and lets the old ones go.
fn fly(time: Res<Time>, mut spray: ResMut<Spray>) {
    let dt = time.delta_secs();
    spray.particles.retain_mut(|p| {
        p.age += dt;
        p.velocity *= (-p.drag * dt).exp();
        match p.shape {
            Shape::Puff => p.velocity.y += p.lift * dt,
            Shape::Chunk => p.velocity.y -= GRAVITY * dt,
        }
        p.position += p.velocity * dt;
        p.angle += p.turn * dt;
        p.age < p.life && p.position.y > p.floor
    });
}

/// The particles as quads facing the camera, back to front.
fn draw(
    spray: Res<Spray>,
    darkness: Option<Res<crate::grade::Darkness>>,
    cameras: Query<&Transform, With<Camera3d>>,
    mut clouds: Query<(&Mesh3d, &mut Visibility), With<Cloud>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let (Ok(eye), Ok((cloud, mut visibility))) = (cameras.single(), clouds.single_mut()) else {
        return;
    };
    let shown = !spray.particles.is_empty();
    visibility.set_if_neq(if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    if !shown {
        return;
    }
    let Some(mut mesh) = meshes.get_mut(&cloud.0) else {
        return;
    };
    // Unlit, so dimmed at night by hand.
    let light = 1.0 - 0.85 * darkness.map_or(0.0, |d| d.0);
    let (right, up, back) = (eye.right(), eye.up(), eye.back());
    let mut order: Vec<(f32, usize)> = spray
        .particles
        .iter()
        .enumerate()
        .map(|(i, p)| (p.position.distance_squared(eye.translation), i))
        .collect();
    order.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    let n = order.len();
    let mut positions = Vec::with_capacity(n * 4);
    let mut colors = Vec::with_capacity(n * 4);
    let mut uvs = Vec::with_capacity(n * 4);
    let mut indices = Vec::with_capacity(n * 6);
    for (_, i) in order {
        let p = &spray.particles[i];
        let t = p.age / p.life;
        let (size, alpha, u) = match p.shape {
            Shape::Puff => {
                let grown = 1.0 - (-3.0 * t).exp();
                let size = p.size.0 + (p.size.1 - p.size.0) * grown;
                let alpha = (p.age / 0.12).min(1.0) * (1.0 - t).powf(1.5);
                (size, alpha, 0.0)
            }
            Shape::Chunk => (p.size.0, 1.0 - smoothstep((0.8, 1.0), t), 0.5),
        };
        let (sin, cos) = p.angle.sin_cos();
        let half = size / 2.0;
        let (a, b) = (
            (right * cos + up * sin) * half,
            (up * cos - right * sin) * half,
        );
        let base = positions.len() as u32;
        for (corner, uv) in [
            (-a - b, [0.0, 1.0]),
            (a - b, [0.5, 1.0]),
            (a + b, [0.5, 0.0]),
            (-a + b, [0.0, 0.0]),
        ] {
            positions.push((p.position + corner).to_array());
            uvs.push([uv[0] + u, uv[1]]);
            colors.push([
                p.color[0] * light,
                p.color[1] * light,
                p.color[2] * light,
                p.color[3] * alpha,
            ]);
        }
        indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    let normals = vec![back.to_array(); positions.len()];
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roads_smoke_only_when_sliding_and_mud_flies_off_road() {
        assert_eq!(rates(Surface::Asphalt, 30.0, 0.5), [0.0, 0.0]);
        assert!(rates(Surface::Asphalt, 0.0, 15.0)[0] > 40.0);
        assert_eq!(rates(Surface::Asphalt, 0.0, 15.0)[1], 0.0);
        assert!(rates(Surface::Field, 15.0, 0.0)[1] > 5.0);
        assert!(rates(Surface::Field, 5.0, 8.0)[1] > rates(Surface::Field, 5.0, 0.0)[1]);
        assert_eq!(rates(Surface::Dirt, 0.0, 0.0), [0.0, 0.0]);
    }

    #[test]
    fn the_texture_is_a_soft_puff_and_a_blob() {
        let image = texture();
        let data = image.data.unwrap();
        let alpha = |x: u32, y: u32| data[((y * TEXTURE * 2 + x) * 4 + 3) as usize];
        let half = TEXTURE / 2;
        assert!(alpha(half, half) > 100);
        assert_eq!(alpha(0, 0), 0);
        assert_eq!(alpha(TEXTURE + half, half), 255);
        assert_eq!(alpha(TEXTURE, 0), 0);
    }
}
