//! The career as text: `<key> <values>` lines for the money, trust and counts, the ride,
//! an `item` line per part waiting in the garage, and a block per car from `car` to
//! `endcar` (its build, engine parts, fuel, treads, engine wear, and its ECU tune between
//! `tune` and `endtune`, as tunes are saved).

use super::{Career, Item, OwnedCar, Ride, Wear};
use crate::car::CarSetup;
use crate::chassis::{CHASSIS, CLUTCHES, CarBuild, GEARBOXES, PAINTS, SUSPENSIONS};
use crate::physics::engine::EngineBuild;
use crate::physics::engine::damage::{Damage, MAX_CYLINDERS};
use crate::physics::engine::ecu::{Ecu, EcuModel};
use crate::physics::engine::families::FAMILIES;
use crate::physics::engine::maps::EngineMaps;
use crate::physics::engine::parts::{Parts, SLOTS};
use crate::physics::tire::{COMPOUNDS, Tread};
use crate::storage::Folder;
use crate::workshop;

const SAVES: Folder = Folder {
    cache: false,
    name: "career",
    extension: "save",
    web_prefix: "car-sim-career:",
};
const NAME: &str = "career";

pub fn load() -> Option<String> {
    SAVES.load(NAME)
}

pub fn store(text: &str) -> Result<(), String> {
    SAVES.save(NAME, text)
}

pub fn write(career: &Career) -> String {
    let mut text = String::from("# car-sim career\n");
    text += &format!("money {}\n", career.money);
    text += &format!("trust {}\n", career.trust);
    text += &format!("pizzas {}\n", career.pizzas);
    text += &format!("deliveries {}\n", career.deliveries);
    text += &match career.ride {
        Ride::Mom => "ride mom\n".to_string(),
        Ride::Own(i) => format!("ride {i}\n"),
    };
    for item in &career.items {
        text += &format!("item {}\n", item_words(item));
    }
    for car in &career.cars {
        write_car(&mut text, car);
    }
    text
}

fn item_words(item: &Item) -> String {
    match *item {
        Item::Engine { slot, part } => format!("engine {slot} {part}"),
        Item::Ecu(model) => format!("ecu {}", model.key()),
        Item::Tires(i) => format!("tires {i}"),
        Item::Clutch(i) => format!("clutch {i}"),
        Item::Suspension(i) => format!("suspension {i}"),
        Item::Gearbox(i) => format!("gearbox {i}"),
        Item::LimitedSlip => "lsd".to_string(),
    }
}

fn read_item(words: &[&str]) -> Option<Item> {
    let number = |i: usize| words.get(i)?.parse::<usize>().ok();
    let item = match *words.first()? {
        "engine" => {
            let (slot, part) = (number(1)?, number(2)?);
            (part < SLOTS.get(slot)?.parts.len()).then_some(Item::Engine { slot, part })?
        }
        "ecu" => Item::Ecu(ecu_model(words.get(1)?)?),
        "tires" => Item::Tires(number(1).filter(|&i| i < COMPOUNDS.len())?),
        "clutch" => Item::Clutch(number(1).filter(|&i| i < CLUTCHES.len())?),
        "suspension" => Item::Suspension(number(1).filter(|&i| i < SUSPENSIONS.len())?),
        "gearbox" => Item::Gearbox(number(1).filter(|&i| i < GEARBOXES.len())?),
        "lsd" => Item::LimitedSlip,
        _ => return None,
    };
    Some(item)
}

fn ecu_model(key: &str) -> Option<EcuModel> {
    EcuModel::ALL.into_iter().find(|model| model.key() == key)
}

fn write_car(text: &mut String, car: &OwnedCar) {
    let (b, engine) = (&car.setup.build, &car.setup.engine);
    let p = &engine.parts;
    *text += "car\n";
    *text += &format!(
        "build {} {} {} {} {} {} {} {}\n",
        b.chassis,
        b.paint,
        b.gearbox,
        b.clutch,
        b.final_drive,
        u8::from(b.limited_slip),
        b.suspension,
        b.tires
    );
    let chosen: Vec<String> = p.chosen.iter().map(usize::to_string).collect();
    *text += &format!(
        "parts {} {} {} {} {} {} {} {}\n",
        p.family,
        p.version,
        chosen.join(" "),
        p.intake_timing,
        p.exhaust_timing,
        p.boost,
        p.ecu.key(),
        p.header_room
    );
    *text += &format!("octane {}\n", engine.build.octane);
    let wear = &car.wear;
    *text += &format!("fuel {}\n", wear.fuel);
    let treads: Vec<String> = (wear.treads.iter())
        .map(|t| format!("{} {}", t.temperature, t.wear))
        .collect();
    *text += &format!("treads {}\n", treads.join(" "));
    let d = &wear.damage;
    let mut damage: Vec<String> = (d.cylinders.iter())
        .flat_map(|c| [c.piston, c.valves, c.rod])
        .map(|v| v.to_string())
        .collect();
    damage.extend([d.head_gasket, d.bearings, d.turbo].map(|v| v.to_string()));
    *text += &format!("damage {}\n", damage.join(" "));
    *text += "tune\n";
    *text += &engine.build.ecu.to_text(&engine.build.hardware.name);
    *text += "endtune\nendcar\n";
}

