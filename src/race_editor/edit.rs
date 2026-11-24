//! The race editor's mouse and keys. Left click on the ground: with the grid tool (G) a
//! grid spot, its box turning with the mouse until the next click (Esc calls it off);
//! with the checkpoint tool (C) the next checkpoint, Ctrl+click one between the two
//! around it. A disc dragged moves what it marks; a click on a grid spot's disc or box
//! turns it again; a click on the first checkpoint's disc makes the route a loop.
//! Shift+wheel turns the selected grid spot a degree a notch, Delete removes what's under
//! the pointer (or the selected grid spot), Ctrl+Z / Ctrl+Y undo and redo.

use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_egui::input::EguiWantsInput;

use super::show::CarBox;
use super::{Draft, Plan};
use crate::editor::camera::{FlyCamera, look_at};
use crate::editor::overlay::ground_hit;
use crate::events::race::{GridSpot, MAX_RACERS};
use crate::map::MapView;
use crate::terrain::Terrain;

/// A disc is under the pointer within this many pixels of its middle.
const PICK: f32 = 16.0;
/// The pointer moving this far (pixels) with the button down is a drag, not a click.
const DRAG: f32 = 4.0;
/// A grid spot turns towards the pointer once it's this far from its middle (m).
const TURN_REACH: f32 = 0.5;

/// The systems that change the plan.
#[derive(SystemSet, Clone, PartialEq, Eq, Hash, Debug)]
pub struct EditSet;

pub struct EditPlugin;

impl Plugin for EditPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tool>()
            .init_resource::<Mode>()
            .init_resource::<Pointer>()
            .init_resource::<Selected>()
            .init_resource::<AimAt>()
            .add_systems(Update, (aim, point, edit).chain().in_set(EditSet));
    }
}

/// What a click on the ground places.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Tool {
    #[default]
    Grid,
    Checkpoints,
}

/// Something placed: a grid spot or a checkpoint, by its index in the plan.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    Spot(usize),
    Check(usize),
}

/// What the mouse is doing.
#[derive(Resource, Default)]
pub enum Mode {
    #[default]
    Idle,
    /// A grid spot facing the pointer until the next click (`before`: the plan to go
    /// back to if it's called off, without it if it was new).
    Turning { spot: usize, before: Plan },
    /// A disc held down: dragged once the pointer moves (`from`, in pixels), else a click
    /// on it.
    Holding {
        target: Target,
        before: Plan,
        from: Vec2,
        moved: bool,
    },
}

/// Where the pointer is.
#[derive(Resource, Default)]
pub struct Pointer {
    /// On the ground, when it's over the 3D view (not the window, the map, or looking
    /// around).
    pub ground: Option<Vec2>,
    /// What's under it: a disc (`on_disc`), or a grid spot's box.
    pub hover: Option<Target>,
    pub on_disc: bool,
}

/// The grid spot Shift+wheel turns (the last one placed, turned, moved or clicked).
#[derive(Resource, Default)]
pub struct Selected(pub Option<usize>);

/// A map point for the camera to look at (opening a route).
#[derive(Resource, Default)]
pub struct AimAt(pub Option<Vec2>);

/// Where a map point is drawn: on the road surface there, or the ground.
pub fn on_ground(terrain: &Terrain, p: Vec2) -> Vec3 {
    Vec3::new(p.x, terrain.ground(p.x, p.y, f32::INFINITY).height, p.y)
}

/// Where a target is on the map.
pub fn target_at(plan: &Plan, target: Target) -> Option<Vec2> {
    match target {
        Target::Spot(i) => plan.grid.get(i).map(|spot| spot.at),
        Target::Check(i) => plan.checkpoints.get(i).copied(),
    }
}

