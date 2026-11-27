//! The built-in catalogue's chassis, gearboxes, clutches, suspension kits and paints.

use super::{Clutch, Paint, Suspension};
use crate::body::{Body, Detail, Tone};
use crate::cargo::{Area, Hold};
use crate::chassis::{Chassis, EngineBay, Gearbox, GearboxKind, Orientation, RollBars};
use crate::physics::drivetrain::Drive;
use crate::physics::fuel::Tank;
use crate::physics::tire::STREET;
use crate::wheels::{Rim, TireSize, Wheel, WheelRoom};

pub fn clutches() -> Vec<Clutch> {
    let clutch = |name: &str, torque| Clutch {
        name: name.into(),
        torque,
    };
    vec![
        clutch("Stock", 1.0),
        clutch("Sport", 1.6),
        clutch("Race", 2.5),
    ]
}

pub fn suspensions() -> Vec<Suspension> {
    let kit = |name: &str, ride, damping, camber, reach| Suspension {
        name: name.into(),
        ride,
        damping,
        camber,
        reach,
    };
    vec![
        kit("Stock", 1.0, 1.0, 0.0, 0.5),
        kit("Sport", 1.2, 1.15, 0.8, 1.0),
        kit("Track", 1.45, 1.3, 1.8, 2.0),
    ]
}

/// The paint shops' first, then the dull ones cars leave the factory in (one per
/// chassis, so there's a reason to go to a paint shop).
pub fn paints() -> Vec<Paint> {
    let shop = [
        ("Signal red", [0.80, 0.10, 0.08]),
        ("Racing blue", [0.10, 0.28, 0.75]),
        ("Sunflower", [0.95, 0.72, 0.08]),
        ("Glacier white", [0.90, 0.90, 0.88]),
        ("Graphite", [0.20, 0.21, 0.23]),
        ("Silver", [0.62, 0.64, 0.67]),
        ("Forest green", [0.08, 0.32, 0.18]),
        ("Tangerine", [0.92, 0.42, 0.08]),
        ("Black", [0.04, 0.04, 0.05]),
        ("Sky blue", [0.40, 0.66, 0.86]),
    ];
    let factory = [
        ("Pale beige", [0.76, 0.71, 0.60]),
        ("Putty", [0.62, 0.60, 0.54]),
        ("Faded olive", [0.42, 0.43, 0.32]),
        ("Primer grey", [0.47, 0.48, 0.47]),
        ("Mushroom", [0.55, 0.50, 0.45]),
        ("Off-white", [0.80, 0.79, 0.74]),
        ("Mud brown", [0.36, 0.29, 0.22]),
        ("Slate", [0.33, 0.36, 0.39]),
        ("Dull navy", [0.18, 0.21, 0.30]),
        ("Dove grey", [0.66, 0.66, 0.64]),
        ("Khaki", [0.58, 0.53, 0.40]),
    ];
    let paint = |shop: bool| {
        move |(name, color): (&str, [f32; 3])| Paint {
            name: name.into(),
            color,
            shop,
        }
    };
    (shop.into_iter().map(paint(true)))
        .chain(factory.into_iter().map(paint(false)))
        .collect()
}

/// Anti-roll bars of `sizes` (mm), the factory's at `factory` in them, and a 20 mm one
/// giving the axle `rate` (N·m per degree of roll).
fn bars(sizes: &[u8], factory: usize, rate: f32) -> RollBars {
    RollBars {
        sizes: sizes.to_vec(),
        factory,
        rate,
    }
}

/// The codes of engines.
fn codes(codes: &[&str]) -> Vec<String> {
    codes.iter().map(|code| code.to_string()).collect()
}

