# The Vireo's engines, cylinder heads and ECUs (planned with the user, 2026-11-27)

The Vireo's lineup changes to three engines inspired by Honda's B16 and B20: a 1.6 and a
2.0 with one cam over two valves per cylinder, and a hot 1.6 with two cams over four
valves, switching cams and a head that flows very well high up. Its head on the 2.0
block should be the swap players want (a "Frankenstein" engine).

## Steps

1. **Cylinder heads are parts** (done): `Family::heads` (`HeadDef`), `Kind::Head` in the
   long block, holding porting, camshafts, valve springs and cam phasers. A head keeps
   its valve sizes on another bore. Saved cars from before read into the factory head.
2. **Maps per cam profile** (done): the maps by load (MBT, efficiency, knock limit,
   exhaust temperature, heat to coolant, peak pressure) are baked for each profile, and
   an ECU that switches cams has fuel, λ and spark maps of its own for the second.
3. **ECUs from the catalogue** (done): `EcuModel` becomes data (`Catalogue::ecus`: most
   cylinders, cam switching, phasers, wideband, locked, and for a stock ECU the version
   whose factory maps it carries).

   | Item | Hardware | Maps | Tunable |
   |---|---|---|---|
   | B16 ECU | B-series stock: 4 cylinders, no cam control, narrowband | B16 factory | locked |
   | B20 ECU | the same | B20 factory | locked |
   | B16R ECU | 4 cylinders, cam switching, narrowband | B16R factory (both cams) | locked |
   | R80 ECU | 10 cylinders, narrowband | R80 factory | locked |
   | Programmable ECU (4 cylinders) | up to 4, cam switching and phasers, narrowband or wideband | yours | yes |
   | Programmable ECU (10 cylinders) | up to 10 (set in the tune), no cam control, narrowband or wideband | yours | yes |

   A stock ECU always runs its version's factory maps, whatever engine it's on. Locked:
   the dyno shows the readings but not the editor, and the server refuses a tune.
   Fitting refuses an ECU that can't drive the engine's cylinders. Old saved
   `ecu.stock` becomes the engine version's stock ECU, `ecu.programmable` the
   programmable one for its cylinders.
4. **The lineup** (done): B16 (1.6, SOHC, 8 valves), B20 (2.0, SOHC, 8 valves; replaces the
   B18), B16R (1.6, DOHC, 16 valves, switching cams from the factory, about 8200 rpm).
   Trims `vireo.1-6`, `vireo.2-0` (the 1.8's equipment), `vireo.1-6r` (new), each with
   its stock ECU. The B20 block with the B16R head: more power than either, but the
   B20's rods (made for its long stroke and 6000 rpm) cap the revs until forged rods go
   in, and it needs an ECU that switches cams and fuels a 2.0. Old saves: `b18` →
   `b20`, `vireo.1-8` → `vireo.2-0`.
5. **Lambda sensors and closed loop** (done): a required engine slot "Lambda sensor"
   (narrowband stock, wideband at tuning shops). Narrowband: a steep switch at λ = 1
   with a short lag; closed loop only where the target is λ = 1 (not on the overrun cut
   or limiter), a short-term trim (a jump at each switch and a ramp, ±25 %), so it hunts
   around stoichiometric; open loop elsewhere. Wideband (programmable ECUs only): closed
   loop to the target everywhere, with a settable authority; a stock ECU reads it as a
   narrowband. The trim is physics state (`snapshot!`).
6. **Shops, looks and notes** (done; programmable ECUs at the tuning shops): heads, long blocks and programmable ECUs at the engine
   shop, programmable ECUs and the wideband sensor at tuning shops, stock ECUs at car
   workshops; cam covers by head (one narrow cover for one cam, a wide one for two).

## Assumptions (stated to the user, not yet answered)

- Codes B16, B20, B16R; the start car stays the base 1.6 (now the SOHC, slower).
- Swapped heads keep their absolute sizes; compression stays the block's; any B-series
  head fits any B-series block.
- Locked stock ECUs mean forced induction or a swap needs a programmable ECU (before, the
  dyno could give a stock ECU a factory tune for anything).
- The dyno has its own exhaust gas analyser: the tuning always shows the true λ; on the
  road the telemetry (2) shows what the sensor reads (narrowband volts, wideband λ).
- No long-term fuel trims, no sensor warm-up or cold-start enrichment for now.
- Physics tests that build an engine from bare hardware keep a factory ECU calibrated for
  that hardware.
- Saved cars are converted, not dropped.