fn aim(
    mut aim: ResMut<AimAt>,
    terrain: Res<Terrain>,
    mut cameras: Query<(&mut FlyCamera, &mut Transform)>,
) {
    let Some(at) = aim.0 else {
        return;
    };
    let Ok((mut camera, mut transform)) = cameras.single_mut() else {
        return;
    };
    look_at(&mut camera, &mut transform, on_ground(&terrain, at), 60.0);
    aim.0 = None;
}

/// Finds where the pointer is on the ground and what's under it.
#[allow(clippy::too_many_arguments)]
fn point(
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<FlyCamera>>,
    egui: Res<EguiWantsInput>,
    view: Res<MapView>,
    buttons: Res<ButtonInput<MouseButton>>,
    (terrain, draft, car, mode): (Res<Terrain>, Res<Draft>, Res<CarBox>, Res<Mode>),
    mut pointer: ResMut<Pointer>,
) {
    *pointer = Pointer::default();
    let busy = !matches!(*mode, Mode::Idle);
    let over_ui = !busy && (egui.wants_pointer_input() || egui.is_pointer_over_area());
    if view.open || over_ui || buttons.pressed(MouseButton::Right) {
        return;
    }
    let (Ok(window), Ok((camera, eye))) = (windows.single(), cameras.single()) else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(eye, cursor) else {
        return;
    };
    pointer.ground = ground_hit(&terrain, ray.origin, *ray.direction).map(|(_, p)| p.xz());
    let plan = &draft.plan;
    let discs = (plan.grid.iter().enumerate())
        .map(|(i, spot)| (Target::Spot(i), spot.at))
        .chain((plan.checkpoints.iter().enumerate()).map(|(i, &p)| (Target::Check(i), p)));
    let mut nearest: Option<(f32, Target)> = None;
    for (target, at) in discs {
        let Ok(screen) = camera.world_to_viewport(eye, on_ground(&terrain, at)) else {
            continue;
        };
        let distance = screen.distance(cursor);
        if distance < PICK && nearest.is_none_or(|(d, _)| distance < d) {
            nearest = Some((distance, target));
        }
    }
    if let Some((_, target)) = nearest {
        pointer.hover = Some(target);
        pointer.on_disc = true;
    } else if let Some(ground) = pointer.ground {
        pointer.hover = (plan.grid.iter())
            .position(|spot| car.covers(spot, ground))
            .map(Target::Spot);
    }
}

