#!/usr/bin/env bash
# Hooks the screenshot helper (and wheel logger) into the game. Undo with strip.sh before committing.
cp /repo/tools/dev/shot.rs /repo/src/shot.rs
sed -i "s/^mod visuals;\$/mod visuals;\nmod shot;/; s/^        .add_systems(Startup, setup)\$/        .add_systems(Startup, setup)\n        .add_plugins((shot::ShotPlugin, shot::WheelLogPlugin))/" /repo/src/main.rs
