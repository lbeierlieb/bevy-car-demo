//! The sky: a dome around the camera with a texture worked out at startup, deep blue
//! overhead fading to a pale haze at the horizon (the fog's colour, so the land melts into
//! it), a glow around the sun and a layer of fair-weather clouds lit from the sun's side.

use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::transform::TransformSystems;

use crate::terrain::hash;

/// Towards the sun (not normalised): where the sunlight comes from.
pub const SUN: Vec3 = Vec3::new(30.0, 60.0, 20.0);
/// The haze at the horizon, also the fog's colour.
pub const HAZE: Color = Color::srgb(0.74, 0.83, 0.92);
/// Straight up.
const ZENITH: Color = Color::srgb(0.13, 0.38, 0.84);
/// The sun's glow and its disc.
const GLOW: Color = Color::srgb(1.0, 0.93, 0.78);
/// The clouds' sunlit tops and their shaded undersides.
const CLOUD_LIT: Color = Color::srgb(1.0, 0.99, 0.96);
const CLOUD_SHADE: Color = Color::srgb(0.66, 0.71, 0.80);

/// The dome's radius (m): inside the camera's far plane (1000 m), outside the loaded land.
const RADIUS: f32 = 900.0;
/// The texture: azimuth across, elevation from the zenith (top) to the horizon (bottom).
const WIDTH: usize = 2048;
const HEIGHT: usize = 512;
/// The cloud layer's scale (noise cells per unit of height) and how much of the sky the
/// clouds cover (the noise level they start at, 0..1).
const CLOUD_SCALE: f32 = 1.3;
const CLOUD_COVER: f32 = 0.5;

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(HAZE))
            .add_systems(Startup, spawn)
            .add_systems(
                PostUpdate,
                follow_camera.before(TransformSystems::Propagate),
            );
    }
}

#[derive(Component)]
struct SkyDome;

fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let texture = images.add(texture());
    commands.spawn((
        SkyDome,
        Mesh3d(meshes.add(dome())),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(texture),
            unlit: true,
            fog_enabled: false,
            cull_mode: None,
            ..default()
        })),
        Transform::default(),
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

/// Keeps the dome centred on the camera, so it's always as far away.
fn follow_camera(
    cameras: Query<&Transform, (With<Camera3d>, Without<SkyDome>)>,
    mut domes: Query<&mut Transform, With<SkyDome>>,
) {
    let Some(camera) = cameras.iter().next() else {
        return;
    };
    for mut dome in &mut domes {
        dome.translation = camera.translation;
    }
}

