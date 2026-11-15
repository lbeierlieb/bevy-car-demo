//! Bakes a square of real-world terrain, land use, roads and buildings into the binary
//! format that car-sim embeds (see `src/terrain.rs` in the game for the reader).
//!
//! Usage: `cargo run --release -- <lat> <lon> <size_m> <out_file>`
//!
//! Elevation is the 1 m lidar terrain model (DGM1) of the Bavarian surveying
//! administration, so this only works inside Bavaria. Everything else comes from the
//! OpenStreetMap API. The map frame is aligned with UTM zone 32 (x east, z south), the
//! grid the DGM1 tiles use.

use std::collections::HashMap;
use std::io::Cursor;

use quick_xml::events::Event;

/// Terrain grid spacing in metres.
const CELL: f64 = 2.0;
const USER_AGENT: &str = "car-sim-bake/0.2 (Bevy car simulation demo)";
/// Bridges: lidar shows the ground under them, so their decks are rebuilt as straight
/// ramps between the ends, with the terrain raised beside them over `BLEND` metres.
const ROAD_STEP: f64 = 2.0;
const SHOULDER: f64 = 1.0;
const BLEND: f64 = 6.0;

type Error = Box<dyn std::error::Error>;
type Point = (f64, f64);
type Tags = HashMap<String, String>;

/// Land use classes; where polygons overlap, the higher priority wins.
mod class {
    pub const RESIDENTIAL: u8 = 1;
    /// Four colours of fields (2..=5), chosen per field.
    pub const FARMLAND: u8 = 2;
    pub const MEADOW: u8 = 6;
    pub const SCRUB: u8 = 7;
    pub const FOREST: u8 = 8;
    pub const WATER: u8 = 9;
    pub const PAVED: u8 = 10;

    /// All field colours rank the same.
    pub fn priority(class: u8) -> u8 {
        if (FARMLAND..MEADOW).contains(&class) {
            FARMLAND
        } else {
            class
        }
    }
}

fn land_class(tags: &Tags) -> Option<u8> {
    let tag = |k: &str| tags.get(k).map(String::as_str);
    if tag("amenity") == Some("parking") || tags.contains_key("area:highway") {
        return Some(class::PAVED);
    }
    if tags.contains_key("highway") && tag("area") == Some("yes") {
        return Some(class::PAVED);
    }
    Some(match (tag("landuse"), tag("natural"), tag("leisure")) {
        (Some("forest"), _, _) | (_, Some("wood"), _) => class::FOREST,
        (Some("basin" | "reservoir"), _, _) | (_, Some("water"), _) => class::WATER,
        (_, Some("scrub" | "heath"), _) => class::SCRUB,
        (Some("meadow" | "grass" | "cemetery" | "village_green"), _, _)
        | (_, Some("grassland"), _)
        | (_, _, Some("pitch" | "park" | "garden" | "playground")) => class::MEADOW,
        (Some("farmland" | "orchard" | "vineyard"), _, _) => class::FARMLAND,
        (
            Some(
                "residential" | "farmyard" | "allotments" | "industrial" | "commercial"
                | "construction" | "retail",
            ),
            _,
            _,
        ) => class::RESIDENTIAL,
        _ => return None,
    })
}

/// WGS84/ETRS89 latitude and longitude to UTM zone 32N metres (Krüger series, which is
/// millimetre-accurate within the zone).
fn utm32(lat: f64, lon: f64) -> Point {
    let (a, f): (f64, f64) = (6_378_137.0, 1.0 / 298.257_222_101);
    let n = f / (2.0 - f);
    let big_a = a / (1.0 + n) * (1.0 + n * n / 4.0 + n.powi(4) / 64.0);
    let alpha = [
        n / 2.0 - 2.0 * n * n / 3.0 + 5.0 * n.powi(3) / 16.0,
        13.0 * n * n / 48.0 - 3.0 * n.powi(3) / 5.0,
        61.0 * n.powi(3) / 240.0,
    ];
    let (phi, dlambda) = (lat.to_radians(), (lon - 9.0).to_radians());
    let e = 2.0 * n.sqrt() / (1.0 + n);
    let t = (phi.sin().atanh() - e * (e * phi.sin()).atanh()).sinh();
    let xi = (t / dlambda.cos()).atan();
    let eta = (dlambda.sin() / (1.0 + t * t).sqrt()).atanh();
    let (mut east, mut north) = (eta, xi);
    for (j, a) in alpha.iter().enumerate() {
        let k = 2.0 * (j + 1) as f64;
        east += a * (k * xi).cos() * (k * eta).sinh();
        north += a * (k * xi).sin() * (k * eta).cosh();
    }
    (500_000.0 + 0.9996 * big_a * east, 0.9996 * big_a * north)
}

