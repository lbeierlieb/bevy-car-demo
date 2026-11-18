//! Fitting a part from the shelf to one of the player's cars. The part it replaces goes onto
//! the shelf (worn tires go to the bin). Fitting never touches the ECU's tune: after
//! changing the engine, the tune is still the one made for the old parts until it's
//! changed by hand.

use super::{Item, OwnedCar, Wear};
use crate::car::CarSetup;
use crate::chassis::{CLUTCHES, GEARBOXES, SUSPENSIONS};
use crate::physics::engine::EngineBuild;
use crate::physics::engine::maps::{self, EngineMaps};
use crate::physics::engine::parts::{Parts, SLOTS};
use crate::physics::tire::COMPOUNDS;

/// What a part is called on the shelf and in the shop.
pub fn name(item: Item) -> String {
    match item {
        Item::Engine { slot, part } => {
            format!("{}: {}", SLOTS[slot].name, SLOTS[slot].parts[part].name)
        }
        Item::Ecu(model) => format!("ECU: {}", model.name()),
        Item::Tires(i) => format!("{} tires", COMPOUNDS[i].name),
        Item::Clutch(i) => format!("{} clutch", CLUTCHES[i].0),
        Item::Suspension(i) => format!("{} suspension kit", SUSPENSIONS[i].0),
        Item::Gearbox(i) => format!("Gearbox: {}", GEARBOXES[i].name),
        Item::LimitedSlip => "Limited-slip differentials".to_string(),
    }
}

/// Fitting a part: what comes off (onto the shelf), and the engine's new parts if the
/// engine changes (its maps are needed before it's done, see `finish_engine`).
#[derive(Debug, PartialEq)]
pub struct Fitted {
    pub off: Option<Item>,
    pub engine: Option<Parts>,
}

/// Fits `item` to `car`, or says why it doesn't go on.
pub fn fit(car: &mut OwnedCar, item: Item) -> Result<Fitted, String> {
    let mut build = car.setup.build.clone();
    let mut parts = car.setup.engine.parts.clone();
    let off = match item {
        Item::Tires(i) => {
            if build.wheels.iter().all(|wheel| wheel.compound == i) {
                car.wear.treads = Wear::new(&car.setup).treads;
            }
            for wheel in &mut build.wheels {
                wheel.compound = i;
            }
            None
        }
        Item::Clutch(i) => Some(Item::Clutch(std::mem::replace(&mut build.clutch, i))),
        Item::Suspension(i) => Some(Item::Suspension(std::mem::replace(
            &mut build.suspension,
            i,
        ))),
        Item::Gearbox(i) => {
            if !build.chassis().gearboxes().any(|g| g == i) {
                return Err(format!(
                    "The {} doesn't fit this car's layout.",
                    GEARBOXES[i].name
                ));
            }
            Some(Item::Gearbox(std::mem::replace(&mut build.gearbox, i)))
        }
        Item::LimitedSlip => {
            if build.limited_slip {
                return Err("It has limited-slip differentials already.".to_string());
            }
            build.limited_slip = true;
            None
        }
        Item::Ecu(model) => Some(Item::Ecu(std::mem::replace(&mut parts.ecu, model))),
        Item::Engine { slot, part } => {
            let old = std::mem::replace(&mut parts.chosen[slot], part);
            let off = Item::Engine { slot, part: old };
            // Taking a part off for nothing ("None") leaves nothing for the shelf.
            super::catalog::price(off).map(|_| off)
        }
    };
    let engine_changed = parts != car.setup.engine.parts;
    if build != car.setup.build {
        let tires_changed = build.wheels != car.setup.build.wheels;
        let engine = car.setup.engine.build.clone();
        car.setup = CarSetup::new(build, car.setup.engine.parts.clone(), engine);
        if tires_changed {
            car.wear.treads = Wear::new(&car.setup).treads;
        }
    }
    let engine = if !engine_changed {
        None
    } else if parts.ecu != car.setup.engine.parts.ecu && same_hardware(&parts, car) {
        // Only the ECU changed: same maps, same tune, another box.
        finish_engine(car, parts, car.setup.engine.build.maps.clone());
        None
    } else {
        Some(parts)
    };
    Ok(Fitted { off, engine })
}