pub fn gearboxes() -> Vec<Gearbox> {
    vec![
        Gearbox {
            name: "5-speed (transverse)".into(),
            kind: GearboxKind::Transverse,
            gears: vec![3.55, 1.95, 1.31, 0.97, 0.78],
            reverse: 3.4,
            mass: 35.0,
            clutch_torque: 200.0,
            torque: 200.0,
            shift_time: 0.28,
            engage_time: 0.16,
        },
        Gearbox {
            name: "6-speed close-ratio (transverse)".into(),
            kind: GearboxKind::Transverse,
            gears: vec![3.3, 2.05, 1.48, 1.14, 0.92, 0.76],
            reverse: 3.3,
            mass: 42.0,
            clutch_torque: 380.0,
            torque: 360.0,
            shift_time: 0.22,
            engage_time: 0.15,
        },
        // Behind a big longitudinal engine: long ratios, two overdrives, made for torque.
        Gearbox {
            name: "6-speed heavy duty (inline)".into(),
            kind: GearboxKind::Inline,
            gears: vec![2.66, 1.78, 1.3, 1.0, 0.74, 0.5],
            reverse: 2.9,
            mass: 60.0,
            clutch_torque: 720.0,
            torque: 700.0,
            shift_time: 0.28,
            engage_time: 0.16,
        },
    ]
}

pub const fn wheel(diameter: u8, rim_width: u8, width: u16, aspect: u8, compound: usize) -> Wheel {
    Wheel {
        rim: Rim {
            diameter,
            width: rim_width,
        },
        tire: TireSize {
            width,
            aspect,
            rim: diameter,
        },
        compound,
    }
}

const fn detail(at: [f32; 3], size: [f32; 3], tone: Tone) -> Detail {
    Detail { at, size, tone }
}

/// The hot hatch (with its base engine, the B16), which the game starts with.
pub const DEFAULT_CHASSIS: usize = 0;

pub fn chassis() -> Vec<Chassis> {
    vec![vireo(), merrow()]
}

/// The hot hatch.
fn vireo() -> Chassis {
    Chassis {
        name: "Vireo".into(),
        class: "Hatchback".into(),
        about: "A light three-door hatch from the early nineties: low nose, long glass, a \
                chopped tail, front-wheel drive and eager fours; the 1.8 comes with \
                alloys, bigger brakes and a limited-slip differential"
            .into(),
        // Without the seats (see `items`: 50 kg with them, the car's centre where it was).
        mass: 770.0,
        mass_at: 1.2665,
        cg_height: 0.5,
        wheelbase: 2.57,
        track: [1.475, 1.465],
        // The 1.6's wheels; the 1.8 has its own (see `items`).
        wheels: [
            wheel(14, 55, 185, 60, STREET),
            wheel(14, 55, 185, 60, STREET),
        ],
        room: [
            WheelRoom {
                diameter: 0.62,
                width: 215,
                min_rim: 13,
            },
            WheelRoom {
                diameter: 0.62,
                width: 215,
                min_rim: 13,
            },
        ],
        // Without the brakes (see `items`).
        hub: [8.0, 6.0],
        ride: [1.5, 1.7],
        damping: 0.32,
        bars: [
            bars(&[18, 20, 22, 24, 26], 1, 270.0),
            bars(&[14, 16, 18, 20, 22], 1, 340.0),
        ],
        camber: [-1.0, -1.0],
        pressure: [2.45, 2.1],
        max_steer: 0.55,
        drag_area: 0.62,
        downforce_area: [0.0, 0.0],
        tank: Tank {
            litres: 45.0,
            at: 2.15,
            height: 0.3,
        },
        bay: EngineBay {
            place: "front, transverse".into(),
            orientation: Orientation::Transverse,
            length: 0.74,
            width: 1.0,
            height: 0.66,
            engine_at: -0.12,
            gearbox_at: -0.12,
            header_room: 1.15,
        },
        drive: Drive::Front,
        limited_slip: false,
        gearbox: 0,
        final_drive: 4.25,
        engines: codes(&["B16", "B18"]),
        paint: 11,
        body: Body {
            length: 4.07,
            width: 1.69,
            front_overhang: 0.85,
            floor: 0.14,
            // A low, pointed nose, the bonnet rising to a raked windscreen, a belt line
            // rising a little to the tail.
            deck: vec![
                [0.0, 0.42],
                [0.05, 0.56],
                [0.7, 0.68],
                [1.12, 0.8],
                [3.95, 0.92],
                [4.07, 0.8],
            ],
            // Long door glass and a small quarter window, the roof falling a little to a
            // thick C pillar and a steep, short hatch.
            cabin: vec![
                [1.1, 0.8, 1.0],
                [1.85, 1.32, 1.0],
                [3.0, 1.34, 1.0],
                [3.5, 1.28, 2.0],
                [3.95, 0.95, 0.0],
            ],
            nose: 0.8,
            tail: 0.92,
            tumblehome: 0.8,
            open_wheels: false,
            openings: vec![],
            details: vec![
                // The glass panel low in the hatch, under the number plate's lip.
                detail([4.06, 0.72, 0.0], [0.03, 0.14, 1.1], Tone::Glass),
                // Dark rubbing strips along the doors.
                detail([2.0, 0.5, 0.84], [1.6, 0.06, 0.03], Tone::Dark),
                detail([2.0, 0.5, -0.84], [1.6, 0.06, 0.03], Tone::Dark),
            ],
            // Long doors from the front wheel arch; bumpers wrapping round the ends.
            door: [1.22, 2.45],
            bumpers: [0.3, 0.3],
            bumper_top: 0.44,
            bulges: vec![],
            side_pipes: false,
        },
        eye: [2.0, 1.1, 0.36],
        insulation: 0.4,
        traffic: 3.0,
        // From the passenger's footwell to the hatch: the trunk under its cover, 0.7 m
        // deep; the rear bench's place and the passenger seat's while they're out.
        hold: Hold {
            from: 1.75,
            left: -0.6,
            base: 0.2,
            areas: vec![
                area([15, 22], [0, 12], 3, 7, None),
                area([8, 15], [0, 12], 1, 9, Some("Rear bench")),
                area([0, 8], [7, 12], 0, 9, Some("Passenger seat")),
            ],
        },
    }
}

