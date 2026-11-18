//! The career as text: `<key> <values>` lines for the money, trust and counts, the ride,
//! an `item` line per part waiting in the garage, a block per car from `car` to `endcar`
//! (its build, engine parts, fuel, treads, engine wear, and its ECU tune between `tune`
//! and `endtune`, as tunes are saved), and a block per dyno run from `run` to `endrun`
//! (its texts with newlines and backslashes escaped, and a `point` line per sample).

use super::dyno::DynoRun;
use super::rollers::Sample;
use super::{Career, Item, OwnedCar, Ride, Wear};
use crate::car::CarSetup;
use crate::chassis::{CHASSIS, CLUTCHES, CarBuild, GEARBOXES, PAINTS, SUSPENSIONS};
use crate::physics::engine::EngineBuild;
use crate::physics::engine::damage::{Damage, MAX_CYLINDERS};
use crate::physics::engine::ecu::{Ecu, EcuModel};
use crate::physics::engine::families::FAMILIES;
use crate::physics::engine::maps::EngineMaps;
use crate::physics::engine::parts::{Parts, SLOTS};
use crate::physics::tire::{COMPOUNDS, RIBS, Tread};
use crate::storage::Folder;
use crate::wheels::{Rim, TireSize, Wheel};
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
    for run in &career.runs {
        write_run(&mut text, run);
    }
    text
}

/// A text on one line: backslashes and newlines escaped.
fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\n', "\\n")
}

fn unescape(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
        } else if let Some(next) = chars.next() {
            out.push(if next == 'n' { '\n' } else { next });
        }
    }
    out
}

fn write_run(text: &mut String, run: &DynoRun) {
    *text += &format!("run {} {}\n", run.number, run.gear);
    *text += &format!("name {}\n", escape(&run.name));
    *text += &format!("about {}\n", escape(&run.car));
    *text += &format!("notes {}\n", escape(&run.notes));
    for broke in &run.broke {
        *text += &format!("broke {}\n", escape(broke));
    }
    for s in &run.samples {
        *text += &format!("point {} {} {}\n", s.rpm, s.crank, s.wheel);
    }
    *text += "endrun\n";
}

/// A run from its first line's words after `run` and the lines up to `endrun`.
fn read_run(words: &[&str], block: &[&str]) -> Option<DynoRun> {
    let mut run = DynoRun {
        number: words.first()?.parse().ok()?,
        name: String::new(),
        notes: String::new(),
        car: String::new(),
        gear: words.get(1)?.parse().ok()?,
        samples: Vec::new(),
        broke: Vec::new(),
    };
    for line in block {
        let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "name" => run.name = unescape(rest),
            "about" => run.car = unescape(rest),
            "notes" => run.notes = unescape(rest),
            "broke" => run.broke.push(unescape(rest)),
            "point" => {
                let numbers: Vec<f32> = (rest.split_whitespace())
                    .filter_map(|w| w.parse().ok())
                    .filter(|n: &f32| n.is_finite())
                    .collect();
                if let [rpm, crank, wheel] = numbers[..] {
                    run.samples.push(Sample { rpm, crank, wheel });
                }
            }
            _ => {}
        }
    }
    Some(run)
}

