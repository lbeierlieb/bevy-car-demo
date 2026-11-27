---
name: fake-wheel
description: Running the virtual uinput wheel (tools/dev/fakewheel) to test wheel input and force feedback headless, and its gotchas.
---

# Virtual wheel (`tools/dev/fakewheel`)

A uinput device named "Fake Wheel" with steering, three pedal axes, buttons and
FF_CONSTANT. It runs a script over wall-clock time (axes wiggle at 20 s so the game
registers them, gate 1 at 32 s, clutch out with throttle from 40 s, steer from 90 s)
and logs the force levels the game uploads.

```sh
cd tools/dev/fakewheel && nix develop /repo -c cargo build --release && cd /repo
sudo tools/dev/fakewheel/target/release/fakewheel > /tmp/fakewheel.log &
bash tools/dev/unstrip.sh
XDG_CONFIG_HOME=/repo/tools/dev/wheel-config WHEELLOG=1 bash tools/dev/shot.sh /tmp/wheel.png 600
bash tools/dev/strip.sh; sudo pkill fakewheel
```

`tools/dev/wheel-config` holds bindings for the fake wheel (note: the game saves
changed settings back there). Gotchas found with it:

- The game runs at a few frames per second under lavapipe; the script runs in wall time.
- Xvfb never focuses the window; `WHEELLOG=1` forces focus so force feedback runs.
- gilrs flips Y axes (ABS_Y reads inverted); binding by moving the control handles it.
- The kernel only reports changes, and gilrs doesn't read initial state: a control
  counts once it has moved.
