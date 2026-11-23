//! Streams the map's meshes: only chunks near the camera are built and kept on the GPU,
//! so memory stays the same however big the map is. Each loaded chunk has three meshes:
//! ground, road ribbons, and objects (houses, trees and fences).

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::buildings::Buildings;
use crate::fences::Fences;
use crate::terrain::{FlatMesh, RoadRibbons, Terrain};
use crate::trees::Forest;

/// Chunks whose centre is within this distance of the camera are loaded: just past the
/// fog (which ends at 480 m) plus half a chunk diagonal.
const LOAD_DISTANCE: f32 = 560.0;
/// Loaded chunks are dropped beyond this, a little further out so they don't flicker
/// in and out at the edge.
const UNLOAD_DISTANCE: f32 = 660.0;
/// Chunks built per frame once the first view is loaded.
const BUILDS_PER_FRAME: usize = 6;

pub struct SceneryPlugin;

impl Plugin for SceneryPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Reload>()
            .add_systems(Startup, setup)
            .add_systems(PostUpdate, (reload, stream).chain());
    }
}

/// Sent after the `Terrain` resource was replaced (the map editor's re-bake): every chunk
/// is built again from the new map.
#[derive(Message)]
pub struct Reload;

#[derive(Resource)]
struct Scenery {
    roads: RoadRibbons,
    buildings: Buildings,
    fences: Fences,
    forest: Forest,
    ground_material: Handle<StandardMaterial>,
    road_material: Handle<StandardMaterial>,
    object_material: Handle<StandardMaterial>,
    /// Entities of each loaded chunk, by chunk index.
    loaded: Vec<Option<Vec<Entity>>>,
}

fn setup(
    mut commands: Commands,
    terrain: Res<Terrain>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(Scenery {
        roads: RoadRibbons::new(&terrain),
        buildings: Buildings::new(&terrain),
        fences: Fences::new(&terrain),
        forest: Forest::new(&terrain),
        ground_material: materials.add(StandardMaterial {
            perceptual_roughness: 0.95,
            ..default()
        }),
        road_material: materials.add(StandardMaterial {
            perceptual_roughness: 0.9,
            depth_bias: 1.0,
            ..default()
        }),
        object_material: materials.add(StandardMaterial {
            perceptual_roughness: 0.85,
            ..default()
        }),
        loaded: vec![None; terrain.chunk_count()],
    });
}

fn reload(
    mut commands: Commands,
    mut reloads: MessageReader<Reload>,
    terrain: Res<Terrain>,
    mut scenery: ResMut<Scenery>,
) {
    if reloads.read().count() == 0 {
        return;
    }
    for entity in scenery.loaded.iter_mut().filter_map(Option::take).flatten() {
        commands.entity(entity).despawn();
    }
    scenery.roads = RoadRibbons::new(&terrain);
    scenery.buildings = Buildings::new(&terrain);
    scenery.fences = Fences::new(&terrain);
    scenery.forest = Forest::new(&terrain);
    scenery.loaded = vec![None; terrain.chunk_count()];
}

fn stream(
    mut commands: Commands,
    terrain: Res<Terrain>,
    mut scenery: ResMut<Scenery>,
    mut meshes: ResMut<Assets<Mesh>>,
    camera: Query<&GlobalTransform, With<Camera3d>>,
) {
    let Ok(camera) = camera.single() else {
        return;
    };
    let eye = camera.translation().xz();
    let distance = |index: usize| {
        let (lo, hi) = terrain.chunk_bounds(index);
        ((lo + hi) / 2.0).distance(eye)
    };

    for index in 0..scenery.loaded.len() {
        if scenery.loaded[index].is_some() && distance(index) > UNLOAD_DISTANCE {
            // Despawning drops the last handles to the chunk's meshes, which frees them.
            for entity in scenery.loaded[index].take().unwrap() {
                commands.entity(entity).despawn();
            }
        }
    }

    let mut wanted: Vec<(usize, f32)> = (0..scenery.loaded.len())
        .filter(|&index| scenery.loaded[index].is_none())
        .map(|index| (index, distance(index)))
        .filter(|&(_, d)| d < LOAD_DISTANCE)
        .collect();
    wanted.sort_by(|a, b| a.1.total_cmp(&b.1));
    // Load the whole first view at once, then a few chunks per frame, nearest first.
    let first_view = scenery.loaded.iter().all(Option::is_none);
    let budget = if first_view {
        usize::MAX
    } else {
        BUILDS_PER_FRAME
    };
    for (index, _) in wanted.into_iter().take(budget) {
        let entities = build_chunk(&mut commands, &terrain, &scenery, &mut meshes, index);
        scenery.loaded[index] = Some(entities);
    }
}

fn build_chunk(
    commands: &mut Commands,
    terrain: &Terrain,
    scenery: &Scenery,
    meshes: &mut Assets<Mesh>,
    index: usize,
) -> Vec<Entity> {
    let (mut ground, mut roads, mut objects) = Default::default();
    scenery.roads.ground_mesh(terrain, index, &mut ground);
    scenery.roads.mesh(terrain, index, &mut roads, &mut objects);
    scenery.buildings.mesh(terrain, index, &mut objects);
    scenery.fences.mesh(terrain, index, &mut objects);
    scenery.forest.mesh(terrain, index, &mut objects);

    // Ground and roads don't cast shadows: on gentle real terrain self-shadowing is barely
    // visible, and skipping it keeps the big meshes out of the shadow pass. They still
    // receive shadows from houses, trees and cars.
    let layers: [(FlatMesh, &Handle<StandardMaterial>, bool); 3] = [
        (ground, &scenery.ground_material, false),
        (roads, &scenery.road_material, false),
        (objects, &scenery.object_material, true),
    ];
    let mut entities = Vec::new();
    for (mesh, material, casts_shadows) in layers {
        if mesh.is_empty() {
            continue;
        }
        let mut entity = commands.spawn((
            Mesh3d(meshes.add(mesh.build())),
            MeshMaterial3d(material.clone()),
        ));
        if !casts_shadows {
            entity.insert(NotShadowCaster);
        }
        entities.push(entity.id());
    }
    entities
}