fn item_words(item: &Item) -> String {
    match *item {
        Item::Engine { slot, part } => format!("engine {slot} {part}"),
        Item::Ecu(model) => format!("ecu {}", model.key()),
        Item::Rims(rim) => format!("rims {} {}", rim.diameter, rim.width),
        Item::Tires(size, compound) => format!(
            "tires {} {} {} {}",
            size.width,
            size.aspect,
            size.rim,
            compound_key(compound)
        ),
        Item::Clutch(i) => format!("clutch {i}"),
        Item::Suspension(i) => format!("suspension {i}"),
        Item::RollBar {
            chassis,
            axle,
            diameter,
        } => format!("rollbar {chassis} {axle} {diameter}"),
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
        "rims" => {
            let rim = Rim {
                diameter: number(1)?.try_into().ok()?,
                width: number(2)?.try_into().ok()?,
            };
            Rim::all().any(|r| r == rim).then_some(Item::Rims(rim))?
        }
        "tires" => {
            let size = TireSize {
                width: number(1)?.try_into().ok()?,
                aspect: number(2)?.try_into().ok()?,
                rim: number(3)?.try_into().ok()?,
            };
            let compound = compound(words.get(4)?)?;
            TireSize::all()
                .any(|s| s == size)
                .then_some(Item::Tires(size, compound))?
        }
        "clutch" => Item::Clutch(number(1).filter(|&i| i < CLUTCHES.len())?),
        "suspension" => Item::Suspension(number(1).filter(|&i| i < SUSPENSIONS.len())?),
        "rollbar" => {
            let (chassis, axle) = (number(1)?, number(2)?);
            let diameter = u8::try_from(number(3)?).ok()?;
            (CHASSIS.get(chassis)?.bars.get(axle)?.find(diameter))?;
            Item::RollBar {
                chassis,
                axle,
                diameter,
            }
        }
        "gearbox" => Item::Gearbox(number(1).filter(|&i| i < GEARBOXES.len())?),
        "lsd" => Item::LimitedSlip,
        _ => return None,
    };
    Some(item)
}

/// What a set of tires of a compound cost before tires had sizes: an old save's set on the
/// shelf (`tires <compound>`) is paid back, not knowing which car it was for.
fn old_tires_refund(words: &[&str]) -> Option<f64> {
    let [kind, which] = words else {
        return None;
    };
    let compound = compound(which).filter(|_| *kind == "tires")?;
    Some([260.0, 320.0, 650.0, 1_100.0][compound])
}

/// A compound by its name in lower case.
fn compound_key(compound: usize) -> String {
    COMPOUNDS[compound].name.to_lowercase()
}

/// A compound by its name in lower case, or by its place among the first three (street,
/// sport, race), as saves had it before there were touring tires.
fn compound(word: &str) -> Option<usize> {
    let name = match word.parse::<usize>() {
        Ok(old) => ["street", "sport", "race"].get(old)?.to_string(),
        Err(_) => word.to_string(),
    };
    (0..COMPOUNDS.len()).find(|&i| compound_key(i) == name)
}

fn ecu_model(key: &str) -> Option<EcuModel> {
    EcuModel::ALL.into_iter().find(|model| model.key() == key)
}

