//! Bakes a rectangle of real-world terrain, land use, roads and buildings into the binary
//! format that car-sim embeds (see `src/terrain.rs` in the game for the reader).
//!
//! Usage (see `USAGE`): `bake import …` adds what OpenStreetMap has around a map rectangle
//! to the map file (`osm.rs`), which is then ours and edited in JOSM; `bake <map.osm> …`
//! bakes from the map file, with the spawn heading in compass degrees (0 north, 90 east).
//!
//! Elevation is the 1 m lidar terrain model (DGM1) of the Bavarian surveying
//! administration, so this only works inside Bavaria. OpenStreetMap is read via the
//! Overpass API. Downloads are cached in `tools/bake/cache`; delete it to fetch fresh
//! data. The map frame is aligned with UTM zone 32 (x east, z south), the grid the DGM1
//! tiles use.

mod clearance;
mod fences;
#[path = "../../../src/osm.rs"]
mod osm;
mod profile;
mod roads;
mod terrain;

use std::collections::HashMap;
use std::io::Cursor;

use osm::{Frame, Osm, Tags};
use roads::{Feature, Grid, Road, RoadKind};

/// Terrain grid spacing in metres, at full detail.
const CELL: f64 = 2.0;
/// Chunk side in cells (96 m). Each chunk is stored at one of the steps below, which all
/// divide it. The map is a whole number of chunks along each axis.
const CHUNK_CELLS: usize = 48;
/// Chunk resolutions as steps in cells: 2 m and 8 m.
const STEP_DETAIL: usize = 1;
const STEP_EMPTY: usize = 4;
/// Width of the raised rim at the map edge.
const RIM: f64 = 30.0;
const USER_AGENT: &str = "car-sim-bake/0.3 (Bevy car simulation demo)";
const CACHE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/cache");
const OVERPASS: &str = "https://overpass-api.de/api/interpreter";

type Error = Box<dyn std::error::Error>;
type Point = (f64, f64);

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

/// Something in the map to fix: where, a short label and what it is.
struct Marker {
    at: Point,
    name: String,
    desc: String,
}

impl Marker {
    /// A road still running into a building.
    fn road_in_building(
        c: &clearance::Conflict,
        roads: &[Road],
        road_ways: &[i64],
        buildings: &[Building],
    ) -> Self {
        let road = &roads[c.road];
        let mut desc = format!(
            "way {} ({:.1} m wide) {:.1} m into building {}",
            road_ways[c.road], road.width, c.overlap, buildings[c.building].way
        );
        if let Some(gap) = c.gap {
            desc += &format!(", {gap:.1} m between the houses");
        }
        if c.junction {
            desc += ", near a junction";
        }
        Self {
            at: c.at,
            name: format!("{:.1} m", c.overlap),
            desc,
        }
    }
}

/// Writes the markers as GPX waypoints (JOSM opens them as a marker layer).
fn write_markers(path: &str, markers: &[Marker], frame: &Frame) -> Result<(), Error> {
    let mut gpx = String::from(
        "<?xml version='1.0' encoding='UTF-8'?>\n<gpx version='1.1' creator='bake' xmlns='http://www.topografix.com/GPX/1/1'>\n",
    );
    for m in markers {
        let (lat, lon) = frame.to_geo(m.at);
        gpx += &format!(
            "  <wpt lat='{lat:.7}' lon='{lon:.7}'><name>{}</name><desc>{}</desc></wpt>\n",
            m.name, m.desc
        );
    }
    gpx += "</gpx>\n";
    std::fs::write(path, gpx)?;
    Ok(())
}

/// The cached file `name`, or else the result of `fetch`, which is then cached.
fn cached(name: &str, fetch: impl FnOnce() -> Result<Vec<u8>, Error>) -> Result<Vec<u8>, Error> {
    let path = std::path::Path::new(CACHE).join(name);
    if let Ok(bytes) = std::fs::read(&path) {
        eprintln!("cached {name}");
        return Ok(bytes);
    }
    let bytes = fetch()?;
    std::fs::create_dir_all(CACHE)?;
    std::fs::write(&path, &bytes)?;
    Ok(bytes)
}

fn read_body(mut response: ureq::http::Response<ureq::Body>) -> Result<Vec<u8>, Error> {
    Ok(response
        .body_mut()
        .with_config()
        .limit(1 << 30)
        .read_to_vec()?)
}

fn get(url: &str) -> Result<Vec<u8>, Error> {
    eprintln!("GET {url}");
    read_body(ureq::get(url).header("User-Agent", USER_AGENT).call()?)
}