/// The map frame: UTM shifted to the map centre, with z pointing south.
struct Frame {
    east: f64,
    north: f64,
}

impl Frame {
    fn to_local(&self, lat: f64, lon: f64) -> Point {
        let (e, n) = utm32(lat, lon);
        (e - self.east, self.north - n)
    }
}

fn get(url: &str) -> Result<Vec<u8>, Error> {
    eprintln!("GET {url}");
    let mut response = ureq::get(url).header("User-Agent", USER_AGENT).call()?;
    Ok(response
        .body_mut()
        .with_config()
        .limit(100 << 20)
        .read_to_vec()?)
}

/// DGM1 lidar terrain: 1 km tiles of 1000×1000 one-metre pixels, named by their
/// south-west corner in UTM kilometres.
struct Dgm1 {
    tiles: HashMap<(i64, i64), Vec<f32>>,
}

impl Dgm1 {
    fn fetch(east: (f64, f64), north: (f64, f64)) -> Result<Self, Error> {
        let km = |v: f64| (v / 1000.0).floor() as i64;
        let mut tiles = HashMap::new();
        for e in km(east.0)..=km(east.1) {
            for n in km(north.0)..=km(north.1) {
                let url = format!("https://download1.bayernwolke.de/a/dgm/dgm1/{e}_{n}.tif");
                let mut decoder = tiff::decoder::Decoder::new(Cursor::new(get(&url)?))?;
                let tiff::decoder::DecodingResult::F32(heights) = decoder.read_image()? else {
                    return Err(format!("{url}: expected float32 heights").into());
                };
                tiles.insert((e, n), heights);
            }
        }
        Ok(Self { tiles })
    }

    /// Height of the one-metre pixel whose south-west corner is (east, north).
    fn pixel(&self, east: i64, north: i64) -> f64 {
        let tile = &self.tiles[&(east.div_euclid(1000), north.div_euclid(1000))];
        // Rows run from north to south.
        let (col, row) = (east.rem_euclid(1000), 999 - north.rem_euclid(1000));
        tile[(row * 1000 + col) as usize] as f64
    }

    /// Bilinear between pixel centres.
    fn sample(&self, east: f64, north: f64) -> f64 {
        let (x, y) = (east - 0.5, north - 0.5);
        let (ix, iy) = (x.floor() as i64, y.floor() as i64);
        let (fx, fy) = (x - ix as f64, y - iy as f64);
        let row = |iy| self.pixel(ix, iy) * (1.0 - fx) + self.pixel(ix + 1, iy) * fx;
        row(iy) * (1.0 - fy) + row(iy + 1) * fy
    }
}

#[derive(Clone, Copy)]
enum RoadKind {
    Paved = 0,
    Unpaved = 1,
    Water = 2,
}

struct Road {
    kind: RoadKind,
    width: f64,
    bridge: bool,
    points: Vec<Point>,
}