fn write_car(text: &mut String, car: &OwnedCar) {
    let (b, engine) = (&car.setup.build, &car.setup.engine);
    let p = &engine.parts;
    *text += "car\n";
    *text += &format!(
        "build {} {} {} {} {} {} {}\n",
        b.chassis,
        b.paint,
        b.gearbox,
        b.clutch,
        b.final_drive,
        u8::from(b.limited_slip),
        b.suspension,
    );
    let wheels: Vec<String> = (b.wheels.iter())
        .map(|w| {
            let (rim, tire) = (w.rim, w.tire);
            format!(
                "{} {} {} {} {}",
                rim.diameter,
                rim.width,
                tire.width,
                tire.aspect,
                compound_key(w.compound)
            )
        })
        .collect();
    *text += &format!("wheels {}\n", wheels.join(" "));
    // Camber front and rear (degrees), then cold pressures (bar).
    let [cf, cr] = b.camber;
    let [pf, pr] = b.pressure;
    *text += &format!("align {cf} {cr} {pf} {pr}\n");
    // The anti-roll bars' diameters front and rear (mm).
    let bars = [0, 1].map(|axle| b.chassis().bars[axle].sizes[b.bars[axle]]);
    *text += &format!("bars {} {}\n", bars[0], bars[1]);
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
    *text += &format!("age {}\n", car.age);
    let wear = &car.wear;
    *text += &format!("fuel {}\n", wear.fuel);
    // Each tire's wear across its ribs (a parked car cools down).
    let treads: Vec<String> = (wear.treads.iter())
        .flat_map(|t| t.wear)
        .map(|w| w.to_string())
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
            "item" => match read_item(rest) {
                Some(item) => career.items.push(item),
                None => career.money += old_tires_refund(rest).unwrap_or(0.0),
            },
            "car" => {
                let block: Vec<&str> = lines
                    .by_ref()
                    .take_while(|l| l.trim() != "endcar")
                    .collect();
                career.cars.extend(read_car(&block));
            }
            "run" => {
                let block: Vec<&str> = lines
                    .by_ref()
                    .take_while(|l| l.trim() != "endrun")
                    .collect();
                career.runs.extend(read_run(rest, &block));
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
    let (mut build, mut parts, mut octane, mut age) = (None, None, 95.0, 0.0);
    let (mut wheels, mut align, mut bars) = (None, None, None);
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
            "wheels" => wheels = read_wheels(rest),
            "align" if numbers.len() == 4 && numbers.iter().all(|n| n.is_finite()) => {
                align = Some([[numbers[0], numbers[1]], [numbers[2], numbers[3]]]);
            }
            "bars" if numbers.len() == 2 => bars = Some([numbers[0], numbers[1]]),
            "parts" => parts = read_parts(rest),
            "octane" => octane = numbers.first().copied().unwrap_or(octane),
            "age" => {
                age = numbers
                    .first()
                    .map_or(0.0, |&a| f64::from(a).clamp(0.0, 1.0))
            }
            "fuel" => fuel = numbers.first().copied().unwrap_or(0.0),
            "treads" if numbers.len() == 4 * RIBS => {
                for (tread, ribs) in treads.iter_mut().zip(numbers.chunks(RIBS)) {
                    for (wear, &rib) in tread.wear.iter_mut().zip(ribs) {
                        *wear = rib.clamp(0.0, 1.0);
                    }
                }
            }
            // Before ribs: a temperature and a wear per tire.
            "treads" if numbers.len() == 8 => {
                for (tread, pair) in treads.iter_mut().zip(numbers.chunks(2)) {
                    tread.set_wear(pair[1].clamp(0.0, 1.0));
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
    let (mut build, parts): (CarBuild, _) = (build?, parts?);
    if let Some(wheels) = wheels {
        build.wheels = wheels;
    }
    // Saves from before the alignment have the kit's camber and the factory's pressures.
    if let Some([camber, pressure]) = align {
        (build.camber, build.pressure) = (camber, pressure);
    }
    build.clamp_alignment();
    // Saves from before anti-roll bars have the factory's; a size no longer made too.
    for (axle, diameter) in bars.into_iter().flatten().enumerate() {
        let made = &build.chassis().bars[axle];
        if let Some(bar) = u8::try_from(diameter as u32)
            .ok()
            .and_then(|d| made.find(d))
        {
            build.bars[axle] = bar;
        }
    }
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
        age,
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
        ..CarBuild::factory(n(0)?)
    }
    .with_suspension(n(6).filter(|&s| s < SUSPENSIONS.len())?);
    // Saves from before rims and tire sizes have the compound here, on factory sizes.
    let mut build = build;
    if let Some(compound) = words.get(7).and_then(|word| compound(word)) {
        for wheel in &mut build.wheels {
            wheel.compound = compound;
        }
    }
    Some(build)
}

/// Front and rear: rim diameter and width, tire width and aspect, and compound.
fn read_wheels(words: &[&str]) -> Option<[Wheel; 2]> {
    let n = |i: usize| words.get(i)?.parse::<u16>().ok();
    let byte = |i: usize| n(i).and_then(|v| u8::try_from(v).ok());
    let wheel = |at: usize| {
        let wheel = Wheel {
            rim: Rim {
                diameter: byte(at)?,
                width: byte(at + 1)?,
            },
            tire: TireSize {
                width: n(at + 2)?,
                aspect: byte(at + 3)?,
                rim: byte(at)?,
            },
            compound: compound(words.get(at + 4)?)?,
        };
        wheel.tire.fits(&wheel.rim).then_some(wheel)
    };
    Some([wheel(0)?, wheel(5)?])
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
            Item::Rims(Rim {
                diameter: 18,
                width: 85,
            }),
            Item::Tires(
                TireSize {
                    width: 245,
                    aspect: 40,
                    rim: 18,
                },
                SPORT,
            ),
            Item::LimitedSlip,
            Item::RollBar {
                chassis: 3,
                axle: 1,
                diameter: 22,
            },
        ];
        // A turbo city car with a worn engine and a little fuel, and a stock coupé.
        let mut minim = CarSetup::factory(0, 1);
        minim.engine.build.ecu.rev_limit = 6543.0;
        let mut wear = Wear::new(&minim);
        wear.fuel = 7.5;
        wear.damage.cylinders[2].piston = 0.25;
        wear.damage.turbo = 0.5;
        wear.treads[3].wear[4] = 0.4;
        minim.build.wheels[1] = Wheel {
            rim: Rim {
                diameter: 15,
                width: 65,
            },
            tire: TireSize {
                width: 195,
                aspect: 50,
                rim: 15,
            },
            compound: 2,
        };
        (minim.build.camber, minim.build.pressure) = ([-1.2, -0.9], [2.45, 1.95]);
        minim.build.bars = [4, 0];
        career.cars.push(OwnedCar {
            setup: minim,
            wear,
            age: 0.5,
        });
        let coupe = CarSetup::factory(3, 0);
        let wear = Wear::new(&coupe);
        career.cars.push(OwnedCar {
            setup: coupe,
            wear,
            age: 0.0,
        });
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
        assert_eq!(b.wear.treads[3].wear[4], 0.4);
        assert_eq!(b.age, 0.5);
        assert_eq!(
            back.cars[1].setup.engine.parts,
            career.cars[1].setup.engine.parts
        );
    }

    use crate::physics::tire::SPORT;

    #[test]
    fn old_saves_put_their_compound_on_factory_sizes() {
        let build = read_build(&["3", "0", "6", "0", "3.9", "0", "0", "1"]).unwrap();
        let factory = CarBuild::factory(3).wheels;
        assert_eq!(build.wheels.map(|w| w.tire), factory.map(|w| w.tire));
        assert_eq!(build.wheels.map(|w| w.compound), [SPORT; 2]);
    }

    #[test]
    fn dyno_runs_read_back_with_their_texts() {
        let mut career = Career::new();
        let sample = |rpm: f32| Sample {
            rpm,
            crank: 200.5,
            wheel: 90_000.25,
        };
        career.runs.push(DynoRun {
            number: 7,
            name: "Before the cams \\o/".to_string(),
            notes: "Hot day.\nSecond line, with a \\n in it.".to_string(),
            car: "Tarn · W20 2.0".to_string(),
            gear: 4,
            samples: vec![sample(2000.0), sample(2100.0)],
            broke: vec!["Cylinder 2's piston is holed".to_string()],
        });
        let mut back = Career::new();
        read(&mut back, &write(&career));
        assert_eq!(back.runs, career.runs);
        // A run line without its number is left out.
        read(&mut back, "run\nname x\nendrun\nmoney 3\n");
        assert_eq!((back.runs.len(), back.money), (1, 3.0));
    }

    #[test]
    fn broken_saves_keep_what_they_can() {
        let mut career = Career::new();
        read(
            &mut career,
            "money 99\nitem engine 99 1\nitem tires 1\ncar\nbuild 99\nendcar\nride 3\n",
        );
        // A set of tires from before they had sizes is paid back (sport tires, as old
        // saves name compounds by their place among street, sport and race).
        assert_eq!(career.money, 99.0 + 650.0);
        assert!(career.items.is_empty());
        assert!(career.cars.is_empty());
        assert_eq!(career.ride, Ride::Mom);
    }
}