/// The V10 roadster.
fn merrow() -> Chassis {
    Chassis {
        name: "Merrow".into(),
        class: "Roadster".into(),
        about: "An early-nineties open two-seater built round an eight-litre V10: a long, low \
                bonnet, the engine set back behind the front axle, wide tires at the back, \
                side pipes and no roof to speak of"
            .into(),
        // Without the seats (see `items`).
        mass: 780.0,
        mass_at: 1.42,
        cg_height: 0.46,
        wheelbase: 2.44,
        track: [1.52, 1.54],
        wheels: [
            wheel(17, 95, 275, 40, STREET),
            wheel(17, 115, 335, 35, STREET),
        ],
        room: [
            WheelRoom {
                diameter: 0.7,
                width: 295,
                min_rim: 17,
            },
            WheelRoom {
                diameter: 0.71,
                width: 345,
                min_rim: 17,
            },
        ],
        // Without the brakes (see `items`).
        hub: [16.0, 20.0],
        ride: [1.45, 1.6],
        damping: 0.3,
        bars: [
            bars(&[24, 26, 28, 30, 32], 2, 200.0),
            bars(&[16, 18, 20, 22, 24], 2, 300.0),
        ],
        camber: [-0.8, -1.0],
        pressure: [2.1, 1.85],
        max_steer: 0.5,
        drag_area: 0.82,
        downforce_area: [0.0, 0.0],
        tank: Tank {
            litres: 85.0,
            at: 2.15,
            height: 0.35,
        },
        bay: EngineBay {
            place: "front-mid, longitudinal".into(),
            orientation: Orientation::Longitudinal,
            length: 0.98,
            width: 0.92,
            height: 0.62,
            engine_at: 0.42,
            gearbox_at: 1.25,
            header_room: 0.8,
        },
        drive: Drive::Rear,
        limited_slip: true,
        gearbox: 2,
        final_drive: 3.07,
        engines: codes(&["R80"]),
        paint: 18,
        body: Body {
            length: 4.45,
            width: 1.92,
            front_overhang: 0.95,
            floor: 0.12,
            // A low, blunt nose, a long bonnet rising slowly to a raked windscreen, a
            // short rear deck and a tail cut off square.
            deck: vec![
                [0.0, 0.42],
                [0.07, 0.58],
                [0.5, 0.66],
                [1.2, 0.74],
                [1.95, 0.8],
                [2.6, 0.82],
                [3.3, 0.85],
                [4.33, 0.88],
                [4.45, 0.74],
            ],
            // The windscreen's base and top: no roof.
            cabin: vec![[1.95, 0.8, 1.0], [2.42, 1.12, 1.0]],
            nose: 0.76,
            tail: 0.9,
            tumblehome: 0.9,
            open_wheels: false,
            // The cockpit, from the windscreen to behind the seats.
            openings: vec![[1.95, 3.18, 0.66]],
            details: vec![
                // Hoops behind the seats.
                detail([3.12, 1.0, 0.36], [0.07, 0.3, 0.26], Tone::Dark),
                // Vents in the fenders behind the front wheels.
                detail([1.5, 0.6, 0.955], [0.3, 0.1, 0.02], Tone::Dark),
                detail([1.5, 0.48, 0.955], [0.3, 0.06, 0.02], Tone::Dark),
            ],
            door: [1.62, 2.88],
            bumpers: [0.36, 0.3],
            bumper_top: 0.42,
            // A long bulge down the middle of the bonnet, over the engine's intake.
            bulges: vec![[0.75, 1.88, 0.3, 0.05]],
            side_pipes: true,
        },
        eye: [2.68, 0.98, 0.36],
        insulation: 0.15,
        // Rare in traffic: about one car in fifty.
        traffic: 0.06,
        // A small trunk under the boot lid, over the rear axle, and the passenger seat's
        // place while it's out.
        hold: Hold {
            from: 2.4,
            left: -0.6,
            base: 0.2,
            areas: vec![
                area([10, 19], [1, 11], 2, 6, None),
                area([0, 7], [7, 12], 0, 6, Some("Passenger seat")),
            ],
        },
    }
}

