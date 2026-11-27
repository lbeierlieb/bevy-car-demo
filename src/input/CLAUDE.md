# Notes: Wheel support

Part of the repo's notes for Claude Code (conventions in `/repo/CLAUDE.md`). Loaded when working in `src/input/`; keep it up to date.

## Wheel support

Wheel and pedals come through gamepad input (gilrs), bound in a control profile in the
central menu (Esc, Settings, Controls) and saved to a config file. `src/wheel_base.rs` sets the base's rotation range
and plays force feedback, both through hid-fanatecff on Linux.

### Design decisions (agreed with the user)

- Target: Linux with hid-fanatecff. Force feedback only there; input works everywhere.
- Force feedback: the rack torque (`sim/steering.rs`, `Rack`, averaged over the
  substeps): every front tire force's moment about its steering axis from the linkage
  (the axis tilted back for 30 mm of caster trail, which the linkage lacks), pneumatic
  trail (falls with slip angle and slip ratio), Fx at the carcass' sideways deflection,
  and the driveshaft torque along the shaft (inner joints by the differential, 0.12 m
  left of the middle with a transverse gearbox: torque steer). Asked for by the user
  (light cars felt too light, wanted bumps and FWD/AWD power): full scale is 0.055 m ×
  the front axle's weight (`full_scale`, every car ~0.6 at 0.7 g), and "Road feel"
  (`ffb_road`, default 2.5) scales what's off a 0.03 s low-pass. `steering_feel`
  (ignored) prints every car. Damping/friction from the base's tuning
  menu (evdev 0.13 can't send damper parameters). Rate-limited, fades in with speed.
- The game sets the base's rotation range (default 540°) and restores it on exit;
  either end is full lock. If it can't, soft lock like BeamNG: the base's range is read
  from sysfs (readable even when not writable) or given in the menu, the axis is
  scaled to the game's range, and force feedback pushes back past either end (full
  force 10° past; stays on with the tire forces switched off). Only the force feedback
  device's axis covers the base's range (`WheelBase::axis_range`; on Linux the device
  taking a constant force): any other steering axis, a pad's stick, covers the game's
  (it snapped to full lock at a small push with the base plugged in, 2026-11-24), on a
  curve. **Curves** (asked for by the user, 2026-11-24): each control profile has
  `input::Curves` for steering, gas and brake. Drawn curves since 2026-11-24 (asked for
  by the user, like a MIDI controller's): `input::Curve` is 2–9 points (ends at push 0
  and 1, any height), joined by a monotone cubic (Fritsch–Carlson, no overshoot) and
  baked into a 257-entry table whenever the points change, so `Curve::at` is a lookup
  and a lerp. `input::Preset`s: Linear, Gentle (^1.25, gas default), Progressive (^1.5,
  brake default), Stick (^1.8, steering default), Precise (^2.6), Smooth (S), Quick.
  `wheel.cfg`: `curve steer|throttle|brake x,y x,y …` after the `profile` line; an
  older `curves s g b` (exponents) or `stick_curve` becomes `Curve::power` (five
  points). The menu: Controls, Bindings, Curves, a screen per curve
  (`menu::curve::editor`): the graph with the control's push now (`input::push_value`),
  left/right pick a point, choose grabs it, arrows move it, choose keeps, back puts it
  back (`CentralMenu::grabbed`/`cancel_grab`); the mouse drags, double-click adds,
  right-click removes; rows for the preset, adding and removing points. Always applied, no guessing what's a stick (triggers report as
  buttons 0..1 or axes -1..1, some pedals share one axis): a wheel profile sets them to 1;
  the steering's never applies to the force feedback wheel's axis. A pre-profile file's
  "Wheel" profile is linear; an older global `stick_curve` goes into every profile.
- Starter (asked for by the user, 2026-11-22): the ignition button (I or bound) held
  runs the starter (`Drivetrain::starter`: `STARTER_TORQUE` falling to nothing at
  `STARTER_SPEED`, ignition on); the ECU fires after `SYNC_TURNS` revolutions and the
  engine has caught past `CATCH_RPM` (0.5 s for small engines to 1.25 s for the V12);
  pressed while running it switches the engine off (`switch_off`), which stays off (the
  automatic clutch's restart is only for stalls). Cranking is heard as the engine at
  the starter's speed, with the starter's whine (`audio/effects.rs`).
- Clutch pedal: engine can stall, the starter restarts it. Without a bound pedal the
  clutch is automatic and the engine never stalls. The pedal works like a real one
  (`drivetrain::ENGAGEMENT`, asked for by the user: it had to be floored and stalled at
  a slight lift): fully engaged over the top 12 %, apart over the bottom 28 % (and gears
  go in from there), biting at 55 % with 3 % of the capacity (creeps at idle), rising
  steeply to the top. On idle alone it pulls away when held around 45–50 % until
  rolling; with 10 % throttle even a half-second release works (stock coupé, 350 N·m).
- No automatic gearbox for the player (removed at the user's wish, 2026-11-20; real
  automatic gearboxes are to come later). `Drivetrain::automatic` (rpm-based shifting)
  is only kept for the test drivers (consumption, flat-out runs).
- H-shifter: with a clutch pedal bound, a gear only goes in with the clutch pressed
  (otherwise neutral, grinding), or when the engine already turns within `SYNC_RPM`
  (150) of the gear's speed (engine off and standing, or rev-matched; asked for by the
  user, 2026-11-22). Without a pedal, the automatic clutch shifts, blipping the throttle
  up to the lower gear's speed on a downshift (`BLIP_RPM`). Paddles shift
  with the automatic clutch even when a pedal is bound.
