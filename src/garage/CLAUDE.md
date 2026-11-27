# Notes: Shops

Part of the repo's notes for Claude Code (conventions in `/repo/CLAUDE.md`). Loaded when working in `src/garage/`; keep it up to date.

## Shops (agreed with the user, 2026-11-21; built 2026-11-22)

The tool windows (3–6) became places on the map to drive to; the keys 3–6 are gone.
`src/shops.rs`. **The server says where they are**: its own list, separate from the map
(`server --shops FILE`, `services.car-sim.shops = FILE`; none without one, at the
user's wish, 2026-11-22; the repo's list is `assets/shops.cfg` (tests read it): lines
`kind x z name`, the building's map point; one of each in Baunach and Rattelsdorf, and
shared by Gerach and Reckendorf, 1.6 km apart; names made up, never a real business or brand, at the user's wish), sent as `ToClient::Shops` after the
welcome (protocol 6). Alone they're the repo's lists (the game's own server). The client puts
each marker (glowing disc, 3.5 m, and a beam) on the nearest paved road at the edge on
the building's side (`kerb`); names float above within 500 m. Free (no money). The plan
as built, in order:

1. ✓ server list, protocol, markers and names. 2. ✓ Stop on a marker to go in (window opens,
   parking brake; drive off or Esc leaves). 3. ✓ **Ghosts**: a car stopped on a marker,
   in a shop or in the dyno is a ghost (a flag in its state): no collisions with players'
   cars or traffic (the server's traffic ignores it), drawn half transparent; it stays a
   ghost after leaving until it overlaps nobody. So several players use a shop at once.
   (`CarState::ghost`; `shops` sets `net::WantsGhost`, `client::haunt` puts `looks::Ghost`
   on our car, `move_cars` on remote ones; `push_off_remote_cars` skips ghosts, the server
   decides ghosts itself since 2026-11-24 (`server::haunt`, see "Server authority") and
   removes a ghost's `traffic::Visitor`; `visuals` swaps in a second, blended material set.)
4. ✓ Shops on the map (M), with a legend (`map::MapLegend`); their names only when zoomed in (under 1.6 m per screen point, `MapMark::label_below`; at the user's wish, 2026-11-22). 5. ✓ **Gas station**: 95/98/102; the tank keeps
   its octane, mixing by volume; the engine's knock limit reads the tank's octane (the
   panel's octane and fuel load sliders go). Since 2026-11-22 (asked for by the user) a
   scene like the alignment bay's (`gas.rs`): "Up to the pump" puts the car (drawn from
   `Looks`, engine off) at a pump on a forecourt 600 m **over** the marker (open air: a
   2.4 km meadow hides the land below; canopy, three islands, the shop, trees), the
   filler flap open on the right over the rear wheel (front wheel when the tank is up
   front), nozzle and a hanging hose from the pump; the pump's display (egui-painted in
   perspective, point by point) shows grade, litres of this fill and the tank; the grade's
   button lights and its nozzle leaves the holster. The window on the right (simplified
   at the user's wish, 2026-11-23): the grade, then "Pump" (at `FLOW`, 1 L/s, until
   stopped or full; while it pumps the focus is locked on it, now "Stop", and back does
   nothing), what's in the tank. Shots: `TAPS="Enter@200"` is "Up to the
   pump", then "Pump" is the pump window's second row (`ArrowDown@236;Enter@240`);
   `FUEL=12` empties the tank to 12 L at frame 160. 6. ✓ **Alignment shop**: camber, pressures
   (from the garage), toe and brake torque/balance (from the old panel; into `CarBuild`).
   Since 2026-11-22 (asked for by the user) a scene like the dyno's (`alignment.rs`): the
   shop's window has "Onto the alignment lift", a bay 220 m under the marker with a
   four-post lift (turn plates, slip plates, ramps), the car on it drawn from the driven
   car's `Looks` (steer and spin zeroed), a head clamped on each wheel (the clamp tilts
   with the wheel's pose, the head hangs level), red beams along the sides and across
   under the bumpers, and the console's screen (egui-painted like the dyno's) with each
   wheel's camber (measured from the pose, so the car's weight shows), toe and pressure
   against the spec (kit camber ±0.5°, `FACTORY_TOE` ±0.1°, suited pressure ±0.15 bar),
   green or red. The window on the right is `garage::alignment`, applied live to the
   parked car (`car::Away` set, as in the dyno); drag swings, the wheel zooms. Shots: `TAPS="Enter@200"`
   onto the lift; the rows are camber, toe, pressure per axle.
7. ✓ **Paint shop**: the shop paints (swatches; gone from the garage). Every chassis
   leaves the factory in one dull paint, no choice (asked for by the user, 2026-11-22, so
   there's a reason to paint): `PAINTS` is the 10 shop paints (`SHOP_PAINTS`), then 11
   factory ones (beige, putty, primer grey…; the paint shop never offers those); traffic
   is in a factory paint (any, since there's one chassis), a `PAINTED` fifth in a shop paint (so a painted car can still
   hide in it). Protocol 15. Since
   2026-11-22 (asked for by the user) a scene like the alignment bay's (`paint.rs`): "Into
   the paint booth" puts the car (drawn from `Looks`, engine off) in a spray booth 240 m
   under the marker, sized to the car (white walls with seams, glowing filter ceiling and
   angled light rows, grates over pits along both sides, folding glass doors ahead, a
   yellow filter bank at the back, control panel and door on the left, a shelf of tins
   (lids in each paint, the picked one pulled out) and the gun on its hook on the right, a
   test card on a stand by the right front wheel in the picked paint). The window on the
   right: "Colour" (a row opening a picker screen of swatches: the arrows pick, the booth
   showing it, choose keeps, back goes back to the one before; at the user's wish,
   2026-11-23: in the list, up from the swatches always picked another colour first),
   "Spray it X" sprays a coat (`SPRAY` 6 s): a mist of the paint (12
   nested unlit ellipsoid shells round the body, `cull_mode` none, plus 600 droplets going
   round and sinking) thickens, the paint goes onto the parked car at `THICKEST` (3 s,
   `shops::rebuild`), then it clears. Shots: `TAPS="Enter@200"` into the booth; "Colour" is the first
   row (`Enter` opens the picker), "Spray it" the second; use `STEP=0.05`
   (60 frames to the thickest). 8. ✓ **Showroom**: since 2026-11-22 (asked for by the user) a scene like the gas
   station's (`garage/showroom.rs`): "Into the showroom" puts you in a hall with glass
   front and left side, 600 m **over** the marker (meadow, forecourt with flags and a
   pylon, the driven car drawn from `Looks` waiting outside the glass). Every chassis
   stands on the floor in its factory paint at its own `place` (3 left, 7 back, 3 right;
   `traffic::FactoryLooks`); the one picked turns on a turntable under spotlights (its
   floor place hidden), a sign beside it egui-painted with name, class and engine. "Start
   it" idles the picked engine (the model's `Motion::engine`, heard by `hear_others`) and
   blips it every 3.5 s. The window on the right: the list and `brochure`; "Drive it"
   builds the car and goes straight out to it (once built), in neutral with the parking
   brake on (`fit` leaves every car put down afresh so). The camera stays within `REACH` (7.4 m) of the
   turntable, clear of the floor cars. Shots: `TAPS="Enter@200"` in, then the first row is the car
   (`ArrowRight` the next), then the engine, "Drive it", "Start it".
   (`shops::shop_window` draws a shop's window for other modules, saying when the car came in).
9. ✓ **Parts shop**, since 2026-11-26 **car workshops, tuning shops and tire shops**
   (see "Owning cars and parts", milestone 6: the fitting window). Since 2026-11-22
   (asked for by the user, "car on a lift") a scene
   like the alignment bay's (`garage/bay.rs`): "Onto the lift" puts the car (drawn from
   `Looks`, engine off, wheels hanging at `DROOP`) up on a blue two-post lift (`RAISE`
   1.3 m) in a workshop 260 m under the marker; the arms' pads go under the sills behind
   the front wheels and ahead of the rear ones, made afresh for another chassis. An engine
   like the car's stands on an engine stand in front of the car (family size and layout:
   inline, V or flat; a turbo if it has one), tires like its in a rack at the back;
   workbench with pegboard, tool chest, shelves of boxes, roller door, oil drain. The
   fitting window (`garage::fit`) is on the right. Shots: `TAPS="Enter@200"` onto the lift;
   the rows are the car's slots by group (Engine first). Back steps out of a screen, on
   the row gone into it from (`Fitter::came_from`, as in the central menu; a mouse click
   counts as that row), and from the first screen back to the car. 10. ✓ **Dyno** (`dyno/`): the career's chassis dyno room back
   (`mod.rs`, `rollers.rs`, `chart.rs`). A copy of the car runs in the room 200 m under
   the marker; the driven car stays parked there, a ghost (`car::Away` holds its input and
   R, `EngineAudio::hushed`); the engine (tune, cam timing, wastegate, wear) and the fuel
   go back to it on leaving. The window (up all the time) has tabs: Runs (kept for the
   session only), ECU tune (`ecu_ui::tuning`), Cams and boost (applied to the copy,
   baking maps if needed, tune and wear kept). Hold: up through the gears, then the
   rollers are motored to the speed for an rpm in fourth (`HOLD_RATE`) at a set throttle,
   power = what the wheels push with. Runs can't be skipped (at the user's wish,
   2026-11-23). The engine dyno window (5) is gone. 11. ✓ Clean-up:
   the panel lost its setup section (springs, dampers, upper pivot, tire μ: they follow
   from the parts); **R only puts the car back on the road** (`Car::place` keeps fuel,
   tires, engine wear, and leaves it in neutral with the parking brake on, so a teleport
   (rescue, a round starting or over, joining) never stalls it; agreed: no hint suggests anything for an empty tank or a wrecked
   engine, for now); `ui::tool_window` is gone; help, README.

No scene's window has a "Back to the car" row (removed at the user's wish, 2026-11-27):
back (Esc, a pad's or wheel's back button) leaves, and the prompt line says so.

Not done yet: dyno runs aren't saved; the HUD shows the parked car's gauges while in the
dyno's room.