/// DGM1 lidar terrain: 1 km tiles of 1000×1000 one-metre pixels, named by their
/// south-west corner in UTM kilometres.
struct Dgm1 {
    /// The south-west tile's name, and the tiles along north.
    first: (i64, i64),
    rows: i64,
    /// Column-major: east, then north.
    tiles: Vec<Vec<f32>>,
}

impl Dgm1 {
    fn fetch(east: (f64, f64), north: (f64, f64)) -> Result<Self, Error> {
        let km = |v: f64| (v / 1000.0).floor() as i64;
        let mut tiles = Vec::new();
        for e in km(east.0)..=km(east.1) {
            for n in km(north.0)..=km(north.1) {
                let url = format!("https://download1.bayernwolke.de/a/dgm/dgm1/{e}_{n}.tif");
                let tile = cached(&format!("dgm1_{e}_{n}.tif"), || get(&url))?;
                let mut decoder = tiff::decoder::Decoder::new(Cursor::new(tile))?;
                let tiff::decoder::DecodingResult::F32(heights) = decoder.read_image()? else {
                    return Err(format!("{url}: expected float32 heights").into());
                };
                tiles.push(heights);
            }
        }
        Ok(Self {
            first: (km(east.0), km(north.0)),
            rows: km(north.1) - km(north.0) + 1,
            tiles,
        })
    }

    /// Height of the one-metre pixel whose south-west corner is (east, north).
    fn pixel(&self, east: i64, north: i64) -> f64 {
        let (e, n) = (
            east.div_euclid(1000) - self.first.0,
            north.div_euclid(1000) - self.first.1,
        );
        let tile = &self.tiles[(e * self.rows + n) as usize];
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
    let kind = match tag("surface").map(|s| s.split(':').next().unwrap_or(s)) {
        Some("asphalt" | "concrete" | "paved" | "chipseal" | "metal") => RoadKind::Asphalt,
        Some(
            "paving_stones" | "sett" | "cobblestone" | "unhewn_cobblestone" | "grass_paver"
            | "bricks",
        ) => RoadKind::Cobbles,
        Some("gravel" | "fine_gravel" | "compacted" | "pebblestone" | "unpaved" | "shells") => {
            RoadKind::Gravel
        }
        Some("ground" | "dirt" | "earth" | "grass" | "mud" | "sand" | "wood" | "woodchips") => {
            RoadKind::Dirt
        }
        // Untagged (or unknown): tracks by their grade, other roads asphalt.
        _ if tag("highway") == Some("track") => match tag("tracktype") {
            Some("grade1") => RoadKind::Asphalt,
            Some("grade4" | "grade5") => RoadKind::Dirt,
            _ => RoadKind::Gravel,
        },
        _ => RoadKind::Asphalt,
    };
    Some((kind, width))
}

struct Building {
    way: i64,
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

/// A way's points in local metres (missing or deleted nodes are skipped).
fn points(osm: &Osm, refs: &[i64], frame: &Frame) -> Vec<Point> {
    refs.iter()
        .filter_map(|r| osm.nodes.get(r).filter(|n| !n.meta.deleted()))
        .map(|n| frame.to_local(n.lat, n.lon))
        .collect()
}

/// Joins the member ways of a multipolygon into closed rings; unclosable pieces are dropped.
fn rings(osm: &Osm, members: &[osm::Member], frame: &Frame) -> Vec<Vec<Point>> {
    let mut pieces: Vec<Vec<i64>> = members
        .iter()
        .filter(|m| m.kind == "way")
        .filter_map(|m| osm.ways.get(&m.id))
        .filter(|w| !w.meta.deleted())
        .map(|w| w.refs.clone())
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
            rings.push(points(osm, &ring, frame));
        }
    }
    rings
}

