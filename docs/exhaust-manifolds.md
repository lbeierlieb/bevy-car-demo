# Exhaust manifolds and turbo mounts: the plan

Asked for by the user on 2026-11-27: the exhaust was too simple. Any engine had one
"Exhaust manifold" slot and, beside it, one "Turbo or supercharger" slot, independent of
each other, so long-tube headers took a turbo, and a V10 had one header and one turbo.
Wanted: V10 long-tube headers (one per bank, ending at the cat), turbo manifolds per bank
with a turbo on each, and one big manifold joining all ten into a single turbo; on an
inline six, NA long tubes, a single-turbo manifold and a twin-turbo manifold (3 + 3).
Sides may be required to match. The milestones go in order; each ends with clippy
(native and wasm) and tests passing, rustfmt clean, the game playable, and committed
atomically (see CLAUDE.md, "Working conventions").

The user's answers (2026-11-27): no inline-six test engine (the layout must allow
one, but nothing is built for it until a real I6 family comes); the plan as below.

## What the model assumed before

- `EngineHardware::exhaust.header_length` is one primary length (the cycle simulation's
  header pipe, reflecting inverted off a collector at constant pressure, `cycle.rs`
  `HEADER_REFLECTION`), and `turbo: Option<Turbo>` one turbo taking all the exhaust:
  one turbine and wastegate in `exhaust_pressure`, one shaft (`Engine::turbo_speed`,
  `step_turbo`), one compressor feeding the tract (`tract_inflow`).
- Header parts are generic multipliers on the engine's header length (`catalogue/parts.rs`:
  stock, short ×0.56, long-tube ×1.375) and fit every engine; turbo kits
  (`parts::TurboKit::fit`) are scaled by the whole engine's displacement.
- Item tree: the engine item's slots are the long block, then `ENGINE_SLOTS` (intake
  manifold, exhaust manifold, air filter, throttle, turbo or supercharger, flywheel),
  flat. The intercooler, catalyst, silencer and exhaust pipe are the car's
  (`CAR_SLOTS`).
- The chassis' `header_room` caps the primaries' length (`Parts::header_room`).

## The idea

**The exhaust manifold decides the layout; turbos sit in mounts on it.** A manifold
without mounts (long tubes, a stock NA manifold) can't carry a turbo.

```
Engine
├ Long block …
├ Intake manifold, air filter, throttle, flywheel
├ Supercharger            (what's left of "Turbo or supercharger")
└ Exhaust manifold        (made for one engine family)
   ├ Turbo, left bank     (only on turbo manifolds; as many mounts as branches)
   └ Turbo, right bank
```

A manifold definition: primary length (m, absolute, for its family), collector kind
(`Tuned`: a collector into the pipe; `Turbine`: a turbine inlet), branches (groups of
cylinders, each to one collector or turbo: all the same size), and whether each branch
has a turbo mount. Examples:

| Engine | Manifold | Branches → mounts |
|---|---|---|
| R80 V10 | Stock cast manifolds (pair) | 2 × 5 → 0 |
| R80 V10 | Long-tube headers (pair, ending at the cat) | 2 × 5 → 0 |
| R80 V10 | Turbo manifolds (pair) | 2 × 5 → 2 |
| R80 V10 | 10-into-1 turbo manifold (crossover) | 1 × 10 → 1 |
| B16/B18 | Stock cast 4-into-1, short 4-1, long-tube 4-2-1 | 1 × 4 → 0 |
| B16/B18 | Turbo manifold | 1 × 4 → 1 |
| (a future I6) | long-tube 6-into-1 / single turbo / twin 3 + 3 | 1×6→0, 1×6→1, 2×3→2 |

Rules (`Item::fine` and `fitting`): a manifold fits only its engine family; a turbo
manifold's mounts are all empty or all hold the **same turbo definition** with the same
knob values (wastegate); turbos and a supercharger exclude each other (no twin-charging
for now); a shop fitting a turbo fills every mount at once (like "both on the axle").
An empty-mount turbo manifold runs as an NA engine with that manifold.

## Different turbos per side

