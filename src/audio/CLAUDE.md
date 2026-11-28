# Notes: Audio

Part of the repo's notes for Claude Code (conventions in `/repo/CLAUDE.md`). Loaded when working in `src/audio/`; keep it up to date.

## Audio

The engine's layout (`families::Layout`) sets how many pulses there are in a cycle and
which bank's exhaust each goes down (`audio.rs`, `sound`): a cross-plane V8's banks fire
unevenly, a flat four's pulses bunch up through unequal headers, an odd-fire V10
(`Layout::OddFireV10`, the R-series: common crank pins, 2026-11-27) fires 54° and 90°
apart in turn (`Sound::late`, each firing's lateness in firing intervals: its banks in
turn, each even at 144°, out of step), the rest fire evenly; inline engines have one
exhaust (no panning). The R80 rendered at 2500 rpm: the mono sum's order 2.5 is 14 dB
over order 5 (rendered as an even V10, 5 dB under), order 7.5 +5.5 dB (−14); peak 0.93
outside, 0.98 inside, nothing over 0.99; never listened to.

The exhaust hardware colours it (`ExhaustSound::of`, parts compared with the stock ones
for the engine's size): silencer and cat muffle (level, upper resonances; straight pipe
loud and rasping), bore (deeper, louder), header length (a quarter-wave resonance), a
turbine (smeared, quieter pulses). The turbo whistles: band-passed noise (Q 4) centred at 1 Hz per shaft rev/s,
level ∝ share of its limit², more under load. All constants at the top of `audio.rs`,
none tuned by ear yet. No diverter "pssh" or overrun pops (fuel cut leaves nothing to burn).