/// Kind and width of a drivable road or a waterway; `None` for anything else.
fn road_style(tags: &Tags) -> Option<(RoadKind, f64)> {
    let tag = |k: &str| tags.get(k).map(String::as_str);
    if let Some(waterway) = tag("waterway") {
        let width = match waterway {
            "river" => 6.0,
            "stream" | "canal" => 1.5,
            "drain" | "ditch" => 0.8,
            _ => return None,
        };
        return Some((RoadKind::Water, width));
    }
    let default_width = match (tag("highway")?, tag("service")) {
        ("motorway" | "trunk" | "primary", _) => 7.0,
        ("secondary", _) => 6.5,
        ("tertiary" | "motorway_link" | "trunk_link" | "primary_link" | "secondary_link", _) => 6.0,
        ("residential" | "tertiary_link", _) => 5.0,
        ("unclassified" | "living_street", _) => 4.5,
        ("service", Some("parking_aisle")) => 5.5,
        ("service", Some("driveway")) => 3.0,
        ("service", _) => 3.5,
        ("track", _) => 2.8,
        _ => return None,
    };
    // Prefer explicit tags: `width` in metres (possibly with a unit), else lanes of 3 m.
    let number = |k: &str| tag(k).and_then(|v| v.trim_end_matches('m').trim().parse::<f64>().ok());
    let width = number("width")
        .or_else(|| number("lanes").map(|lanes| lanes * 3.0 + 0.5))
        .unwrap_or(default_width)
        .clamp(2.0, 15.0);
    let paved = match tag("surface") {
        Some(
            "asphalt" | "concrete" | "concrete:plates" | "concrete:lanes" | "paving_stones"
            | "sett",
        ) => true,
        Some(_) => false,
        None => tag("highway") != Some("track"),
    };
    Some((
        if paved {
            RoadKind::Paved
        } else {
            RoadKind::Unpaved
        },
        width,
    ))
}

struct Building {
    /// `building:levels` from OSM, or 0 if not tagged.
    levels: f64,
    /// Outline in local metres, not closed.
    points: Vec<Point>,
}

struct Area {
    class: u8,
    /// Closed rings (outer and inner alike; filled with the even-odd rule).
    rings: Vec<Vec<Point>>,
}

/// Parsed OSM XML: nodes, and ways and relations with their member refs and tags.
#[derive(Default)]
struct Osm {
    nodes: HashMap<u64, Point>,
    ways: HashMap<u64, (Vec<u64>, Tags)>,
    relations: Vec<(u64, Vec<u64>, Tags)>,
}