fn area(along: [u8; 2], across: [u8; 2], floor: u8, ceiling: u8, seat: Option<&str>) -> Area {
    Area {
        along,
        across,
        floor,
        ceiling,
        seat: seat.map(String::from),
    }
}

#[cfg(test)]
mod tests {
    use crate::catalogue::{DEFAULT_CHASSIS, builtin};
    use crate::chassis::{CarBuild, Misfit};
    use crate::physics::engine::maps::EngineMaps;
    use crate::physics::engine::parts::Parts;
    use crate::wheels::{Rim, TireSize};

    fn family(code: &str) -> usize {
        builtin().find_engine(code).expect("an engine").0
    }

    #[test]
    fn factory_cars_are_complete() {
        let catalogue = builtin();
        assert_eq!(catalogue.chassis[DEFAULT_CHASSIS].name, "Vireo");
        for (index, made) in catalogue.chassis.iter().enumerate() {
            let build = CarBuild::factory(index);
            assert!(
                catalogue
                    .gearboxes_for(made)
                    .any(|gearbox| gearbox == made.gearbox)
            );
            assert!(!catalogue.paints[made.paint].shop);
            // The factory's wheels fit, on the sizes on offer.
            for (room, wheel) in made.room.iter().zip(&made.wheels) {
                assert_eq!(room.misfit(wheel), None, "{}", made.name);
                assert!(Rim::all().any(|rim| rim == wheel.rim), "{}", made.name);
                assert!(TireSize::all().any(|tire| tire == wheel.tire));
            }
            for (option, code) in made.engines.iter().enumerate() {
                // The factory's engines fit without a swap subframe or an adapter, with
                // their own exhaust manifold: their maps come baked with the game.
                let fit = made.fit(&catalogue, family(code)).expect("it fits");
                assert!(!fit.swap && !catalogue.needs_adapter(made.gearbox, family(code)));
                let parts = build.factory_engine(option);
                let hardware = parts.hardware();
                let free = Parts::factory(code).unwrap().hardware();
                assert_eq!(
                    hardware.exhaust.header_length, free.exhaust.header_length,
                    "{} with the {code}",
                    made.name
                );
                assert!(EngineMaps::stock(&hardware).is_some());
            }
            // The bay's place matches where the engine sits.
            let at = made.bay.engine_at / made.wheelbase;
            let place = &made.bay.place;
            assert_eq!(place.starts_with("front"), at < 0.3, "{}", made.name);
            assert_eq!(place == "rear", at > 1.0, "{}", made.name);
        }
    }

    /// The hot hatch with its bay `by` metres narrower (across the car).
    fn narrowed(by: f32) -> crate::chassis::Chassis {
        let mut made = builtin().chassis[DEFAULT_CHASSIS].clone();
        made.bay.width -= by;
        made
    }

    #[test]
    fn space_decides_what_fits() {
        let catalogue = builtin();
        let four = family("B16");
        let fit = catalogue.chassis[DEFAULT_CHASSIS].fit(&catalogue, four);
        assert!(!fit.expect("its own engine fits").swap);
        // Across the car, the gearbox sits on the end of the crank.
        assert_eq!(
            narrowed(0.1).fit(&catalogue, four),
            Err(Misfit(
                "4 cm too wide for the front, transverse bay".to_string()
            ))
        );
    }

