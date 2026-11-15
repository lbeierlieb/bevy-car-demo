#!/usr/bin/env bash
# Removes the screenshot hook again; afterwards `grep -c shot src/main.rs` must print 0.
sed -i "/^mod shot;\$/d; /^        .add_plugins((shot::ShotPlugin, shot::WheelLogPlugin))\$/d" /repo/src/main.rs
rm -f /repo/src/shot.rs
