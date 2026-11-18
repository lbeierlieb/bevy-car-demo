//! What things cost: cars new and used, parts at the parts shop, repairs.

use super::{Item, Wear};
use crate::car::CarSetup;
use crate::physics::engine::damage::Damage;
use crate::physics::engine::ecu::EcuModel;
use crate::physics::engine::parts::SLOTS;
use crate::wheels::TireSize;

/// Each chassis new with its base engine (€, in `CHASSIS` order).
const CHASSIS_PRICES: [f64; 13] = [
    9_000.0, 21_000.0, 26_000.0, 30_000.0, 28_000.0, 42_000.0, 38_000.0, 65_000.0, 78_000.0,
    190_000.0, 52_000.0, 24_000.0, 22_000.0,
];
/// Each engine option past the base one adds this share to a car's price.
const OPTION_PRICE: f64 = 0.2;
/// A used car sells for this share of its price new, before its wear.
const RESALE: f64 = 0.45;

/// A pair of rims (€ per inch of diameter times inch of width).
const RIMS: f64 = 2.4;
/// A pair of tires (€ per mm of width times inch of rim diameter), by compound.
const TIRES: [f64; 4] = [0.04, 0.05, 0.1, 0.17];
pub const CLUTCHES: [f64; 3] = [180.0, 450.0, 900.0];
pub const SUSPENSIONS: [f64; 3] = [400.0, 950.0, 1_800.0];
pub const GEARBOXES: [f64; 8] = [
    900.0, 1_600.0, 1_000.0, 1_300.0, 2_600.0, 1_500.0, 2_200.0, 3_900.0,
];
pub const LIMITED_SLIP: f64 = 750.0;
const ECUS: [f64; 2] = [300.0, 1_400.0];
/// Engine parts by slot (in `SLOTS` order) and part; nothing for what isn't sold ("None").
const ENGINE_PARTS: [&[f64]; SLOTS.len()] = [
    &[180.0, 450.0, 550.0, 1_200.0],
    &[350.0, 1_100.0, 1_300.0],
    &[220.0, 600.0, 700.0],
    &[200.0, 900.0, 1_600.0],
    &[90.0, 260.0],
    &[220.0, 520.0, 900.0, 1_500.0],
    &[110.0, 300.0, 650.0],
    &[300.0, 900.0, 1_500.0],
    &[250.0, 700.0, 700.0],
    &[200.0, 500.0, 850.0],
    &[30.0, 80.0, 120.0],
    &[150.0, 350.0, 550.0],
    &[0.0, 1_800.0, 2_400.0, 3_200.0],
    &[300.0, 900.0, 0.0],
    &[450.0, 650.0, 0.0],
    &[250.0, 400.0, 0.0],
    &[200.0, 450.0],
    &[200.0, 500.0],
    &[250.0, 450.0],
    &[500.0, 1_400.0, 2_600.0],
];
/// Repairs (€): each cylinder's piston, valves and rod, then the head gasket, bearings and
/// turbo, at the share they're worn; and the labour for any repair.
const CYLINDER_REPAIR: [f64; 3] = [180.0, 140.0, 120.0];
const GASKET_REPAIR: f64 = 350.0;
const BEARING_REPAIR: f64 = 900.0;
const TURBO_REPAIR: f64 = 1_200.0;
const LABOUR: f64 = 80.0;

/// A chassis new from the factory with its engine `option`.
pub fn new_price(chassis: usize, option: usize) -> f64 {
    CHASSIS_PRICES[chassis] * (1.0 + OPTION_PRICE * option as f64)
}

/// What a car is worth: its price new less its age (`age` from 0, new, to 1, old), its
/// engine's wear and its tires'.
pub fn worth(setup: &CarSetup, wear: &Wear, age: f64) -> f64 {
    let build = &setup.build;
    let option = (build.chassis().engines.iter())
        .position(|code| setup.engine.parts.is_factory() && *code == factory_code(setup))
        .unwrap_or(0);
    let new = new_price(build.chassis, option);
    let cylinders = setup.engine.build.hardware.cylinders as usize;
    let repairs = repair(&wear.damage, cylinders) + tires_worn(setup, wear);
    (new * RESALE * (1.0 - 0.7 * age) - repairs).max(0.08 * new)
}

/// The factory code of a car's engine version (e.g. "W20T").
fn factory_code(setup: &CarSetup) -> &'static str {
    let parts = &setup.engine.parts;
    parts.family().versions[parts.version].code
}

/// What putting new tires of the same kind on a car costs, by how worn they are.
pub fn tires_worn(setup: &CarSetup, wear: &Wear) -> f64 {
    (0..2)
        .map(|axle| {
            let worn = (wear.treads[2 * axle].wear + wear.treads[2 * axle + 1].wear) / 2.0;
            new_tires(setup, axle) * f64::from(worn)
        })
        .sum()
}

/// What a new pair of the tires on an axle (0 front, 1 rear) of a car costs.
pub fn new_tires(setup: &CarSetup, axle: usize) -> f64 {
    let wheel = setup.build.wheels[axle];
    tire_price(&wheel.tire, wheel.compound)
}

fn tire_price(size: &TireSize, compound: usize) -> f64 {
    TIRES[compound] * f64::from(size.width) * f64::from(size.rim)
}