impl Osm {
    fn parse(&mut self, xml: &str, frame: &Frame) -> Result<(), Error> {
        // The element currently being read: (is relation, id, member refs, tags).
        let mut current: Option<(bool, u64, Vec<u64>, Tags)> = None;
        let mut reader = quick_xml::Reader::from_str(xml);
        loop {
            let event = reader.read_event()?;
            let (Event::Start(e) | Event::Empty(e)) = &event else {
                match event {
                    Event::End(e) if matches!(e.name().as_ref(), "way" | "relation") => {
                        if let Some((relation, id, refs, tags)) = current.take() {
                            if relation {
                                self.relations.push((id, refs, tags));
                            } else {
                                self.ways.insert(id, (refs, tags));
                            }
                        }
                    }
                    Event::Eof => break,
                    _ => {}
                }
                continue;
            };
            let attrs: HashMap<String, String> = e
                .attributes()
                .filter_map(Result::ok)
                .map(|a| (a.key.as_ref().to_owned(), a.value.into_owned()))
                .collect();
            let attr = |key: &str| attrs.get(key).cloned().unwrap_or_default();
            match e.name().as_ref() {
                "node" => {
                    let (lat, lon) = (attr("lat").parse::<f64>()?, attr("lon").parse::<f64>()?);
                    self.nodes
                        .insert(attr("id").parse()?, frame.to_local(lat, lon));
                }
                name @ ("way" | "relation") => {
                    current = Some((
                        name == "relation",
                        attr("id").parse()?,
                        Vec::new(),
                        HashMap::new(),
                    ))
                }
                "nd" => {
                    if let Some((_, _, refs, _)) = &mut current {
                        refs.push(attr("ref").parse()?);
                    }
                }
                "member" if attr("type") == "way" => {
                    if let Some((_, _, refs, _)) = &mut current {
                        refs.push(attr("ref").parse()?);
                    }
                }
                "tag" => {
                    if let Some((_, _, _, tags)) = &mut current {
                        tags.insert(attr("k"), attr("v"));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn points(&self, refs: &[u64]) -> Vec<Point> {
        refs.iter()
            .filter_map(|r| self.nodes.get(r).copied())
            .collect()
    }

    /// Joins the member ways of a multipolygon into closed rings; unclosable pieces are dropped.
    fn rings(&self, members: &[u64]) -> Vec<Vec<Point>> {
        let mut pieces: Vec<Vec<u64>> = members
            .iter()
            .filter_map(|w| self.ways.get(w))
            .map(|w| w.0.clone())
            .collect();
        let mut rings = Vec::new();
        while let Some(mut ring) = pieces.pop() {
            while ring.first() != ring.last() {
                let end = *ring.last().unwrap();
                let Some(i) = pieces
                    .iter()
                    .position(|p| p.first() == Some(&end) || p.last() == Some(&end))
                else {
                    break;
                };
                let mut next = pieces.swap_remove(i);
                if next.first() != Some(&end) {
                    next.reverse();
                }
                ring.extend(&next[1..]);
            }
            if ring.len() >= 4 && ring.first() == ring.last() {
                rings.push(self.points(&ring));
            }
        }
        rings
    }
}

fn fetch_osm(frame: &Frame, center: Point, half_size: f64) -> Result<Osm, Error> {
    // Lat/lon box with a margin; everything is clipped to the map in local metres later.
    let dlat = (half_size * 1.1 / 6_371_000.0).to_degrees();
    let dlon = dlat / center.0.to_radians().cos();
    let (lat, lon) = center;
    let mut osm = Osm::default();
    let url = format!(
        "https://api.openstreetmap.org/api/0.6/map?bbox={:.6},{:.6},{:.6},{:.6}",
        lon - dlon,
        lat - dlat,
        lon + dlon,
        lat + dlat
    );
    osm.parse(&String::from_utf8(get(&url)?)?, frame)?;
    // The map call omits multipolygon member ways outside the box: fetch those in full.
    let multipolygons: Vec<u64> = osm
        .relations
        .iter()
        .filter(|(_, _, tags)| {
            tags.get("type").is_some_and(|t| t == "multipolygon") && land_class(tags).is_some()
        })
        .map(|(id, _, _)| *id)
        .collect();
    for id in multipolygons {
        let url = format!("https://api.openstreetmap.org/api/0.6/relation/{id}/full");
        let mut full = Osm::default();
        full.parse(&String::from_utf8(get(&url)?)?, frame)?;
        osm.nodes.extend(full.nodes);
        for (id, way) in full.ways {
            osm.ways.entry(id).or_insert(way);
        }
    }
    Ok(osm)
}

/// Resamples a polyline at roughly `ROAD_STEP` spacing, keeping the original vertices.
fn resample(points: &[Point]) -> Vec<Point> {
    let mut out = vec![points[0]];
    for pair in points.windows(2) {
        let ((x0, z0), (x1, z1)) = (pair[0], pair[1]);
        let steps = ((x1 - x0).hypot(z1 - z0) / ROAD_STEP).ceil().max(1.0) as usize;
        for i in 1..=steps {
            let t = i as f64 / steps as f64;
            out.push((x0 + (x1 - x0) * t, z0 + (z1 - z0) * t));
        }
    }
    out
}

fn smoothstep(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Even-odd point-in-polygon test over all rings.
fn inside_rings(rings: &[Vec<Point>], (px, pz): Point) -> bool {
    let mut inside = false;
    for ring in rings {
        for (i, &(x0, z0)) in ring.iter().enumerate() {
            let (x1, z1) = ring[(i + 1) % ring.len()];
            if (z0 > pz) != (z1 > pz) && px < x0 + (pz - z0) / (z1 - z0) * (x1 - x0) {
                inside = !inside;
            }
        }
    }
    inside
}

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().collect();
    let [_, lat, lon, size, out] = args.as_slice() else {
        return Err("usage: bake <lat> <lon> <size_m> <out_file>".into());
    };
    let center = (lat.parse::<f64>()?, lon.parse::<f64>()?);
    let (east, north) = utm32(center.0, center.1);
    // Centre on a whole metre so grid vertices line up with lidar pixel edges.
    let frame = Frame {
        east: east.round(),
        north: north.round(),
    };
    let cells = (size.parse::<f64>()? / CELL).round() as usize;
    let half_size = cells as f64 * CELL / 2.0;
    let n = cells + 1;
    let grid = |i: usize| i as f64 * CELL - half_size;
    let to_index = |v: f64| ((v + half_size) / CELL).clamp(0.0, cells as f64) as usize;

    // Terrain: lidar heights, box-filtered over 3×3 metres to smooth out tiny bumps.
    let margin = 3.0;
    let dgm = Dgm1::fetch(
        (
            frame.east - half_size - margin,
            frame.east + half_size + margin,
        ),
        (
            frame.north - half_size - margin,
            frame.north + half_size + margin,
        ),
    )?;
    let height_at = |x: f64, z: f64| {
        let (e, n) = (frame.east + x, frame.north - z);
        let mut sum = 0.0;
        for dx in [-1.0, 0.0, 1.0] {
            for dz in [-1.0, 0.0, 1.0] {
                sum += dgm.sample(e + dx, n + dz);
            }
        }
        sum / 9.0
    };
    let mut heights: Vec<f64> = (0..n * n)
        .map(|i| height_at(grid(i % n), grid(i / n)))
        .collect();

    let osm = fetch_osm(&frame, center, half_size)?;
    let on_map = |&(x, z): &Point| x.abs() < half_size && z.abs() < half_size;

    // Roads, waterways, buildings and land use areas.
    let mut roads = Vec::new();
    let mut buildings = Vec::new();
    let mut areas = Vec::new();
    for (refs, tags) in osm.ways.values() {
        let mut points = osm.points(refs);
        if tags.contains_key("building") {
            points.pop();
            if points.len() >= 3 && points.iter().all(on_map) {
                let levels = tags
                    .get("building:levels")
                    .and_then(|l| l.parse().ok())
                    .unwrap_or(0.0);
                buildings.push(Building { levels, points });
            }
        } else if let Some(class) = land_class(tags).filter(|_| refs.first() == refs.last()) {
            areas.push(Area {
                class,
                rings: vec![points],
            });
        } else if let Some((kind, width)) = road_style(tags) {
            let bridge = tags.get("bridge").is_some_and(|b| b != "no");
            // Split the way wherever it leaves the map.
            for run in points.split(|p| !on_map(p)).filter(|run| run.len() >= 2) {
                roads.push(Road {
                    kind,
                    width,
                    bridge,
                    points: run.to_vec(),
                });
            }
        }
    }
    for (_, members, tags) in &osm.relations {
        if tags.get("type").is_some_and(|t| t == "multipolygon")
            && let Some(class) = land_class(tags)
        {
            areas.push(Area {
                class,
                rings: osm.rings(members),
            });
        }
    }

    // Rasterise land use per grid cell (tested at the cell centre).
    let mut classes = vec![0u8; cells * cells];
    for (i, area) in areas.iter().enumerate() {
        // Alternate field colours so neighbouring fields read as a patchwork.
        let class = if area.class == class::FARMLAND {
            class::FARMLAND + (i % 4) as u8
        } else {
            area.class
        };
        let all = area.rings.iter().flatten();
        let (x0, x1) = all.clone().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
            (lo.min(p.0), hi.max(p.0))
        });
        let (z0, z1) = all.fold((f64::MAX, f64::MIN), |(lo, hi), p| {
            (lo.min(p.1), hi.max(p.1))
        });
        for gz in to_index(z0)..to_index(z1).min(cells) {
            for gx in to_index(x0)..to_index(x1).min(cells) {
                let slot = &mut classes[gz * cells + gx];
                let centre = (grid(gx) + CELL / 2.0, grid(gz) + CELL / 2.0);
                if class::priority(class) > class::priority(*slot)
                    && inside_rings(&area.rings, centre)
                {
                    *slot = class;
                }
            }
        }
    }

    // Bridges: straight decks between the end heights, terrain raised up to them.
    let mut nearest = vec![(f64::INFINITY, 0.0); n * n];
    let grid_height = |x: f64, z: f64| heights[to_index(z) * n + to_index(x)];
    for road in roads.iter_mut().filter(|r| r.bridge) {
        road.points = resample(&road.points);
        let (start, end) = (road.points[0], *road.points.last().unwrap());
        let (h0, h1) = (grid_height(start.0, start.1), grid_height(end.0, end.1));
        let count = road.points.len();
        let deck: Vec<f64> = (0..count)
            .map(|i| h0 + (h1 - h0) * i as f64 / (count - 1) as f64)
            .collect();
        let reach = road.width / 2.0 + SHOULDER + BLEND;
        for (i, pair) in road.points.windows(2).enumerate() {
            let ((x0, z0), (x1, z1)) = (pair[0], pair[1]);
            let (dx, dz) = (x1 - x0, z1 - z0);
            let len2 = (dx * dx + dz * dz).max(1e-9);
            for gz in to_index(z0.min(z1) - reach)..=to_index(z0.max(z1) + reach) {
                for gx in to_index(x0.min(x1) - reach)..=to_index(x0.max(x1) + reach) {
                    let (px, pz) = (grid(gx), grid(gz));
                    let t = (((px - x0) * dx + (pz - z0) * dz) / len2).clamp(0.0, 1.0);
                    let edge = (px - x0 - dx * t).hypot(pz - z0 - dz * t) - road.width / 2.0;
                    let slot = &mut nearest[gz * n + gx];
                    if edge < slot.0 {
                        *slot = (edge, deck[i] * (1.0 - t) + deck[i + 1] * t);
                    }
                }
            }
        }
    }
    for (h, &(edge, deck)) in heights.iter_mut().zip(&nearest) {
        let w = 1.0 - smoothstep((edge - SHOULDER) / BLEND);
        // Only ever raise the ground (stream valleys under the deck get filled).
        *h += (deck - *h).max(0.0) * w;
    }

    // Write (all little-endian): b"CSTR", version u32, cells u32, cell size f32,
    // (cells + 1)² heights f32 relative to the lowest point (row-major, rows along z),
    // cells² land use classes u8 (row-major),
    // road count u32, then per road: kind u8 (0 paved, 1 unpaved, 2 water), width f32,
    // point count u32, points (x, z) f32,
    // building count u32, then per building: levels f32 (0: unknown), point count u32,
    // outline points (x, z) f32.
    let base = heights.iter().copied().fold(f64::INFINITY, f64::min);
    let mut bytes = Vec::new();
    let mut put = |v: &[u8]| bytes.extend_from_slice(v);
    put(b"CSTR");
    put(&3u32.to_le_bytes());
    put(&(cells as u32).to_le_bytes());
    put(&(CELL as f32).to_le_bytes());
    for h in &heights {
        put(&((h - base) as f32).to_le_bytes());
    }
    put(&classes);
    let polyline = |put: &mut dyn FnMut(&[u8]), points: &[Point]| {
        put(&(points.len() as u32).to_le_bytes());
        for &(x, z) in points {
            put(&(x as f32).to_le_bytes());
            put(&(z as f32).to_le_bytes());
        }
    };
    put(&(roads.len() as u32).to_le_bytes());
    for road in &roads {
        put(&[road.kind as u8]);
        put(&(road.width as f32).to_le_bytes());
        polyline(&mut put, &road.points);
    }
    put(&(buildings.len() as u32).to_le_bytes());
    for building in &buildings {
        put(&(building.levels as f32).to_le_bytes());
        polyline(&mut put, &building.points);
    }
    std::fs::write(out, &bytes)?;
    eprintln!(
        "wrote {out}: {cells}x{cells} cells of {CELL} m, {} roads, {} buildings, {} areas, relief {:.1} m, {} KiB",
        roads.len(),
        buildings.len(),
        areas.len(),
        heights.iter().copied().fold(f64::NEG_INFINITY, f64::max) - base,
        bytes.len() / 1024
    );
    Ok(())
}