/// Whether `parts` make the same engine as the car's (apart from the ECU).
fn same_hardware(parts: &Parts, car: &OwnedCar) -> bool {
    let current = &car.setup.engine.build.hardware;
    maps::fingerprint(&parts.hardware()) == maps::fingerprint(current)
}

/// Puts the engine built from `parts` (with `maps` baked for it) into `car`. Its ECU keeps
/// its tune; only the box changes if the parts have another.
pub fn finish_engine(car: &mut OwnedCar, parts: Parts, maps: EngineMaps) {
    let old = &car.setup.engine.build;
    let mut ecu = old.ecu.clone();
    ecu.model = parts.ecu;
    let engine = EngineBuild {
        hardware: parts.hardware(),
        maps,
        ecu,
        octane: old.octane,
    };
    let build = car.setup.build.clone();
    car.setup = CarSetup::new(build, parts, engine);
    car.wear.fuel = car.wear.fuel.min(car.setup.tank.litres);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::engine::ecu::EcuModel;
    use crate::physics::engine::parts::TURBO;
    use crate::physics::tire::SPORT;

    fn coupe() -> OwnedCar {
        let setup = CarSetup::factory(3, 0);
        let wear = Wear::new(&setup);
        OwnedCar {
            setup,
            wear,
            age: 0.0,
        }
    }

    #[test]
    fn chassis_parts_swap_and_the_old_ones_go_on_the_shelf() {
        let mut car = coupe();
        let fitted = fit(&mut car, Item::Clutch(2)).unwrap();
        assert_eq!(fitted.off, Some(Item::Clutch(0)));
        assert_eq!(car.setup.build.clutch, 2);
        car.wear.treads[0].wear = 0.5;
        let fitted = fit(&mut car, Item::Tires(SPORT)).unwrap();
        assert_eq!(
            fitted,
            Fitted {
                off: None,
                engine: None
            }
        );
        assert_eq!(
            car.setup.build.wheels.map(|wheel| wheel.compound),
            [SPORT; 2]
        );
        assert_eq!(car.wear.treads[0].wear, 0.0);
        assert!(fit(&mut car, Item::Gearbox(0)).is_err(), "a transverse box");
        // The coupé comes with open differentials.
        fit(&mut car, Item::LimitedSlip).unwrap();
        assert!(car.setup.build.limited_slip);
        fit(&mut car, Item::LimitedSlip).unwrap_err();
    }

    #[test]
    fn engine_parts_keep_the_tune() {
        let mut car = coupe();
        car.setup.engine.build.ecu.rev_limit = 5000.0;
        let fitted = fit(
            &mut car,
            Item::Engine {
                slot: TURBO,
                part: 2,
            },
        )
        .unwrap();
        // The turbo slot had none: nothing for the shelf.
        assert_eq!(fitted.off, None);
        let parts = fitted.engine.expect("a new engine");
        assert_eq!(parts.chosen[TURBO], 2);
        let maps = EngineMaps::for_hardware(&parts.hardware());
        finish_engine(&mut car, parts, maps);
        assert!(car.setup.engine.build.hardware.turbo.is_some());
        assert_eq!(car.setup.engine.build.ecu.rev_limit, 5000.0);
        // Another ECU box keeps the tune too, and the maps.
        let fitted = fit(&mut car, Item::Ecu(EcuModel::Programmable)).unwrap();
        assert_eq!(
            fitted,
            Fitted {
                off: Some(Item::Ecu(EcuModel::Stock)),
                engine: None
            }
        );
        assert_eq!(car.setup.engine.build.ecu.model, EcuModel::Programmable);
        assert_eq!(car.setup.engine.build.ecu.rev_limit, 5000.0);
    }
}