/// Clicks, drags, turning and keys.
#[allow(clippy::too_many_arguments)]
fn edit(
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    windows: Query<&Window, With<PrimaryWindow>>,
    egui: Res<EguiWantsInput>,
    pointer: Res<Pointer>,
    (mut mode, mut tool, mut selected): (ResMut<Mode>, ResMut<Tool>, ResMut<Selected>),
    mut draft: ResMut<Draft>,
) {
    let cursor = windows.single().ok().and_then(|w| w.cursor_position());
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    match std::mem::take(&mut *mode) {
        Mode::Turning { spot, before } => {
            if keys.just_pressed(KeyCode::Escape) || spot >= draft.plan.grid.len() {
                draft.cancel(before);
                selected.0 = selected.0.filter(|&i| i < draft.plan.grid.len());
                return;
            }
            if let Some(ground) = pointer.ground {
                let at = draft.plan.grid[spot].at;
                let to = ground - at;
                if to.length() > TURN_REACH {
                    let heading = GridSpot::heading_of(to.normalize());
                    if heading != draft.plan.grid[spot].heading {
                        draft.plan.grid[spot].heading = heading;
                        draft.touch();
                    }
                }
            }
            if buttons.just_pressed(MouseButton::Left) && pointer.ground.is_some() {
                draft.commit(before);
                selected.0 = Some(spot);
            } else {
                *mode = Mode::Turning { spot, before };
            }
        }
        Mode::Holding {
            target,
            before,
            from,
            mut moved,
        } => {
            if let Some(cursor) = cursor {
                moved |= cursor.distance(from) > DRAG;
            }
            if moved && let Some(ground) = pointer.ground {
                let plan = &mut draft.plan;
                let place = match target {
                    Target::Spot(i) => plan.grid.get_mut(i).map(|spot| &mut spot.at),
                    Target::Check(i) => plan.checkpoints.get_mut(i),
                };
                if let Some(place) = place
                    && *place != ground
                {
                    *place = ground;
                    draft.touch();
                }
            }
            if buttons.pressed(MouseButton::Left) {
                *mode = Mode::Holding {
                    target,
                    before,
                    from,
                    moved,
                };
                return;
            }
            if moved {
                draft.commit(before);
                if let Target::Spot(i) = target {
                    selected.0 = Some(i);
                }
                return;
            }
            // A click on a disc.
            match target {
                Target::Spot(i) => {
                    selected.0 = Some(i);
                    *mode = Mode::Turning { spot: i, before };
                }
                Target::Check(0)
                    if *tool == Tool::Checkpoints
                        && !draft.plan.looped
                        && draft.plan.checkpoints.len() >= 2 =>
                {
                    draft.change(|plan| plan.looped = true);
                }
                Target::Check(_) => {}
            }
        }
        Mode::Idle => {
            if ctrl && keys.just_pressed(KeyCode::KeyZ) {
                if shift {
                    draft.redo();
                } else {
                    draft.undo();
                }
            } else if ctrl && keys.just_pressed(KeyCode::KeyY) {
                draft.redo();
            } else if !ctrl && keys.just_pressed(KeyCode::KeyG) {
                *tool = Tool::Grid;
            } else if !ctrl && keys.just_pressed(KeyCode::KeyC) {
                *tool = Tool::Checkpoints;
            } else if keys.any_just_pressed([KeyCode::Delete, KeyCode::Backspace])
                && let Some(target) = (pointer.hover).or(selected.0.map(Target::Spot))
            {
                remove(&mut draft, target);
                selected.0 = None;
            }
            selected.0 = selected.0.filter(|&i| i < draft.plan.grid.len());
            let over_ui = egui.wants_pointer_input() || egui.is_pointer_over_area();
            if shift
                && scroll.delta.y != 0.0
                && !over_ui
                && let Some(i) = selected.0
            {
                let step = scroll.delta.y.signum();
                draft.change_merged((0, i), |plan| {
                    let spot = &mut plan.grid[i];
                    spot.heading = (spot.heading.round() + step).rem_euclid(360.0);
                });
            }
            if !buttons.just_pressed(MouseButton::Left) {
                return;
            }
            if let (Some(target), true) = (pointer.hover, pointer.on_disc) {
                *mode = Mode::Holding {
                    target,
                    before: draft.plan.clone(),
                    from: cursor.unwrap_or_default(),
                    moved: false,
                };
                return;
            }
            if let Some(Target::Spot(i)) = pointer.hover {
                selected.0 = Some(i);
                *mode = Mode::Turning {
                    spot: i,
                    before: draft.plan.clone(),
                };
                return;
            }
            let Some(ground) = pointer.ground else {
                return;
            };
            match *tool {
                Tool::Grid if draft.plan.grid.len() >= MAX_RACERS => {
                    draft.note = Some((format!("{MAX_RACERS} grid spots at most."), true));
                }
                Tool::Grid => {
                    let before = draft.plan.clone();
                    // Facing the way the last one placed faces, until turned.
                    let heading = (selected.0.or(draft.plan.grid.len().checked_sub(1)))
                        .map_or(0.0, |i| draft.plan.grid[i].heading);
                    draft.plan.grid.push(GridSpot {
                        at: ground,
                        heading,
                    });
                    draft.touch();
                    let spot = draft.plan.grid.len() - 1;
                    selected.0 = Some(spot);
                    *mode = Mode::Turning { spot, before };
                }
                Tool::Checkpoints => {
                    let at = if ctrl {
                        insert_index(&draft, ground)
                    } else {
                        draft.plan.checkpoints.len()
                    };
                    draft.change(|plan| plan.checkpoints.insert(at, ground));
                }
            }
        }
    }
}

