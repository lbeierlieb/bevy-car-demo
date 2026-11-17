#!/usr/bin/env bash
# Hooks the screenshot helper (and wheel logger) into both apps. Undo with strip.sh before committing.
shopt -s nullglob
cp /repo/tools/dev/shot.rs /repo/src/shot.rs
sed -i "s/^pub mod scenery;\$/pub mod scenery;\npub mod shot;/" /repo/src/lib.rs
for app in /repo/src/main.rs /repo/src/bin/*.rs; do
    sed -i "s/^        .add_plugins(CorePlugin)\$/        .add_plugins(CorePlugin)\n        .add_plugins((shot::ShotPlugin, shot::WheelLogPlugin))/" "$app"
done