/// Reads a career written by `write` into `career`; what's missing or broken is left as
/// it is, and a car that can't be read is left out.
pub fn read(career: &mut Career, text: &str) {
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((&key, rest)) = words.split_first() else {
            continue;
        };
        let number = || rest.first().and_then(|w| w.parse::<f64>().ok());
        match key {
            "money" => career.money = number().unwrap_or(career.money),
            "trust" => career.trust = number().map_or(career.trust, |t| t as f32),
            "pizzas" => career.pizzas = number().map_or(career.pizzas, |n| n as u32),
            "deliveries" => career.deliveries = number().map_or(career.deliveries, |n| n as u32),
            "ride" => {
                career.ride = match rest.first().and_then(|w| w.parse().ok()) {
                    Some(i) => Ride::Own(i),
                    None => Ride::Mom,
                }
            }
            "item" => career.items.extend(read_item(rest)),
            "car" => {
                let block: Vec<&str> = lines
                    .by_ref()
                    .take_while(|l| l.trim() != "endcar")
                    .collect();
                career.cars.extend(read_car(&block));
            }
            _ => {}
        }
    }
    if let Ride::Own(i) = career.ride
        && i >= career.cars.len()
    {
        career.ride = Ride::Mom;
    }
}

fn read_car(block: &[&str]) -> Option<OwnedCar> {
    let (mut build, mut parts, mut octane) = (None, None, 95.0);
    let (mut fuel, mut treads, mut damage) = (0.0, [Tread::default(); 4], Damage::default());
    let mut tune = String::new();
    let mut lines = block.iter();
    while let Some(line) = lines.next() {
        let words: Vec<&str> = line.split_whitespace().collect();
        let Some((&key, rest)) = words.split_first() else {
            continue;
        };
        let numbers: Vec<f32> = rest.iter().filter_map(|w| w.parse().ok()).collect();
        match key {
            "build" => build = read_build(rest),
            "parts" => parts = read_parts(rest),
            "octane" => octane = numbers.first().copied().unwrap_or(octane),
            "fuel" => fuel = numbers.first().copied().unwrap_or(0.0),
            "treads" if numbers.len() == 8 => {
                for (tread, pair) in treads.iter_mut().zip(numbers.chunks(2)) {
                    *tread = Tread {
                        temperature: pair[0],
                        wear: pair[1].clamp(0.0, 1.0),
                    };
                }
            }
            "damage" if numbers.len() == 3 * MAX_CYLINDERS + 3 => {
                for (cylinder, wear) in damage.cylinders.iter_mut().zip(numbers.chunks(3)) {
                    (cylinder.piston, cylinder.valves, cylinder.rod) = (wear[0], wear[1], wear[2]);
                }
                let engine_wide = &numbers[3 * MAX_CYLINDERS..];
                (damage.head_gasket, damage.bearings, damage.turbo) =
                    (engine_wide[0], engine_wide[1], engine_wide[2]);
            }
            "tune" => {
                for line in lines.by_ref().take_while(|l| l.trim() != "endtune") {
                    tune += line;
                    tune.push('\n');
                }
            }
            _ => {}
        }
    }
    let (build, parts) = (build?, parts?);
    let parts = build.install(parts).ok()?;
    let hardware = parts.hardware();
    let maps = workshop::at_hand(&hardware).unwrap_or_else(|| {
        let maps = EngineMaps::for_hardware(&hardware);
        workshop::keep(&hardware, &maps);
        maps
    });
    let mut ecu = Ecu::calibrated(&hardware, &maps, 95.0, parts.ecu);
    ecu.read_text(&tune);
    let engine = EngineBuild {
        hardware,
        maps,
        ecu,
        octane,
    };
    let setup = CarSetup::new(build, parts, engine);
    let fuel = fuel.clamp(0.0, setup.tank.litres);
    Some(OwnedCar {
        setup,
        wear: Wear {
            damage,
            fuel,
            treads,
        },
    })
}