Not built, but the model allows it later: two turbo states sharing the boost pipe (the
intake solve stays its 2×2 implicit step; turbo speeds are explicit), each bank with its
own exhaust pressure, torque the mean of two map lookups (about twice the lookup cost).
The hard part is two unequal compressors on one pipe (the stronger stalls the weaker:
surge and backflow, which the compressor model doesn't cover), and every display and the
ECU's view. With matching sides, N turbos are exactly one turbo state with the flow split
N ways, at no extra cost: that's what's built.

## Assumptions

1. A V engine's pair of headers (or manifolds) is **one item**, sold as a set; matching
   sides come for free. The 10-into-1 is one item too.
2. Turbos stay relative sizes (small, medium, large), sized for the displacement behind
   their mount (a "Medium" on a V10 bank is sized for 4 L).
3. Everything after the manifold stays the car's (pipe, cat, silencer, intercooler), as
   agreed before: twin turbos share one intercooler; a pair's outlets go into whatever
   exhaust the chassis has. No dual-exhaust parts.
4. The cycle simulation stays one cylinder: grouping changes the collector and the
   turbine's flow, not interference between cylinders sharing a collector.
5. `header_room` still caps the primaries' length (long tubes are what it shortens); no
   box check for turbos in the bay yet.
6. Saved cars are converted (format 4), not dropped.
7. Turbo wear is per turbo item, all mounts strained alike; any one failing kills the
   boost (an approximation).

## Milestone 1: data and catalogue (no physics change)

1. `items::Kind::Manifold` and `Kind::Turbo`; a family restriction on `PartDef` (like
   `fits` for chassis, by family or version code), checked in `Item::fine` and by
   `fitting`. The engine item's slots: long block, intake manifold, exhaust manifold
   (`Kind::Manifold`, required), air filter, throttle, supercharger (the old turbo slot,
   superchargers and "None"), flywheel.
2. Manifold definitions per family in the catalogue (`catalogue/engines.rs` data on the
   family, generated into defs in `catalogue/items.rs`): ids like `b.manifold.stock`,
   `b.headers.long-tube`, `b.manifold.turbo`, `r.manifolds.stock`, `r.headers.long-tube`,
   `r.manifolds.turbo`, `r.manifold.turbo-crossover`. Each a `Does::Manifold` with the
   layout above; turbo mounts are `SlotDef`s of `Kind::Turbo`, not required. Primary
   lengths match today's results (B series stock = today's stock header length after the
   family's preset; short/long as today's multipliers; R80 likewise).
3. Turbo definitions (`Kind::Turbo`, the old `turbocharger.small|medium|large` ids kept,
   wear `Turbo`) go only into mounts.
4. The bridge: `Item::resolve` gives `Parts` the manifold (which one, with its layout) and
   the turbo (which, how many: the mount count when filled); `items::assemble` puts them
   back. Rules: all mounts the same turbo and knobs, no turbo with a supercharger.
5. Trims: the B16/B18/R80 trims get their stock manifolds. Shops file: car shops the
   stock manifolds, tuning shops headers, turbo manifolds and turbos; the engine shop as
   before. `what_the_shops_sell_is_in_the_catalogue` passes.
6. Saved cars, format 4 (`saved.rs`): an old exhaust manifold part becomes the family's
   matching one; an old turbo in the old slot moves onto the family's turbo manifold
   (replacing the header); old supercharger ids stay in the supercharger slot. Tested.
7. Until milestone 2 the hardware keeps working as now: the manifold sets
   `header_length`, a filled mount set gives `turbo` (sized for the whole engine as
   before, so nothing measured changes yet; all baked maps unchanged).
8. Protocol bump (catalogue, items on the wire). Fitting rules tested in
   `fitting::tests` (long tubes have no mount, a turbo fills every mount, mixed turbos
   refused, turbo + supercharger refused, a B-series manifold refused on the R80).

## Milestone 2: physics

1. `hardware::Exhaust`: `header_length` becomes `manifold: Manifold { primary_length,
   collector: Collector, branches: u32 }`; `EngineHardware::turbos: u32` (0 without a
   turbo; the `Turbo` describes one of them). Maps' fingerprint follows.
2. Runtime (`engine/mod.rs`): each turbine and wastegate passes flow / turbos
   (`exhaust_pressure`, `step_turbo`), the compressor flow × turbos goes into the tract
   (`tract_inflow`); one shaft state. Telemetry, damage strain unchanged per turbo.
3. Turbo kits sized for the displacement behind a mount (`TurboKit::fit` with the
   branch's share; a manifold with one branch: the whole engine as now, so the B series'
   turbo results stay).
4. `cycle.rs`: the collector kind sets the header's reflection (`Tuned` as now; a
   turbine inlet reflects much less, so less scavenging). Re-bake every stock engine
   (`bake_stock_maps`).
5. `Ecu::calibrated` and factory tunes work for twin turbos (the turbine-inlet margin
   per turbo).
6. Tests: long tubes beat a turbo manifold with empty mounts on the R80 low down or
   through the band as the model says; on the R80 twin turbos (pair) spool earlier than
   the 10-into-1 with the same turbo kit (smaller per turbo), and the single big turbo
   holds boost high up; the B-series turbo results unchanged within tolerance;
   `every_factory_engine_idles_and_pulls` passes.

## Milestone 3: the game

1. `garage/fit.rs`: the manifold's screen with its mounts; fitting a turbo fills all
   mounts; readouts (primary length, branches, mounts).
2. `mechanicals.rs`: headers per bank as the manifold says (long tubes, cast manifolds,
   turbo manifolds), a turbo at each collector, the crossover pipe for the 10-into-1.
3. `Looks`/sound: the turbo count (`ExhaustSound`, the whistle per turbo) — protocol
   bump; telemetry (2) shows boost and turbo speed (and the count).
4. Notes (CLAUDE.md) and this plan as built.

## Later (not in this plan)

Different turbos per side; sequential turbos, twin-scroll; pulse interference by firing
order within a collector (the odd-fire V10, 3-cylinder groups); an inline six family;
dual-exhaust parts; turbos checked against the bay's room.
