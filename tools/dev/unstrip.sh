#!/usr/bin/env bash
# Hooks the screenshot helper (and wheel logger) into the apps. Undo with strip.sh before committing.
shopt -s nullglob
cp /repo/tools/dev/shot.rs /repo/src/shot.rs
sed -i "s/^pub mod scenery;\$/pub mod scenery;\npub mod shot;/" /repo/src/lib.rs
for app in /repo/src/main.rs /repo/src/bin/*.rs; do
    sed -i "s/^        .add_plugins(CorePlugin)\$/        .add_plugins(CorePlugin)\n        .add_plugins((shot::ShotPlugin, shot::WheelLogPlugin))/" "$app"
    sed -i "s/^        .add_plugins(editor::EditorPlugin)\$/        .add_plugins(editor::EditorPlugin)\n        .add_plugins(shot::EditorShotPlugin)/" "$app"
    sed -i "s/^        .add_plugins(race_editor::RaceEditorPlugin { open })\$/        .add_plugins(race_editor::RaceEditorPlugin { open })\n        .add_plugins(shot::EditorShotPlugin)/" "$app"
done