fn read_build(words: &[&str]) -> Option<CarBuild> {
    let n = |i: usize| words.get(i)?.parse::<usize>().ok();
    let build = CarBuild {
        chassis: n(0).filter(|&c| c < CHASSIS.len())?,
        paint: n(1).filter(|&p| p < PAINTS.len())?,
        gearbox: n(2).filter(|&g| g < GEARBOXES.len())?,
        clutch: n(3).filter(|&c| c < CLUTCHES.len())?,
        final_drive: words.get(4)?.parse().ok().filter(|f: &f32| f.is_finite())?,
        limited_slip: n(5)? != 0,
        suspension: n(6).filter(|&s| s < SUSPENSIONS.len())?,
        tires: n(7).filter(|&t| t < COMPOUNDS.len())?,
    };
    Some(build)
}

fn read_parts(words: &[&str]) -> Option<Parts> {
    let n = |i: usize| words.get(i)?.parse::<usize>().ok();
    let f = |i: usize| words.get(i)?.parse::<f32>().ok();
    let family = n(0).filter(|&f| f < FAMILIES.len())?;
    let version = n(1).filter(|&v| v < FAMILIES[family].versions.len())?;
    let mut parts = Parts::all_stock(family, version);
    for (slot, chosen) in parts.chosen.iter_mut().enumerate() {
        *chosen = n(2 + slot).filter(|&c| c < SLOTS[slot].parts.len())?;
    }
    let after = 2 + SLOTS.len();
    parts.intake_timing = f(after)?;
    parts.exhaust_timing = f(after + 1)?;
    parts.boost = f(after + 2)?;
    parts.ecu = ecu_model(words.get(after + 3)?)?;
    parts.header_room = f(after + 4)?;
    Some(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_career_reads_back_as_it_was_written() {
        let mut career = Career::new();
        career.money = 1234.5;
        career.trust = 17.0;
        career.pizzas = 3;
        career.items = vec![
            Item::Engine { slot: 12, part: 2 },
            Item::Ecu(EcuModel::Programmable),
            Item::Tires(1),
            Item::LimitedSlip,
        ];
        // A turbo city car with a worn engine and a little fuel, and a stock coupé.
        let mut minim = CarSetup::factory(0, 1);
        minim.engine.build.ecu.rev_limit = 6543.0;
        let mut wear = Wear::new(&minim);
        wear.fuel = 7.5;
        wear.damage.cylinders[2].piston = 0.25;
        wear.damage.turbo = 0.5;
        wear.treads[3].wear = 0.4;
        career.cars.push(OwnedCar { setup: minim, wear });
        let coupe = CarSetup::factory(3, 0);
        let wear = Wear::new(&coupe);
        career.cars.push(OwnedCar { setup: coupe, wear });
        career.ride = Ride::Own(1);

        let text = write(&career);
        let mut back = Career::new();
        read(&mut back, &text);
        assert_eq!(back.money, 1234.5);
        assert_eq!(back.trust, 17.0);
        assert_eq!(back.pizzas, 3);
        assert_eq!(back.items, career.items);
        assert_eq!(back.ride, Ride::Own(1));
        assert_eq!(back.cars.len(), 2);
        let (a, b) = (&career.cars[0], &back.cars[0]);
        assert_eq!(a.setup.build, b.setup.build);
        assert_eq!(a.setup.engine.parts, b.setup.engine.parts);
        assert_eq!(b.setup.engine.build.ecu.rev_limit, 6543.0);
        assert_eq!(b.wear.fuel, 7.5);
        assert_eq!(b.wear.damage, a.wear.damage);
        assert_eq!(b.wear.treads[3].wear, 0.4);
        assert_eq!(
            back.cars[1].setup.engine.parts,
            career.cars[1].setup.engine.parts
        );
    }

    #[test]
    fn broken_saves_keep_what_they_can() {
        let mut career = Career::new();
        read(
            &mut career,
            "money 99\nitem engine 99 1\nitem tires 1\ncar\nbuild 99\nendcar\nride 3\n",
        );
        assert_eq!(career.money, 99.0);
        assert_eq!(career.items, [Item::Tires(1)]);
        assert!(career.cars.is_empty());
        assert_eq!(career.ride, Ride::Mom);
    }
}