    #[test]
    fn tight_bays_shorten_the_headers() {
        // A four filling the bay from front to back leaves room only for short
        // primaries, and its maps aren't the factory's any more: the manifold is part of
        // the cylinder side.
        let catalogue = builtin();
        let four = family("B16");
        let roomy = catalogue.chassis[DEFAULT_CHASSIS]
            .fit(&catalogue, four)
            .unwrap();
        let mut tight = builtin().chassis[DEFAULT_CHASSIS].clone();
        tight.bay.length = catalogue.families[four].width;
        let tight = tight.fit(&catalogue, four).unwrap();
        assert!(tight.header_room < roomy.header_room);
        let mut parts = Parts::factory("B16").unwrap();
        parts.header_room = tight.header_room;
        let hardware = parts.hardware();
        assert_eq!(hardware.exhaust.header_length, tight.header_room);
        assert!(EngineMaps::stock(&hardware).is_none());
    }

    #[test]
    fn the_roadster_is_a_heavy_v10_driving_its_rear_wheels() {
        use crate::car::CarSetup;
        use crate::chassis::{GearboxKind, Orientation};
        use crate::physics::drivetrain::Drive;
        let catalogue = builtin();
        let merrow = catalogue.find_chassis("Merrow").expect("the roadster");
        let setup = CarSetup::factory(merrow, 0);
        let full = setup.tank.litres;
        let (mass, front) = (setup.mass_with(full), setup.front_weight_with(full));
        let hardware = setup.engine.parts.hardware();
        println!(
            "Merrow: {mass:.0} kg full, {:.1} % front, {:.1} kg/kW",
            front * 100.0,
            mass / 305.0
        );
        assert!((1400.0..1500.0).contains(&mass), "{mass} kg");
        assert!((0.46..0.52).contains(&front), "{front} at the front");
        assert_eq!(hardware.name, "R80 8.0 V10");
        let made = &catalogue.chassis[merrow];
        assert_eq!(made.drive, Drive::Rear);
        assert_eq!(made.bay.orientation, Orientation::Longitudinal);
        assert!(setup.build.limited_slip);
        // Its gearbox takes the V10's torque, and only inline gearboxes go in.
        assert_eq!(setup.build.gearbox().kind, GearboxKind::Inline);
        assert!(setup.engine.gearbox_torque >= 640.0 && setup.engine.clutch_torque >= 650.0);
        let boxes: Vec<usize> = catalogue.gearboxes_for(made).collect();
        assert_eq!(boxes, [made.gearbox]);
        // Wider tires at the back, on 17" rims.
        let [f, r] = setup.build.wheels;
        assert_eq!(
            (f.tire.label(), r.tire.label()),
            ("275/40 R17".into(), "335/35 R17".into())
        );
        // The V10 goes into the roadster, not into the hatch.
        let vee = family("R80");
        assert!(
            catalogue.chassis[DEFAULT_CHASSIS]
                .fit(&catalogue, vee)
                .is_err()
        );
        assert!(made.fit(&catalogue, vee).is_ok());
        // Rare in traffic: one car in 40 to 60.
        let all: f32 = catalogue.chassis.iter().map(|c| c.traffic).sum();
        let one_in = all / made.traffic;
        assert!(
            (40.0..60.0).contains(&one_in),
            "one traffic car in {one_in}"
        );
    }

    #[test]
    fn gearboxes_bolt_to_their_engines() {
        let catalogue = builtin();
        let hatch = &catalogue.chassis[DEFAULT_CHASSIS];
        let boxes: Vec<&str> = (catalogue.gearboxes_for(hatch))
            .map(|g| catalogue.gearboxes[g].name.as_str())
            .collect();
        assert_eq!(
            boxes,
            ["5-speed (transverse)", "6-speed close-ratio (transverse)"]
        );
        // The hatch's own gearbox takes its engine; the other needs a plate.
        assert!(!catalogue.needs_adapter(hatch.gearbox, family("B16")));
        assert!(catalogue.needs_adapter(1 - hatch.gearbox, family("B16")));
    }
}
