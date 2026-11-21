//! Name tags over the cars the server gives a name (`net::client::Tag`: the other
//! players' cars, or none), in the scene like anything else: each name is
//! drawn once into a texture (white with a dark outline, in egui's font through
//! ab_glyph) on a quad that faces the camera, scaled with its distance so it stays about
//! the same size on screen. They're covered by whatever is in front of them (houses,
//! trees, the ground, other cars), like anything else. Whether players' cars have names
//! is the server's choice (`--name-tags`).

use std::collections::HashMap;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use bevy::asset::RenderAssetUsages;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::net::client::{RemoteCar, Tag};

/// Tags show over cars nearer than this (m), their bottom this far above the car's middle.
pub const RANGE: f32 = 150.0;
const HEIGHT: f32 = 1.4;
/// A tag's height as a share of its distance: about 27 px at 720 px in the chase view.
const SIZE: f32 = 0.032;
/// The font's size in the texture, and the outline's width (px).
const FONT: f32 = 30.0;
const OUTLINE: i32 = 3;
/// The longest name drawn (characters).
const LONGEST: usize = 32;

pub struct NameTagPlugin;

impl Plugin for NameTagPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            (spawn, place)
                .chain()
                .in_set(PlaceTags)
                .before(bevy::transform::TransformSystems::Propagate),
        );
    }
}

/// Where the tags are placed for the camera: whatever moves the camera after `Update`
/// goes before it.
#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PlaceTags;

/// A name tag over a car.
#[derive(Component)]
struct Label {
    car: Entity,
    name: String,
    /// Width over height.
    aspect: f32,
}

/// A label for each car with a name (a new one when the name changes); the labels of cars
/// that are gone or lost their name go too.
fn spawn(
    mut commands: Commands,
    named: Query<(Entity, &Tag), With<RemoteCar>>,
    labels: Query<(Entity, &Label)>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut quad: Local<Option<Handle<Mesh>>>,
) {
    let names: HashMap<Entity, &String> = named.iter().map(|(car, tag)| (car, &tag.0)).collect();
    let mut labelled = HashMap::new();
    for (entity, label) in &labels {
        if names.get(&label.car) == Some(&&label.name) {
            labelled.insert(label.car, entity);
        } else {
            commands.entity(entity).despawn();
        }
    }
    for (car, name) in names {
        if labelled.contains_key(&car) {
            continue;
        }
        let (image, aspect) = draw(name);
        let material = StandardMaterial {
            base_color_texture: Some(images.add(image)),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            fog_enabled: false,
            cull_mode: None,
            double_sided: true,
            ..default()
        };
        let quad = quad
            .get_or_insert_with(|| meshes.add(Rectangle::new(1.0, 1.0)))
            .clone();
        commands.spawn((
            Label {
                car,
                name: name.clone(),
                aspect,
            },
            Mesh3d(quad),
            MeshMaterial3d(materials.add(material)),
            Transform::default(),
            Visibility::Hidden,
            NotShadowCaster,
            NotShadowReceiver,
        ));
    }
}

/// Puts each tag over its car, facing the camera at a size for its distance.
fn place(
    cameras: Query<&Transform, (With<Camera3d>, Without<Label>)>,
    cars: Query<&Transform, (With<RemoteCar>, Without<Label>)>,
    mut labels: Query<(&Label, &mut Transform, &mut Visibility)>,
) {
    let Ok(eye) = cameras.single() else {
        return;
    };
    for (label, mut transform, mut visibility) in &mut labels {
        let at = cars
            .get(label.car)
            .map(|car| car.translation + Vec3::Y * HEIGHT);
        let distance = at.map_or(f32::INFINITY, |at| at.distance(eye.translation));
        let shown = distance <= RANGE;
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        let (Ok(at), true) = (at, shown) else {
            continue;
        };
        let height = SIZE * distance;
        *transform = Transform {
            translation: at + Vec3::Y * height / 2.0,
            rotation: eye.rotation,
            scale: Vec3::new(height * label.aspect, height, 1.0),
        };
    }
}

/// `name` in white with a dark outline on a transparent texture just big enough, and its
/// width over its height.
fn draw(name: &str) -> (Image, f32) {
    let font =
        FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).expect("egui's font is a font");
    let scale = PxScale::from(FONT);
    let scaled = font.as_scaled(scale);
    let pad = OUTLINE + 1;
    let mut glyphs = Vec::new();
    let mut x = pad as f32;
    let mut last = None;
    for c in name.chars().take(LONGEST) {
        let id = scaled.glyph_id(c);
        if let Some(last) = last {
            x += scaled.kern(last, id);
        }
        glyphs.push(id.with_scale_and_position(scale, point(x, pad as f32 + scaled.ascent())));
        x += scaled.h_advance(id);
        last = Some(id);
    }
    let width = (x.ceil() as i32 + pad).max(1) as usize;
    let height = ((scaled.ascent() - scaled.descent()).ceil() as i32 + 2 * pad) as usize;
    let mut cover = vec![0.0f32; width * height];
    for glyph in glyphs {
        let Some(outline) = font.outline_glyph(glyph) else {
            continue;
        };
        let bounds = outline.px_bounds();
        outline.draw(|gx, gy, c| {
            let px = bounds.min.x as i32 + gx as i32;
            let py = bounds.min.y as i32 + gy as i32;
            if (0..width as i32).contains(&px) && (0..height as i32).contains(&py) {
                let i = py as usize * width + px as usize;
                cover[i] = cover[i].max(c);
            }
        });
    }
    // The outline: the most cover within its width of each pixel.
    let mut data = Vec::with_capacity(width * height * 4);
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let mut halo = 0.0f32;
            for dy in -OUTLINE..=OUTLINE {
                for dx in -OUTLINE..=OUTLINE {
                    let (nx, ny) = (x + dx, y + dy);
                    if dx * dx + dy * dy <= OUTLINE * OUTLINE + 1
                        && (0..width as i32).contains(&nx)
                        && (0..height as i32).contains(&ny)
                    {
                        halo = halo.max(cover[ny as usize * width + nx as usize]);
                    }
                }
            }
            // White text over the dark outline.
            let text = cover[y as usize * width + x as usize];
            let alpha = text + halo * 0.8 * (1.0 - text);
            let white = if alpha > 0.0 { text / alpha } else { 0.0 };
            let value = (white * 255.0).round() as u8;
            data.extend_from_slice(&[value, value, value, (alpha * 255.0).round() as u8]);
        }
    }
    let image = Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    (image, width as f32 / height as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_drawn_white_on_dark() {
        let (image, aspect) = draw("Anna");
        let data = image.data.unwrap();
        assert!((1.2..5.0).contains(&aspect), "{aspect}");
        let pixels: Vec<&[u8]> = data.chunks(4).collect();
        assert!(
            pixels.iter().any(|p| p[0] > 230 && p[3] > 230),
            "white text"
        );
        assert!(
            pixels.iter().any(|p| p[0] == 0 && p[3] > 150),
            "dark outline"
        );
        assert!(pixels.iter().any(|p| p[3] == 0), "see-through around it");
        // A name that's too long is cut.
        assert!(draw(&"W".repeat(100)).1 < draw(&"W".repeat(40)).1 * 1.01);
    }
}