/// Takes a grid spot or checkpoint out (one undo step).
pub fn remove(draft: &mut Draft, target: Target) {
    draft.change(|plan| match target {
        Target::Spot(i) if i < plan.grid.len() => {
            plan.grid.remove(i);
        }
        Target::Check(i) if i < plan.checkpoints.len() => {
            plan.checkpoints.remove(i);
        }
        _ => {}
    });
}

/// Where a checkpoint at `p` goes into the list: between the two around the part of the
/// route nearest it (along the roads, if the route was found for these checkpoints, else
/// the straight legs between them).
fn insert_index(draft: &Draft, p: Vec2) -> usize {
    let checks = &draft.plan.checkpoints;
    let n = checks.len();
    if n < 2 {
        return n;
    }
    let route = &draft.route;
    if route.checkpoints == *checks && route.marks.len() >= n && route.line.len() >= 2 {
        let nearest = (0..route.line.len() - 1)
            .min_by(|&a, &b| {
                let d = |i: usize| to_segment(p, route.line[i], route.line[i + 1]);
                d(a).total_cmp(&d(b))
            })
            .unwrap_or(0);
        let leg = (route.marks.windows(2))
            .position(|w| usize::from(w[0]) <= nearest && nearest < usize::from(w[1]));
        if let Some(leg) = leg {
            return leg + 1;
        }
    }
    let closing = (draft.plan.looped).then_some((n - 1, 0));
    let legs = (0..n - 1).map(|i| (i, i + 1)).chain(closing);
    legs.min_by(|a, b| {
        let d = |(i, j): (usize, usize)| to_segment(p, checks[i], checks[j]);
        d(*a).total_cmp(&d(*b))
    })
    .map_or(n, |(i, _)| i + 1)
}

/// How far `p` is from the segment `a`–`b`.
fn to_segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let t = ((p - a).dot(b - a) / (b - a).length_squared().max(1e-6)).clamp(0.0, 1.0);
    a.lerp(b, t).distance(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checkpoint_goes_in_between_the_two_around_it() {
        let mut draft = Draft::default();
        draft.plan.checkpoints = vec![
            Vec2::new(0.0, 0.0),
            Vec2::new(100.0, 0.0),
            Vec2::new(100.0, 100.0),
        ];
        assert_eq!(insert_index(&draft, Vec2::new(50.0, 5.0)), 1);
        assert_eq!(insert_index(&draft, Vec2::new(95.0, 50.0)), 2);
        // A loop's closing leg is the end of the list.
        assert_eq!(insert_index(&draft, Vec2::new(60.0, 45.0)), 2);
        draft.plan.looped = true;
        assert_eq!(insert_index(&draft, Vec2::new(60.0, 45.0)), 3);
    }

    #[test]
    fn turning_and_dragging_are_one_step_each() {
        let mut draft = Draft::default();
        draft.change(|plan| plan.checkpoints.push(Vec2::ZERO));
        let before = draft.plan.clone();
        draft.plan.checkpoints[0] = Vec2::ONE;
        draft.touch();
        draft.plan.checkpoints[0] = Vec2::splat(2.0);
        draft.touch();
        draft.commit(before);
        draft.undo();
        assert_eq!(draft.plan.checkpoints, [Vec2::ZERO]);
        draft.redo();
        assert_eq!(draft.plan.checkpoints, [Vec2::splat(2.0)]);
        // Wheel notches on one spot merge.
        draft.change(|plan| {
            plan.grid.push(GridSpot {
                at: Vec2::ZERO,
                heading: 0.0,
            })
        });
        for _ in 0..5 {
            draft.change_merged((0, 0), |plan| plan.grid[0].heading += 1.0);
        }
        assert_eq!(draft.plan.grid[0].heading, 5.0);
        draft.undo();
        assert_eq!(draft.plan.grid[0].heading, 0.0);
    }
}
