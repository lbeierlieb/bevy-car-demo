#!/usr/bin/env bash
# Removes the screenshot hook again; afterwards `grep -c shot src/lib.rs src/main.rs src/bin/*.rs`
# must print 0 for each.
shopt -s nullglob
sed -i "/^pub mod shot;\$/d" /repo/src/lib.rs
for app in /repo/src/main.rs /repo/src/bin/*.rs; do
    sed -i "/^        .add_plugins((shot::ShotPlugin, shot::WheelLogPlugin))\$/d" "$app"
done
rm -f /repo/src/shot.rs