/// A latitude-longitude sphere cap from a little below the horizon (all haze) to the
/// zenith, with the texture's coordinates linear in azimuth and elevation.
fn dome() -> Mesh {
    const SEGMENTS: usize = 96;
    const RINGS: usize = 40;
    // The bottom ring, below the horizon, so looking down a slope shows no gap.
    let mut elevations = vec![-0.6];
    elevations.extend((0..=RINGS).map(|i| FRAC_PI_2 * i as f32 / RINGS as f32));
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    for &e in &elevations {
        for s in 0..=SEGMENTS {
            let u = s as f32 / SEGMENTS as f32;
            let a = u * TAU;
            let d = Vec3::new(e.cos() * a.cos(), e.sin(), e.cos() * a.sin());
            positions.push((d * RADIUS).to_array());
            normals.push((-d).to_array());
            uvs.push([u, 1.0 - e.max(0.0) / FRAC_PI_2]);
        }
    }
    let row = SEGMENTS as u32 + 1;
    let mut indices = Vec::new();
    for r in 0..elevations.len() as u32 - 1 {
        for s in 0..SEGMENTS as u32 {
            let (a, b) = (r * row + s, (r + 1) * row + s);
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// The sky's texture, in the dome's coordinates.
fn texture() -> Image {
    let sun = SUN.normalize();
    let mut data = Vec::with_capacity(WIDTH * HEIGHT * 4);
    for j in 0..HEIGHT {
        let e = FRAC_PI_2 * (1.0 - (j as f32 + 0.5) / HEIGHT as f32);
        for i in 0..WIDTH {
            let a = TAU * (i as f32 + 0.5) / WIDTH as f32;
            let d = Vec3::new(e.cos() * a.cos(), e.sin(), e.cos() * a.sin());
            let color = Srgba::from(LinearRgba::from_vec3(sky(d, sun)));
            data.extend(color.to_u8_array());
        }
    }
    Image::new(
        Extent3d {
            width: WIDTH as u32,
            height: HEIGHT as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

fn linear(color: Color) -> Vec3 {
    color.to_linear().to_vec3()
}

/// The sky's colour (linear) looking along `d`, with the sun towards `sun`.
fn sky(d: Vec3, sun: Vec3) -> Vec3 {
    let up = d.y.max(0.0);
    let toward = d.dot(sun).max(0.0);
    // Haze thins quickly above the horizon.
    let mut color = linear(ZENITH).lerp(linear(HAZE), (-up * 3.2).exp());
    // A broad glow round the sun, a tighter halo and the disc.
    let glow = linear(GLOW);
    color += glow * (0.18 * toward.powi(6) + 0.35 * toward.powi(120));
    let cos_edge = 0.55_f32.to_radians().cos();
    let disc = ((toward - cos_edge) / (1.0 - cos_edge) * 6.0).clamp(0.0, 1.0);
    color = color.lerp(Vec3::splat(1.0), disc);

    // The clouds: a flat layer, seen in perspective, fading into the haze at the horizon.
    if up > 0.02 {
        let p = Vec2::new(d.x, d.z) / up * CLOUD_SCALE;
        // How far apart neighbouring texels land on the layer, so octaves finer than that
        // are left out instead of turning to noise.
        let footprint = CLOUD_SCALE * (FRAC_PI_2 / HEIGHT as f32) / (up * up);
        let n = fbm(p, footprint);
        let density = smooth(CLOUD_COVER, CLOUD_COVER + 0.16, n);
        if density > 0.0 {
            // Thick parts are shaded underneath; the side facing the sun is lit, with a
            // silver edge where the cloud is thin.
            let thick = smooth(CLOUD_COVER + 0.06, CLOUD_COVER + 0.32, n);
            let mut cloud = linear(CLOUD_LIT).lerp(linear(CLOUD_SHADE), thick * 0.8);
            cloud += glow * (1.0 - thick) * 0.5 * toward.powi(10);
            // Far clouds take on the haze.
            let distance = smooth(0.0, 0.35, up);
            let cloud = linear(HAZE).lerp(cloud, distance);
            color = color.lerp(cloud, density * 0.95 * smooth(0.02, 0.12, up));
        }
    }
    color
}

fn smooth(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Fractal value noise, 0..1, leaving out octaves finer than `footprint`.
fn fbm(p: Vec2, footprint: f32) -> f32 {
    let mut sum = 0.0;
    let mut total = 0.0;
    let mut amplitude = 1.0;
    let mut frequency = 1.0;
    for octave in 0..6 {
        // Fade an octave out as its cells shrink towards a texel.
        let keep = smooth(0.5, 0.15, footprint * frequency);
        if keep <= 0.0 {
            break;
        }
        // Each octave turned a little so their grids don't line up.
        let q = Vec2::from_angle(octave as f32 * 0.9).rotate(p) * frequency;
        sum += amplitude * keep * value_noise(q, octave);
        total += amplitude * keep;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    // Missing fine octaves leave the average where it was.
    if total > 0.0 { sum / total } else { 0.5 }
}

fn value_noise(p: Vec2, seed: u32) -> f32 {
    let cell = p.floor();
    let f = p - cell;
    let (x, y) = (cell.x as i32, cell.y as i32);
    let corner = |dx: i32, dy: i32| {
        let n = (x.wrapping_add(dx) as u32)
            .wrapping_mul(73_856_093)
            .wrapping_add((y.wrapping_add(dy) as u32).wrapping_mul(19_349_663))
            .wrapping_add(seed.wrapping_mul(83_492_791));
        hash(n)
    };
    let s = f * f * (Vec2::splat(3.0) - 2.0 * f);
    let bottom = corner(0, 0) + (corner(1, 0) - corner(0, 0)) * s.x;
    let top = corner(0, 1) + (corner(1, 1) - corner(0, 1)) * s.x;
    bottom + (top - bottom) * s.y
}
