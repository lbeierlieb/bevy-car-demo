# Notes: Helicopters

Part of the repo's notes for Claude Code (conventions in `/repo/CLAUDE.md`). Loaded when working in `src/heli/`; keep it up to date.

## Helicopters (asked for by the user, 2026-11-22)

`src/heli/`: `mod.rs` (`HeliPlugin` in `CorePlugin`: the model made of primitives, red
with a white stripe, rotors turning with a blur disc, flashing beacons; `Heli { cable,
load }` + `Transform` is one on screen; `Flown` ones (another player's rescue, the hunt's)
are dead reckoned from their latest `HeliPose`; never fogged, and drawn at 900 m shrunk to
look as big when farther, beacons at least ~0.004 rad across, so the hunt's is seen from
anywhere; `Chase`, the hunt's flight) and `rescue.rs`.

- **Rescue** (R with `ResetRule::Road`, also a hunter's in a hunt): flies in from 140 m
  ahead of the car (where the chase camera looks), holds over its roof on a 12 m cable
  (longer to stay 4 m over trees and houses within 8 m: their `Upright::top`), lets the
  cable down, lifts the car (upright), carries it at ~10 m/s average over a lift clearing
  land, trees and houses along the way, sets it down at the drive spot (a random way along the road, see "Beginner help"), unhooks
  (`Car::place`) and flies off 8 s. ~10 s near a road
  (`the_helicopter_takes_the_car_to_the_road`). Neutral and the parking brake on from the
  call, also once set down. Lets go if the car is moved by something else (> 2 m) or the
  driver's away.
  **The server's alone** (agreed with the user, 2026-11-24: a lean client; a later quick
  transport or reset to the garage needs no client change): R is only
  `DriverInput::reset` (a press in an input that came too late for its step is pressed in
  the next, `Shadow::late`: at a few frames a second R got lost); `Shadow` holds the
  `Rescue`, calls it by its `ResetRule`
  (`Round::reset_rule`, `Shadow::rule`; `Rescue::press`), drives the car as nobody's at
  the wheel while it has it, runs `rescue::fly` after the collisions
  (`Shadow::finish_step`) and refuses placements meanwhile. While it has the car
  (`Shadow::taken`), `Yours::notice` carries what to tell the player ("⚠ You are being
  rescued ⚠\nPlease do not resist!"), and the server sends the player their own car like
  anyone's (`Cars`, no tag; our number comes in `Welcome::car`). The client
  (`car::Spectating` holds the notice) puts `car::ServerDriven` on the active car: not
  driven, simulated, predicted, checked or force-fed, hidden, its engine hushed, its body
  and `Transform` following the remote car of our number (`client::spectate`), which is
  drawn and heard (helicopter from `CarState::heli`) like any other; the HUD shows the
  notice (`hud::server_notice`, first line large; only the border's colour pulses, not its
  width, which resized the frame and wrapped the last letter, and its lines never wrap). **Scripted cars** (protocol 29,
  asked for by the user, 2026-11-26): while the server has a car (`Shadow::taken`) its
  states carry `CarState::scripted` ("nothing a game does changes where it goes"), and
  every game shows it `SCRIPTED_DELAY` (0.1 s) before what's heard, on a cubic between
  the states around that time (`client::interpolate`; `KEPT` 12 states), carried on at
  most `SCRIPTED_AHEAD` past the latest, instead of dead reckoning it (which overshot as
  the helicopter sped up, slowed and set down); its rescue helicopter is drawn at the
  same time between its poses (`RemoteCar::heli_at`, `fly_flown`), so the cable stays on
  the roof. Generic, for later transports; the hunt's helicopter (not a car) is still
  dead reckoned (`a_scripted_car_is_shown_on_its_way_between_its_states`). Set down, the server gives it back as
  a move (`Yours::moved`, notice none), the client takes it up and predicts again
  (`the_server_rescues_a_players_car_while_their_game_looks_on`). The menu's reset row is
  enabled whenever we're driving: whether R does anything is the server's call. No sound
  yet.
- **Hunt helicopter**: the server flies a `Chase` per running round in `run_rounds` over
  the hunted (50 m up, at least 30 m over the ground; no top speed, acceleration ≤ 7 m/s²,
  so it lags, overshoots a hard stop and turns back; wanders ±6 m over a slow target) and
  sends `ToClient::Heli` 15/s to everyone in the round, wherever they are
  (`Connection::hunt_heli`; drawn only in a running round).
- Shots: the rescue runs on the game's own server, on real time (not `STEP`), so tap R
  at a frame and shoot soon after: `TAPS="KeyR@30" HELICAM="-40,12,-30" bash
  tools/dev/shot.sh out.png 44` (the car carried over the houses; the server has it from
  frame ~30 to ~54 at lavapipe's pace). `TOPPLE="side,frame"` (the car onto its roof off
  the road) is a placement the server refuses now, and `OFFLINE=1` has no rescue.