/// Everything the map uses in a lat/lon box around the map, with one Overpass query: the
/// ways with tags that roads, buildings or land use read, the multipolygon relations, and
/// (recursively) all their member ways and nodes, also those outside the box (the bake
/// clips to the map in local metres). Tags the bake doesn't need are dropped.
fn fetch_osm(center: Point, (half_x, half_z): Point) -> Result<Osm, Error> {
    let degrees = |m: f64| (m * 1.1 / 6_371_000.0).to_degrees();
    let (dlat, dlon) = (
        degrees(half_z),
        degrees(half_x) / center.0.to_radians().cos(),
    );
    let (lat, lon) = center;
    let bbox = format!(
        "{:.6},{:.6},{:.6},{:.6}",
        lat - dlat,
        lon - dlon,
        lat + dlat,
        lon + dlon
    );
    let ways: String = [
        "highway",
        "building",
        "waterway",
        "landuse",
        "natural",
        "leisure",
        "amenity",
        "area:highway",
        "barrier",
    ]
    .iter()
    .map(|key| format!("way[\"{key}\"]({bbox});"))
    .collect();
    let query = format!(
        "[out:xml][timeout:600];\
         ({ways}relation[\"type\"=\"multipolygon\"]({bbox}););(._;>;);out body;"
    );
    // Named by the query too, so asking for more fetches again (FNV-1a).
    let hash = query.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    });
    let xml = cached(&format!("osm_{bbox}_{:08x}.xml", hash as u32), || {
        // The public server answers 429, 502 or 504 when it's busy: wait and retry.
        for attempt in 1.. {
            eprintln!("POST {OVERPASS} ({bbox})");
            match ureq::post(OVERPASS)
                .header("User-Agent", USER_AGENT)
                .send_form([("data", query.as_str())])
            {
                Ok(response) => return read_body(response),
                Err(ureq::Error::StatusCode(code @ (429 | 502 | 504))) if attempt < 5 => {
                    eprintln!("  busy ({code}), retrying in a minute");
                    std::thread::sleep(std::time::Duration::from_secs(60));
                }
                Err(e) => return Err(e.into()),
            }
        }
        unreachable!()
    })?;
    let mut osm = Osm::parse(&String::from_utf8(xml)?)?;
    osm.strip_tags();
    osm.bounds
        .push([lat - dlat, lon - dlon, lat + dlat, lon + dlon]);
    Ok(osm)
}

const USAGE: &str = "usage:
  bake import <lat> <lon> <width_m>[x<height_m>] <map.osm>
  bake <map.osm> <lat> <lon> <width_m>[x<height_m>] <out_file> [<spawn_lat> <spawn_lon> [<spawn_heading>]]";

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [command, lat, lon, size, map] if command == "import" => import(lat, lon, size, map),
        [map, lat, lon, size, out, spawn @ ..] if spawn.len() <= 3 && spawn.len() != 1 => {
            bake(map, lat, lon, size, out, spawn)
        }
        _ => Err(USAGE.into()),
    }
}

/// The map's centre, and its size in whole chunks along x and z.
fn extent(lat: &str, lon: &str, size: &str) -> Result<(Point, usize, usize), Error> {
    let (width, height) = match size.split_once('x') {
        Some((w, h)) => (w.parse::<f64>()?, h.parse::<f64>()?),
        None => (size.parse()?, size.parse()?),
    };
    let chunk_count = |m: f64| (m / (CELL * CHUNK_CELLS as f64)).round().max(1.0) as usize;
    Ok((
        (lat.parse()?, lon.parse()?),
        chunk_count(width),
        chunk_count(height),
    ))
}

/// Adds what OpenStreetMap has in and around a map of that size to the map file (made if
/// it doesn't exist), leaving everything already in it as it is.
fn import(lat: &str, lon: &str, size: &str, map: &str) -> Result<(), Error> {
    let (center, chunks_x, chunks_z) = extent(lat, lon, size)?;
    let half = |chunks: usize| (chunks * CHUNK_CELLS) as f64 * CELL / 2.0;
    let fetched = fetch_osm(center, (half(chunks_x), half(chunks_z)))?;
    let mut osm = match std::fs::read_to_string(map) {
        Ok(text) => Osm::parse(&text)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Osm::default(),
        Err(e) => return Err(e.into()),
    };
    let added = osm.merge(fetched);
    std::fs::write(map, osm.write())?;
    eprintln!(
        "{map}: added {added} elements, now {} nodes, {} ways, {} relations",
        osm.nodes.len(),
        osm.ways.len(),
        osm.relations.len()
    );
    Ok(())
}