- Each H gate is a button held while the lever is in it; neutral is "no gate held".
- Put in neutral by the game (`Drivetrain::neutral`: a reset, the rescue, a round's start
  or end, a car built anew), the gate the lever is still in counts as neutral until the
  lever moves (`Drivetrain::parked`, `lever`; asked for by the user, 2026-11-26: it went
  straight back into gear and stalled or ground). A car put down afresh doesn't know the
  lever, so the first gate it sees waits too. Protocol 30 (snapshot fields).
- No hill hold (removed at the user's wish): the handbrake is a parking brake worked
  by Space or a handbrake button (`Car::parking_brake`, on when a car spawns or resets,
  a red (P) under the gear); a handbrake lever (an axis) works directly on top of it.
  `car::parking_brake_rule` (the user, 2026-11-24, "keep it simple"): pressed while on,
  it comes off; pressed standing (under `PARKING_SPEED`, 0.5 m/s), it goes on; pressed
  while driving, no toggle, the handbrake only brakes while held; **any gas releases
  it**, whatever the gear or clutch. Holding it until the car stops doesn't park it.

- Gamepad rumble on crashes (asked for by the user, 2026-11-24: only without settings):
  `rumble.rs`, bevy's `GamepadRumbleRequest` (bevy_gilrs plays it), on the gamepads the
  profile in use binds, never the force feedback wheel; the crash rule is the crash
  sound's (`effects::CrashGate`; read in `Update`, before `effects::cue` clears the
  knocks); not while away, held or while the server has the car; native only. Strengths are a first guess
  (`WEAK`, `LENGTH`), never felt here.

### Still to check on the real wheel

- "Test: push left" in the menu (Controls, Force feedback): turn on "Reverse direction" if it pushes right (the sign
  convention of hid-fanatecff couldn't be verified without hardware).
- Tune the default force strength (0.5) and road feel (2.5) on the real wheel; the
  physical road content is small (std ~2 % of full scale on a rough road, ~5 % floored
  in a FWD car), the 0.5 per-step smoothing in `force_feedback` also eats some of it.
- That the game can set the rotation range (see below; last seen failing), and the soft
  lock where it can't (strength and feel on a direct drive base; may need damping).

### Troubleshooting on the user's machine

The base is a CSL DD in PC mode (USB `0eb7:0020`); it shows up as two input devices,
both named "Fanatec FANATEC Wheel" (padded with spaces). NixOS config uses
`hardware.hid-fanatecff.enable = true`, which adds the kernel module and its udev rules.

- **"No constant-force support" on every device:** the base is bound to `hid-generic`
  (input works, no force feedback) instead of the `fanatec` driver. Check
  `basename "$(readlink -f /sys/class/input/eventN/device/device/driver)"`,
  `lsmod | grep hid_fanatec`, `sudo evtest /dev/input/eventN` (EV_FF with FF_CONSTANT).
  A reboot into a kernel with the module fixed it; replugging the base can also help.
- **"Couldn't set the wheel range in …/range: Permission denied":** the game falls
  back to the soft lock (the menu says "Wheel base is at …°"). The `range` sysfs file is root's; the udev
  rule only does `chown -f :games` on it when the device is added. Needs a `games` group
  (NixOS doesn't create one; `users.groups.games = { };`), the user in it (log in
  again), and the rule to have run after that (replug, or
  `sudo udevadm trigger --action=change --subsystem-match=hid`). Check with
  `ls -l /sys/module/hid_fanatec/drivers/hid:fanatec/*/range`, `getent group games`
  and `id`. If the group owns it but can't write, the mode needs `g+w` too.
- The menu (Controls, "Wheel, pedals and gamepad") shows "Wheel base set to 540°" once the range was written, and names the
  force feedback device or what's wrong with each candidate.