/// What repairing an engine of `cylinders` costs.
pub fn repair(damage: &Damage, cylinders: usize) -> f64 {
    let cylinder: f64 = (damage.cylinders.iter().take(cylinders))
        .map(|c| {
            let worn = [c.piston, c.valves, c.rod].map(|w| f64::from(w.clamp(0.0, 1.0)));
            (0..3).map(|i| worn[i] * CYLINDER_REPAIR[i]).sum::<f64>()
        })
        .sum();
    let engine = f64::from(damage.head_gasket.clamp(0.0, 1.0)) * GASKET_REPAIR
        + f64::from(damage.bearings.clamp(0.0, 1.0)) * BEARING_REPAIR
        + f64::from(damage.turbo.clamp(0.0, 1.0)) * TURBO_REPAIR;
    let parts = cylinder + engine;
    if parts < 1.0 { 0.0 } else { parts + LABOUR }
}

/// What the parts shop sells, by kind, apart from rims and tires, which come in every
/// size on offer (see `wheels`).
pub fn stock() -> Vec<(String, Vec<Item>)> {
    let mut shelves = vec![
        (
            "Clutches".to_string(),
            (0..CLUTCHES.len()).map(Item::Clutch).collect(),
        ),
        (
            "Suspension".to_string(),
            (0..SUSPENSIONS.len()).map(Item::Suspension).collect(),
        ),
        ("Differentials".to_string(), vec![Item::LimitedSlip]),
        (
            "Gearboxes".to_string(),
            (0..GEARBOXES.len()).map(Item::Gearbox).collect(),
        ),
        (
            "ECUs".to_string(),
            EcuModel::ALL.into_iter().map(Item::Ecu).collect(),
        ),
    ];
    for (slot, kind) in SLOTS.iter().enumerate() {
        let items = (0..kind.parts.len())
            .map(|part| Item::Engine { slot, part })
            .filter(|&item| price(item).is_some())
            .collect();
        shelves.push((format!("Engine: {}", kind.name.to_lowercase()), items));
    }
    shelves
}

/// What a part costs at the parts shop, if it's sold at all.
pub fn price(item: Item) -> Option<f64> {
    let price = match item {
        Item::Engine { slot, part } => *ENGINE_PARTS.get(slot)?.get(part)?,
        Item::Ecu(model) => ECUS[EcuModel::ALL.iter().position(|&m| m == model)?],
        Item::Rims(rim) => RIMS * f64::from(rim.diameter) * f64::from(rim.width_inches()),
        Item::Tires(size, compound) => {
            (compound < TIRES.len()).then(|| tire_price(&size, compound))?
        }
        Item::Clutch(i) => *CLUTCHES.get(i)?,
        Item::Suspension(i) => *SUSPENSIONS.get(i)?,
        Item::Gearbox(i) => *GEARBOXES.get(i)?,
        Item::LimitedSlip => LIMITED_SLIP,
    };
    (price > 0.0).then_some(price)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chassis::{self, CHASSIS, GEARBOXES as BOXES};
    use crate::physics::tire::COMPOUNDS;

    #[test]
    fn everything_has_a_price() {
        assert_eq!(CHASSIS_PRICES.len(), CHASSIS.len());
        assert_eq!(TIRES.len(), COMPOUNDS.len());
        // A pair of street tires for the coupé's front costs about what it would.
        let coupe = CarSetup::factory(3, 0);
        let pair = new_tires(&coupe, 0);
        assert!(pair > 150.0 && pair < 250.0, "{pair}");
        assert_eq!(CLUTCHES.len(), chassis::CLUTCHES.len());
        assert_eq!(SUSPENSIONS.len(), chassis::SUSPENSIONS.len());
        assert_eq!(GEARBOXES.len(), BOXES.len());
        assert_eq!(ECUS.len(), EcuModel::ALL.len());
        for (slot, prices) in SLOTS.iter().zip(ENGINE_PARTS) {
            assert_eq!(slot.parts.len(), prices.len(), "{}", slot.name);
            for (part, &price) in slot.parts.iter().zip(prices) {
                // Only taking a part off ("None") is free.
                assert_eq!(price == 0.0, part.name.starts_with("None"), "{}", part.name);
            }
        }
    }

    #[test]
    fn used_cars_are_cheaper_and_wear_costs() {
        let setup = CarSetup::factory(3, 0);
        let new = Wear::new(&setup);
        let fresh = worth(&setup, &new, 0.0);
        assert!(fresh < new_price(3, 0) && fresh > 0.3 * new_price(3, 0));
        let mut worn = new;
        worn.damage.bearings = 1.0;
        worn.treads = [crate::physics::tire::Tread {
            wear: 0.8,
            ..default_tread()
        }; 4];
        assert!(worth(&setup, &worn, 0.0) < fresh - 1_000.0);
        assert!(worth(&setup, &new, 1.0) < fresh / 2.0);
        assert_eq!(repair(&Damage::default(), 4), 0.0);
        assert!(repair(&worn.damage, 4) > BEARING_REPAIR);
    }

    fn default_tread() -> crate::physics::tire::Tread {
        crate::physics::tire::Tread::default()
    }
}