/// Bakes the map of that size around (lat, lon) from the map file and the lidar terrain.
fn bake(
    map: &str,
    lat: &str,
    lon: &str,
    size: &str,
    out: &str,
    spawn: &[String],
) -> Result<(), Error> {
    let spawn: Vec<f64> = spawn.iter().map(|v| v.parse()).collect::<Result<_, _>>()?;
    let (center, chunks_x, chunks_z) = extent(lat, lon, size)?;
    let frame = Frame::around(center.0, center.1);
    let (cells_x, cells_z) = (chunks_x * CHUNK_CELLS, chunks_z * CHUNK_CELLS);
    let (half_x, half_z) = (cells_x as f64 * CELL / 2.0, cells_z as f64 * CELL / 2.0);
    // Grid vertices per row, and conversions between vertex indices and local metres.
    let n = cells_x + 1;
    let grid_x = |i: usize| i as f64 * CELL - half_x;
    let grid_z = |i: usize| i as f64 * CELL - half_z;
    let index_x = |v: f64| ((v + half_x) / CELL).clamp(0.0, cells_x as f64) as usize;
    let index_z = |v: f64| ((v + half_z) / CELL).clamp(0.0, cells_z as f64) as usize;

    // Terrain: lidar heights, box-filtered over 3×3 metres to smooth out tiny bumps.
    let margin = 3.0;
    let dgm = Dgm1::fetch(
        (frame.east - half_x - margin, frame.east + half_x + margin),
        (frame.north - half_z - margin, frame.north + half_z + margin),
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
    let mut heights: Vec<f64> = (0..n * (cells_z + 1))
        .map(|i| height_at(grid_x(i % n), grid_z(i / n)))
        .collect();

    let osm = Osm::parse(&std::fs::read_to_string(map)?)?;

    // Terrain areas (map editor) change the terrain before anything is fitted to it; the
    // road profiles see the change too (as `delta`, added to the lidar).
    let mut terrain_areas = Vec::new();
    let live_ways = osm.ways.iter().filter(|(_, w)| !w.meta.deleted());
    for (&id, way) in live_ways.filter(|(_, w)| w.meta.tags.contains_key("carsim:terrain")) {
        let corners: Vec<(Point, Option<f64>)> = way
            .refs
            .iter()
            .filter_map(|r| {
                let node = osm.nodes.get(r).filter(|n| !n.meta.deleted())?;
                let ele = node.meta.tags.get("ele").and_then(|e| e.parse().ok());
                Some((frame.to_local(node.lat, node.lon), ele))
            })
            .collect();
        match terrain::Area::new(&way.meta.tags, &corners) {
            Ok(area) => terrain_areas.push(area),
            Err(e) => eprintln!("terrain area way {id} skipped: {e}"),
        }
    }
    let delta: Option<Vec<f32>> = (!terrain_areas.is_empty()).then(|| {
        let before = heights.clone();
        for area in &terrain_areas {
            area.apply(&mut heights, n, (-half_x, -half_z), CELL);
        }
        eprintln!("{} terrain areas applied", terrain_areas.len());
        heights
            .iter()
            .zip(&before)
            .map(|(a, b)| (a - b) as f32)
            .collect()
    });
    // The change at a point, between grid vertices.
    let delta_at = |x: f64, z: f64| -> f64 {
        let Some(delta) = &delta else {
            return 0.0;
        };
        let fx = ((x + half_x) / CELL).clamp(0.0, cells_x as f64 - 1e-9);
        let fz = ((z + half_z) / CELL).clamp(0.0, cells_z as f64 - 1e-9);
        let (i, j) = (fx as usize, fz as usize);
        let (tx, tz) = (fx - i as f64, fz - j as f64);
        let d = |i: usize, j: usize| delta[j * n + i] as f64;
        (d(i, j) * (1.0 - tx) + d(i + 1, j) * tx) * (1.0 - tz)
            + (d(i, j + 1) * (1.0 - tx) + d(i + 1, j + 1) * tx) * tz
    };
    // Road heights pinned in the map editor.
    let pins: HashMap<i64, f64> = osm
        .nodes
        .iter()
        .filter(|(_, n)| !n.meta.deleted())
        .filter_map(|(&id, n)| Some((id, n.meta.tags.get("ele")?.parse().ok()?)))
        .collect();
    let on_map = |&(x, z): &Point| x.abs() < half_x && z.abs() < half_z;
    // Roads stop where the rim starts to rise.
    let inside = |&(x, z): &Point| x.abs() < half_x - RIM && z.abs() < half_z - RIM;

    // Roads, waterways, buildings and land use areas. Elements are visited in ID order, so
    // the same data always bakes to the same file.
    let mut roads = Vec::new();
    // Per road: its way, and whether it's off the ground (a bridge or a tunnel).
    let mut road_ways = Vec::new();
    let mut off_ground = Vec::new();
    let mut buildings = Vec::new();
    let mut fence_list = Vec::new();
    let mut areas = Vec::new();
    for (&id, way) in osm.ways.iter().filter(|(_, w)| !w.meta.deleted()) {
        let (refs, tags) = (&way.refs, &way.meta.tags);
        if let Some((kind, height)) = fences::style(tags) {
            let mapped: Vec<(Point, Option<f64>)> = refs
                .iter()
                .filter_map(|r| {
                    let node = osm.nodes.get(r).filter(|n| !n.meta.deleted())?;
                    let p = frame.to_local(node.lat, node.lon);
                    Some((p, fences::opening(&node.meta.tags)))
                })
                .collect();
            for run in mapped.split(|(p, _)| !inside(p)) {
                for points in fences::cut(run) {
                    fence_list.push(fences::Fence {
                        way: id,
                        kind,
                        height,
                        points,
                    });
                }
            }
        }
        let mut points = points(&osm, refs, &frame);
        if tags.contains_key("building") {
            points.pop();
            if points.len() >= 3 && points.iter().all(on_map) {
                let levels = tags
                    .get("building:levels")
                    .and_then(|l| l.parse().ok())
                    .unwrap_or(0.0);
                buildings.push(Building {
                    way: id,
                    levels,
                    points,
                });
            }
        } else if let Some(class) = land_class(tags).filter(|_| refs.first() == refs.last()) {
            areas.push(Area {
                class,
                rings: vec![points],
            });
        } else if let Some((kind, width)) = road_style(tags) {
            // Culverts run under the roads.
            if kind == RoadKind::Water && tags.contains_key("tunnel") {
                continue;
            }
            let bridge = tags.get("bridge").is_some_and(|b| b != "no");
            let mapped: Vec<(i64, Point)> = refs
                .iter()
                .filter_map(|r| {
                    let node = osm.nodes.get(r).filter(|n| !n.meta.deleted())?;
                    Some((*r, frame.to_local(node.lat, node.lon)))
                })
                .collect();
            // Split the way wherever it leaves the map.
            for run in mapped.split(|(_, p)| !inside(p)) {
                let mut run = run.to_vec();
                run.dedup_by(|b, a| a.1 == b.1);
                if run.len() >= 2 {
                    let mut road = Road::new(kind, width, bridge, &run);
                    road.roughness =
                        roads::roughness(kind, tags.get("smoothness").map(String::as_str));
                    roads.push(road);
                    road_ways.push(id);
                    off_ground.push(bridge || tags.contains_key("tunnel"));
                }
            }
        }
    }
    for relation in osm.relations.values().filter(|r| !r.meta.deleted()) {
        let tags = &relation.meta.tags;
        if tags.get("type").is_some_and(|t| t == "multipolygon")
            && let Some(class) = land_class(tags)
        {
            areas.push(Area {
                class,
                rings: rings(&osm, &relation.members, &frame),
            });
        }
    }

    // Rasterise land use per grid cell (tested at the cell centre).
    let mut classes = vec![0u8; cells_x * cells_z];
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
        // Scanlines through the cell centres: a centre is inside if an odd number of
        // edges cross its row to its right.
        let (rows, cols) = (
            index_z(z0)..index_z(z1).min(cells_z),
            index_x(x0)..index_x(x1).min(cells_x),
        );
        let mut crossings = vec![Vec::new(); rows.len()];
        for ring in &area.rings {
            for (i, &(xa, za)) in ring.iter().enumerate() {
                let (xb, zb) = ring[(i + 1) % ring.len()];
                // The rows with za.min(zb) <= centre < za.max(zb), give or take one.
                let row = |z: f64| ((z + half_z) / CELL - 0.5).floor() as isize;
                let first = (row(za.min(zb))).max(rows.start as isize) as usize;
                let last = (row(za.max(zb)) + 1).clamp(0, rows.end as isize) as usize;
                for gz in first..last {
                    let pz = grid_z(gz) + CELL / 2.0;
                    if (za > pz) != (zb > pz) {
                        crossings[gz - rows.start].push(xa + (pz - za) / (zb - za) * (xb - xa));
                    }
                }
            }
        }
        for (gz, xs) in rows.zip(&mut crossings) {
            xs.sort_by(f64::total_cmp);
            let mut passed = 0;
            for gx in cols.clone() {
                let px = grid_x(gx) + CELL / 2.0;
                while passed < xs.len() && xs[passed] <= px {
                    passed += 1;
                }
                let slot = &mut classes[gz * cells_x + gx];
                if (xs.len() - passed) % 2 == 1 && class::priority(class) > class::priority(*slot) {
                    *slot = class;
                }
            }
        }
    }

    // Roads off the houses, and a list of where they still run into one, as waypoints
    // for JOSM next to the map file.
    let outlines: Vec<&[Point]> = buildings.iter().map(|b| b.points.as_slice()).collect();
    let (mut conflicts, moved, farthest) = clearance::nudge(&mut roads, &off_ground, &outlines);
    conflicts.sort_by(|a, b| b.overlap.total_cmp(&a.overlap));
    let mut markers: Vec<Marker> = conflicts
        .iter()
        .map(|c| Marker::road_in_building(c, &roads, &road_ways, &buildings))
        .collect();
    // Barriers along roads moved off them; those still on one (crossing it without a
    // gate) are listed.
    let mut fence_list = fences::drop_on_bridges(fence_list, &roads);
    let fences_moved = fences::clear_roads(&mut fence_list, &roads, &off_ground);
    let mut on_roads = fences::on_roads(&fence_list, &roads, &off_ground);
    on_roads.sort_by(|a, b| b.length.total_cmp(&a.length));
    markers.extend(on_roads.iter().map(|o| Marker {
        at: o.at,
        name: format!("barrier {:.0} m", o.length),
        desc: format!(
            "barrier way {} stands {:.1} m on road way {} ({:.1} m wide)",
            fence_list[o.fence].way, o.length, road_ways[o.road], roads[o.road].width
        ),
    }));
    let report = format!("{}.conflicts.gpx", map.trim_end_matches(".osm"));
    write_markers(&report, &markers, &frame)?;
    eprintln!(
        "moved {moved} roads off buildings (up to {farthest:.1} m) and {fences_moved} barriers off roads; {} roads still in buildings and {} barriers on roads, see {report}",
        conflicts.len(),
        on_roads.len()
    );

    // Road heights: smooth profiles along the lidar; the terrain cut and filled to them
    // (not under bridges: lidar shows the ground there, and the decks span over it).
    roads::profiles(&mut roads, &pins, |(x, z)| {
        dgm.sample(frame.east + x, frame.north - z) + delta_at(x, z)
    });
    let grid = Grid {
        cells_x,
        cells_z,
        cell: CELL,
    };
    roads::conform(&roads, &mut heights, &grid);
    roads::carve_under_bridges(&roads, &mut heights, &grid);
    // Bumps and potholes, on the roads their nodes are on (or next to).
    let features: Vec<Feature> = osm
        .nodes
        .values()
        .filter(|n| !n.meta.deleted())
        .filter_map(|n| {
            let kind = Feature::kind_of(&n.meta.tags)?;
            Feature::place(&roads, kind, frame.to_local(n.lat, n.lon))
        })
        .collect();

    // Raised rim so cars stay on the map.
    for (i, h) in heights.iter_mut().enumerate() {
        let edge = (grid_x(i % n).abs() - (half_x - RIM))
            .max(grid_z(i / n).abs() - (half_z - RIM))
            .max(0.0);
        *h += edge * edge * 0.05;
    }

    // Resolution per chunk, as a step in grid cells: full detail around buildings, roads
    // and paved areas, coarse everywhere else.
    let mut steps = vec![STEP_EMPTY; chunks_x * chunks_z];
    let chunk_size = CHUNK_CELLS as f64 * CELL;
    let mut refine = |(x, z): Point, margin: f64, step: usize| {
        let range = |v: f64, half: f64, count: usize| {
            let index = |v: f64| (((v + half) / chunk_size).max(0.0) as usize).min(count - 1);
            index(v - margin)..=index(v + margin)
        };
        for cz in range(z, half_z, chunks_z) {
            for cx in range(x, half_x, chunks_x) {
                let slot = &mut steps[cz * chunks_x + cx];
                *slot = (*slot).min(step);
            }
        }
    };
    for area in &terrain_areas {
        let (mut lo, mut hi) = (area.outline[0], area.outline[0]);
        for &(x, z) in &area.outline {
            lo = (lo.0.min(x), lo.1.min(z));
            hi = (hi.0.max(x), hi.1.max(z));
        }
        let mut z = lo.1;
        while z <= hi.1 + chunk_size {
            let mut x = lo.0;
            while x <= hi.0 + chunk_size {
                refine((x.min(hi.0), z.min(hi.1)), 4.0, STEP_DETAIL);
                x += chunk_size / 2.0;
            }
            z += chunk_size / 2.0;
        }
    }
    for point in buildings.iter().flat_map(|b| &b.points) {
        refine(*point, 20.0, STEP_DETAIL);
    }
    // Wide enough for the embankments.
    for road in roads.iter().filter(|r| r.kind != RoadKind::Water) {
        for &point in &road.points {
            refine(point, 14.0, STEP_DETAIL);
        }
    }
    for (i, _) in classes
        .iter()
        .enumerate()
        .filter(|(_, c)| **c == class::PAVED)
    {
        refine(
            (
                grid_x(i % cells_x) + CELL / 2.0,
                grid_z(i / cells_x) + CELL / 2.0,
            ),
            10.0,
            STEP_DETAIL,
        );
    }

    // Vertices of coarse chunks average the terrain around them (instead of point-sampling
    // it). A vertex shared by several chunks uses the coarsest of their steps.
    let vertex_step = |i: usize, j: usize| {
        let touching = |v: usize, count: usize| {
            let c = (v / CHUNK_CELLS).min(count - 1);
            if v.is_multiple_of(CHUNK_CELLS) && v > 0 {
                [c - 1, c]
            } else {
                [c, c]
            }
        };
        let (xs, zs) = (touching(i, chunks_x), touching(j, chunks_z));
        let mut step = 0;
        for cz in zs {
            for cx in xs {
                step = step.max(steps[cz * chunks_x + cx]);
            }
        }
        step
    };
    let raw = heights.clone();
    for j in 0..=cells_z {
        for i in 0..=cells_x {
            let s = vertex_step(i, j);
            if s > 1 && i.is_multiple_of(s) && j.is_multiple_of(s) {
                let window = |v: usize, max: usize| v.saturating_sub(s / 2)..=(v + s / 2).min(max);
                let (mut sum, mut count) = (0.0, 0.0);
                for wj in window(j, cells_z) {
                    for wi in window(i, cells_x) {
                        sum += raw[wj * n + wi];
                        count += 1.0;
                    }
                }
                heights[j * n + i] = sum / count;
            }
        }
    }
    // Seams: along an edge between chunks of different steps, put the finer side's edge
    // vertices on the coarser side's straight edges, so the surface has no cracks.
    // `a` is the chunk index across the seam (the seam is on its low side), `b` the chunk
    // index along it.
    let seams = (1..chunks_x)
        .flat_map(|a| (0..chunks_z).map(move |b| (false, a, b)))
        .chain((1..chunks_z).flat_map(|a| (0..chunks_x).map(move |b| (true, a, b))));
    for (along_x, a, b) in seams {
        let (chunk_a, chunk_b) = if along_x {
            ((a - 1) * chunks_x + b, a * chunks_x + b)
        } else {
            (b * chunks_x + a - 1, b * chunks_x + a)
        };
        let s = steps[chunk_a].max(steps[chunk_b]);
        let index = |k: usize| {
            if along_x {
                (a * CHUNK_CELLS) * n + k
            } else {
                k * n + a * CHUNK_CELLS
            }
        };
        for k in b * CHUNK_CELLS..=(b + 1) * CHUNK_CELLS {
            let k0 = k - k % s;
            if !k.is_multiple_of(s) {
                let t = (k - k0) as f64 / s as f64;
                heights[index(k)] = heights[index(k0)] * (1.0 - t) + heights[index(k0 + s)] * t;
            }
        }
    }

    // Write (all little-endian, and the whole file compressed with zlib): b"CSTR", version u32, cells along x and z u32, cell size f32,
    // chunk cells u32, spawn point (x, z) f32, spawn heading as a direction (x, z) f32
    // ((0, 0) if not given), the lowest point's height above sea level f32 (heights are
    // above it),
    // then per chunk (row-major, rows along z): step u8 (in cells), the chunk's
    // (chunk cells / step + 1)² vertex heights u16 in centimetres above the lowest point
    // (row-major, each as the wrapping difference from left + above - above-left, or the
    // one neighbour at the edges, or 0; all low bytes, then all high bytes) and
    // (chunk cells / step)² land use classes u8 (row-major),
    // road count u32, then per road: kind u8 (0 asphalt, 1 gravel, 2 water, 3 cobbles,
    // 4 dirt), bridge u8 (0 or 1), width f32, crown f32 (fall per metre to the edges),
    // roughness f32 (ISO 8608's displacement spectrum at 0.1 cycles/m, in 10⁻⁶ m³),
    // point count u32, points (x, z in cm, height above the lowest point in mm; 0 for
    // waterways): the first as i32, the others as i16 steps from the one before,
    // feature count u32, then per feature: road u32, kind u8 (0 bump, 1 hump, 2 table,
    // 3 pothole), along the road, right of its centreline, length (potholes: diameter) and
    // height (potholes: negative) f32 in metres,
    // building count u32, then per building: levels f32 (0: unknown), point count u32,
    // outline points (x, z) f32,
    // fence count u32, then per fence (barriers: walls, hedges, guard rails too): kind u8
    // (0 rails, 1 metal, 2 panel, 3 wall, 4 hedge, 5 guard rail), height f32, point count
    // u32, points (x, z) f32.
    let base = heights.iter().copied().fold(f64::INFINITY, f64::min);
    let top = heights.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if top - base > u16::MAX as f64 / 100.0 {
        return Err(format!(
            "relief {:.0} m doesn't fit in centimetres as u16",
            top - base
        )
        .into());
    }
    let (spawn, heading) = match spawn[..] {
        [lat, lon, ref heading @ ..] => (
            frame.to_local(lat, lon),
            // Compass degrees to the map frame (x east, z south).
            heading.first().map_or((0.0, 0.0), |h| {
                (h.to_radians().sin(), -h.to_radians().cos())
            }),
        ),
        _ => ((0.0, 0.0), (0.0, 0.0)),
    };
    let mut bytes = Vec::new();
    let mut put = |v: &[u8]| bytes.extend_from_slice(v);
    put(b"CSTR");
    put(&13u32.to_le_bytes());
    put(&(cells_x as u32).to_le_bytes());
    put(&(cells_z as u32).to_le_bytes());
    put(&(CELL as f32).to_le_bytes());
    put(&(CHUNK_CELLS as u32).to_le_bytes());
    put(&(spawn.0 as f32).to_le_bytes());
    put(&(spawn.1 as f32).to_le_bytes());
    put(&(heading.0 as f32).to_le_bytes());
    put(&(heading.1 as f32).to_le_bytes());
    put(&(base as f32).to_le_bytes());
    for cz in 0..chunks_z {
        for cx in 0..chunks_x {
            let s = steps[cz * chunks_x + cx];
            let m = CHUNK_CELLS / s;
            let (i0, j0) = (cx * CHUNK_CELLS, cz * CHUNK_CELLS);
            put(&[s as u8]);
            // Each height as what's left after predicting it from its neighbours before
            // it (left plus above minus above-left; at the edges, the one neighbour),
            // wrapping; low bytes first, then high bytes.
            let cm = |i: usize, j: usize| {
                ((heights[(j0 + j * s) * n + i0 + i * s] - base) * 100.0).round() as u16
            };
            let mut left = Vec::new();
            for j in 0..=m {
                for i in 0..=m {
                    let guess = match (i, j) {
                        (0, 0) => 0,
                        (0, _) => cm(0, j - 1),
                        (_, 0) => cm(i - 1, 0),
                        _ => cm(i - 1, j)
                            .wrapping_add(cm(i, j - 1))
                            .wrapping_sub(cm(i - 1, j - 1)),
                    };
                    left.push(cm(i, j).wrapping_sub(guess));
                }
            }
            put(&left.iter().map(|v| *v as u8).collect::<Vec<_>>());
            put(&left.iter().map(|v| (*v >> 8) as u8).collect::<Vec<_>>());
            for j in 0..m {
                for i in 0..m {
                    put(&[classes[(j0 + j * s + s / 2) * cells_x + i0 + i * s + s / 2]]);
                }
            }
        }
    }
    let polyline = |put: &mut dyn FnMut(&[u8]), points: &[Point]| {
        put(&(points.len() as u32).to_le_bytes());
        for &(x, z) in points {
            put(&(x as f32).to_le_bytes());
            put(&(z as f32).to_le_bytes());
        }
    };
    put(&(roads.len() as u32).to_le_bytes());
    for road in &roads {
        put(&[road.kind as u8, road.bridge as u8]);
        put(&(road.width as f32).to_le_bytes());
        put(&(road.crown() as f32).to_le_bytes());
        put(&(road.roughness as f32).to_le_bytes());
        put(&(road.points.len() as u32).to_le_bytes());
        // Centimetres along, millimetres up: the first point as is, then the steps.
        let mut last = [0i32; 3];
        for (i, &(x, z)) in road.points.iter().enumerate() {
            let height = road.heights.get(i).map_or(0.0, |h| h - base);
            let at = [
                (x * 100.0).round() as i32,
                (z * 100.0).round() as i32,
                (height * 1000.0).round() as i32,
            ];
            for k in 0..3 {
                if i == 0 {
                    put(&at[k].to_le_bytes());
                } else {
                    let step = i16::try_from(at[k] - last[k]).map_err(|_| "road step too long")?;
                    put(&step.to_le_bytes());
                }
            }
            last = at;
        }
    }
    put(&(features.len() as u32).to_le_bytes());
    for f in &features {
        put(&(f.road as u32).to_le_bytes());
        put(&[f.kind as u8]);
        for v in [f.along, f.right, f.size, f.height] {
            put(&(v as f32).to_le_bytes());
        }
    }
    put(&(buildings.len() as u32).to_le_bytes());
    for building in &buildings {
        put(&(building.levels as f32).to_le_bytes());
        polyline(&mut put, &building.points);
    }
    put(&(fence_list.len() as u32).to_le_bytes());
    for fence in &fence_list {
        put(&[fence.kind as u8]);
        put(&(fence.height as f32).to_le_bytes());
        polyline(&mut put, &fence.points);
    }
    let raw = bytes.len();
    let bytes = miniz_oxide::deflate::compress_to_vec_zlib(&bytes, 8);
    std::fs::write(out, &bytes)?;
    eprintln!(
        "compressed {} KiB to {} KiB",
        raw / 1024,
        bytes.len() / 1024
    );
    let count = |s: usize| steps.iter().filter(|&&step| step == s).count();
    eprintln!(
        "chunks: {} at {CELL} m, {} at {} m",
        count(STEP_DETAIL),
        count(STEP_EMPTY),
        CELL * STEP_EMPTY as f64
    );
    eprintln!(
        "wrote {out}: {cells_x}x{cells_z} cells of {CELL} m, {} roads, {} buildings, {} fences, {} areas, relief {:.1} m, {} KiB",
        roads.len(),
        buildings.len(),
        fence_list.len(),
        areas.len(),
        heights.iter().copied().fold(f64::NEG_INFINITY, f64::max) - base,
        bytes.len() / 1024
    );
    Ok(())
}