Other cars are heard too (asked for by the user, 2026-11-21; `hear_others`): any car
with `Looks` and `Motion` that isn't a `Car` (remote cars), the nearest 6
within 150 m (heard from 135 m, so one at the edge doesn't flicker), each an
`AudioPlayer` of its own on a `Voice` entity (fading out over 0.2 s when dropped). The
same synth, placed by `Targets::place`: mixed to one source, panned from the camera
(constant power), as loud as our own at 6 m and falling with 1/distance (faded out over
the last fifth), a low-pass from 10 kHz at 6 m falling with √distance, and Doppler (the
whole engine's time scaled by 343/(343 − closing speed), the closing speed from the
distance's change, smoothed 0.1 s, capped at 100 m/s). Runs in PostUpdate after
transform propagation (the shot cameras move in PostUpdate). The engine comes from the
car's latest state, network delay and all, never predicted (agreed with the user: real
sound is late too). Not done: propagation delay, occlusion, the cost of 6 voices on wasm
untried (a slow page could lower `VOICES`).

Sound effects (asked for by the user, 2026-11-22; `audio/effects.rs`, a second synth
next to the engine's, with the same sound setting): **tire squeal** of the driven car per wheel (band
of noise at ~780 Hz with two overtones, wobble and flutter, pitch rising past the peak),
from the combined slip relative to the peak (from 0.55, full at 1.4), the tread's sliding
speed, the load, asphalt (cobbles half, unpaved none); **gear grinding** while an
H-shifter's lever is in a gate whose gear can't go in (`Drivetrain::grinding`: clutch
not pressed, reverse while rolling) and 0.3 s on a refused reverse by key (clicks at the
dog teeth's clash rate from the speed difference ringing metallic resonances); the
**trumpet** of a race's countdown (`racing::countdown`: G4 at 3, 2, 1, a C major chord
at the green; `Effects::call`); **crashes** from `Car::knocks` (the body's speed change
from houses, trees, fences, cars and, since this, the ground; taken every frame by
`effects::cue`): 1 m/s in a frame is a crash, 15 the hardest (a gliding thud, a crunch of
crackles ringing panel resonances, glass pings from 0.45), knocks kept up while moving
scrape. the **starter** while cranking (2026-11-22: the motor geared 12:1 to the crank;
partials at the pinion's 11 teeth and twice that, the commutator's 22 segments and the
motor's hum, brush noise; each compression dips its speed and swells it; the solenoid
clunks in and the pinion clacks out; let go or overrun, it spins down over ~0.5 s).
`write_effects_wav` (ignored, `EFFECTS_WAV=path`) renders
them all to listen to. Not done: other cars' squeal and crashes (`Motion` has no slip), tuned by ear.

Listening to the engines offline (2026-11-24, for reworking the engine sound):
`ENGINE_WAV=dir ENGINES=M50,B16 cargo test --profile release-dev write_engine_wavs --
--ignored --nocapture` renders each factory engine (all without `ENGINES`) to
`dir/<code>.wav` (23 s, ~2 s for all 22): idle, three blips, a full-throttle pull from
2000 rpm to the redline, lifting off, a cruise, then a pass-by at 90 km/h from 8 m
(the remote-car path: mono, low-pass, pan, Doppler). Raw levels, the same program
every time, so renders before and after a change can be compared. First render: nothing
clips, and the V8s (RMS 0.155) are quieter than the 1-litre three (0.19): nothing in
the level grows with displacement. Since then (with the user listening): the pulse
lasts a crank angle (20° up, 70° down), each cylinder has a fixed strength and lateness
(`CYLINDERS`: the lope), and the pulse runs through three pipes as delay lines (`Tube`:
header, front pipe, tailpipe; inverted reflections from the muffling, losses above
`PIPE_LOSS`) before the old broad resonances, so harmonics swell and fade 4–7 dB over a
pull (was ~2). Upper harmonics: the pulse's front sharpens with load (20° to 4°, starting
with a kink), and what's above 1 kHz bypasses the broad resonances (`BRIGHT`, set by the
muffling; the stock exhaust counts as ~0.89 muffled): the V8 falls ~4–6 dB per octave of
orders at 5000 rpm (was ~10). Tried and dropped: steepening in the header as a
pressure-dependent delay (skipped samples lost the pulse's peak: −2–3 dB in the pull). Tire squeal at half its level, swelling over 0.25 s (scuff first, the tone over it).
**Inside and outside** (`audio/paths.rs`; the cabin's nearness to the engine from the
car's bay, `CabinShape::of_bay`, so an engine subframe's engine behind the seats comes
through brighter): outside, the road's reflection of the exhaust
(`ground_delay` from the car's and the camera's places: ~0.6 ms behind our car, sweeping as
a car passes), scaled so the lows keep their level; from the driver's view (`ChaseCamera::
driver`) every car sound but the race's trumpet goes through `Cabin` (body transmission
two-pole at 450 Hz plus a 15 % leak, three cabin modes 72/118/155 Hz, four early
reflections per ear, narrow stereo), crossfading over 0.3 s on an S-curve (our road reflection keeps its delay inside: dropping it clicked; the render writes `<code>-switch.wav`). Each car its own cabin
(`CabinShape::of`, asked for by the user, 2026-11-24): modes from the cabin outline's run,
the body's width and the roof over the floor; how much comes through and how bright from
`Chassis::insulation` (a constant per body, 0 stripped or open to 1 the saloon) and the
engine's distance from the eyes (the mid-engined cars and the V12 supercar brightest);
open cars (cabin outline of two points or none) get a third of the cabin. One
`paths::Listener` (inside or not, the cabin) shared by every synth (`Listening`
resource). Synthetic (no recorded IR).
**Loudness from the gas** (branch `engine-gas-flow`, protocol 17, tried by the user before
merging, 2026-11-24): each pulse's strength is the gas a firing pushes out, displacement
per cylinder (`EngineCharacter::displacement`, in `Looks`) × how full it got
(`Drivetrain::fill`, air per cycle over the displacement, above 1 on boost; sent as
`EngineRun::fill` in `load`'s old slot, so no extra bytes per snapshot; the 0–1 tone load
is `drivetrain::firing_load(fill)` on the receiver; traffic and the showroom send
`fill_for(load)`). The factor making fewer cylinders louder is gone; `CYLINDER` (0.5 L at
full fill = strength 1) leaves the big engines room under the limiter. A-weighted at full
throttle: three −27.5 dB, 1.6 four −24.2, V8 −22.6/−21.3, V10/V12 −20 (the V12 was 2.4 dB
under the 1.6 before); the whole game ~4 dB quieter. The render writes `<code>-inside.wav`
too. Idle is ~4 dB louder inside (the boom), a V12's pull 5 dB quieter (its energy is
where the body blocks). Measured from the renders with numpy (order levels over the pull, envelope
modulation at half orders); the user judges by ear. Not done: displacement in the level
(needs it in `EngineCharacter`: protocol bump), the road reflection outside, the cabin
inside, the whistle (now ~34 dB under the exhaust at 4000 rpm) never tuned.

The engine sound goes through bevy_audio, which can't set the output buffer size. An
attempt to open the output with rodio/cpal directly (fixed 4096-frame buffer against
underruns) gave no sound at all in the wasm build and was dropped. If it's tried
again, keep the bevy_audio path on wasm.

Web: the browser creates the `AudioContext` suspended unless the page already had a
click or key press, and cpal resumes it only once, at startup, so the game was silent
whenever loading finished before the first input (found 2026-11-19). `web/index.html`
resumes every context on the first pointer/key/touch event. Checked in headless
Chromium (from the pinned nixpkgs, `--use-angle=swiftshader --enable-unsafe-swiftshader`)
with a script that hooks `AudioContext` and `AudioBuffer.copyToChannel` (cpal copies
every buffer through it) and reports the state and the RMS to the server, and clicks
sent through the DevTools protocol (`Input.dispatchMouseEvent`).
