//! Bakes a rectangle of real-world terrain, land use, roads and buildings into the binary
//! format that car-sim embeds (see `src/terrain.rs` in the game for the reader).
//!
//! Usage: `bake <lat> <lon> <width_m>[x<height_m>] <out_file> [<spawn_lat> <spawn_lon>]`
//!
//! Elevation is the 1 m lidar terrain model (DGM1) of the Bavarian surveying
//! administration, so this only works inside Bavaria. Everything else comes from
//! OpenStreetMap, via the Overpass API. Downloads are cached in `tools/bake/cache`;
//! delete it to fetch fresh data. The map frame is aligned with UTM zone 32 (x east, z south), the
//! grid the DGM1 tiles use.

use std::collections::HashMap;
use std::io::Cursor;

use quick_xml::events::Event;

/// Terrain grid spacing in metres, at full detail.
const CELL: f64 = 2.0;
/// Chunk side in cells (96 m). Each chunk is stored at one of the steps below, which all
/// divide it. The map is a whole number of chunks along each axis.
const CHUNK_CELLS: usize = 48;
/// Chunk resolutions as steps in cells: 2 m, 4 m and 8 m.
const STEP_DETAIL: usize = 1;
const STEP_TRACKS: usize = 2;
const STEP_EMPTY: usize = 4;
/// Width of the raised rim at the map edge.
const RIM: f64 = 30.0;
const USER_AGENT: &str = "car-sim-bake/0.3 (Bevy car simulation demo)";
const CACHE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/cache");
const OVERPASS: &str = "https://overpass-api.de/api/interpreter";
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
    tiles: HashMap<(i64, i64), Vec<f32>>,
}

impl Dgm1 {
    fn fetch(east: (f64, f64), north: (f64, f64)) -> Result<Self, Error> {
        let km = |v: f64| (v / 1000.0).floor() as i64;
        let mut tiles = HashMap::new();
        for e in km(east.0)..=km(east.1) {
            for n in km(north.0)..=km(north.1) {
                let url = format!("https://download1.bayernwolke.de/a/dgm/dgm1/{e}_{n}.tif");
                let tile = cached(&format!("dgm1_{e}_{n}.tif"), || get(&url))?;
                let mut decoder = tiff::decoder::Decoder::new(Cursor::new(tile))?;
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
    relations: HashMap<u64, (Vec<u64>, Tags)>,
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
                                self.relations.insert(id, (refs, tags));
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

/// Everything the map uses in a lat/lon box around the map, with one Overpass query: the
/// ways with tags that roads, buildings or land use read, the multipolygon relations, and
/// (recursively) all their member ways and nodes, also those outside the box. Everything
/// is clipped to the map in local metres later.
fn fetch_osm(frame: &Frame, center: Point, (half_x, half_z): Point) -> Result<Osm, Error> {
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
    ]
    .iter()
    .map(|key| format!("way[\"{key}\"]({bbox});"))
    .collect();
    let query = format!(
        "[out:xml][timeout:600];\
         ({ways}relation[\"type\"=\"multipolygon\"]({bbox}););(._;>;);out body;"
    );
    let xml = cached(&format!("osm_{bbox}.xml"), || {
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
    let mut osm = Osm::default();
    osm.parse(&String::from_utf8(xml)?, frame)?;
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
    let (lat, lon, size, out, spawn) = match args.as_slice() {
        [_, lat, lon, size, out] => (lat, lon, size, out, None),
        [_, lat, lon, size, out, spawn_lat, spawn_lon] => (
            lat,
            lon,
            size,
            out,
            Some((spawn_lat.parse::<f64>()?, spawn_lon.parse::<f64>()?)),
        ),
        _ => {
            return Err("usage: bake <lat> <lon> <width_m>[x<height_m>] <out_file> \
                        [<spawn_lat> <spawn_lon>]"
                .into());
        }
    };
    let center = (lat.parse::<f64>()?, lon.parse::<f64>()?);
    let (east, north) = utm32(center.0, center.1);
    // Centre on a whole metre so grid vertices line up with lidar pixel edges.
    let frame = Frame {
        east: east.round(),
        north: north.round(),
    };
    // Whole chunks along each axis.
    let (width, height) = match size.split_once('x') {
        Some((w, h)) => (w.parse::<f64>()?, h.parse::<f64>()?),
        None => (size.parse()?, size.parse()?),
    };
    let chunk_count = |m: f64| (m / (CELL * CHUNK_CELLS as f64)).round().max(1.0) as usize;
    let (chunks_x, chunks_z) = (chunk_count(width), chunk_count(height));
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

    let osm = fetch_osm(&frame, center, (half_x, half_z))?;
    let on_map = |&(x, z): &Point| x.abs() < half_x && z.abs() < half_z;

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
    for (members, tags) in osm.relations.values() {
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
        for gz in index_z(z0)..index_z(z1).min(cells_z) {
            for gx in index_x(x0)..index_x(x1).min(cells_x) {
                let slot = &mut classes[gz * cells_x + gx];
                let centre = (grid_x(gx) + CELL / 2.0, grid_z(gz) + CELL / 2.0);
                if class::priority(class) > class::priority(*slot)
                    && inside_rings(&area.rings, centre)
                {
                    *slot = class;
                }
            }
        }
    }

    // Bridges: straight decks between the end heights, terrain raised up to them.
    let mut nearest = vec![(f64::INFINITY, 0.0); heights.len()];
    let grid_height = |x: f64, z: f64| heights[index_z(z) * n + index_x(x)];
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
            for gz in index_z(z0.min(z1) - reach)..=index_z(z0.max(z1) + reach) {
                for gx in index_x(x0.min(x1) - reach)..=index_x(x0.max(x1) + reach) {
                    let (px, pz) = (grid_x(gx), grid_z(gz));
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

    // Raised rim so cars stay on the map.
    for (i, h) in heights.iter_mut().enumerate() {
        let edge = (grid_x(i % n).abs() - (half_x - RIM))
            .max(grid_z(i / n).abs() - (half_z - RIM))
            .max(0.0);
        *h += edge * edge * 0.05;
    }

    // Resolution per chunk, as a step in grid cells: full detail around buildings, paved
    // roads and paved areas, medium around tracks, coarse everywhere else.
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
    for point in buildings.iter().flat_map(|b| &b.points) {
        refine(*point, 20.0, STEP_DETAIL);
    }
    for road in &roads {
        let (margin, step) = match road.kind {
            RoadKind::Paved => (10.0, STEP_DETAIL),
            RoadKind::Unpaved => (6.0, STEP_TRACKS),
            RoadKind::Water => continue,
        };
        for point in resample(&road.points) {
            refine(point, margin, step);
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

    // Write (all little-endian): b"CSTR", version u32, cells along x and z u32, cell size f32,
    // chunk cells u32, spawn point (x, z) f32,
    // then per chunk (row-major, rows along z): step u8 (in cells), the chunk's
    // (chunk cells / step + 1)² vertex heights f32 relative to the lowest point and
    // (chunk cells / step)² land use classes u8 (row-major),
    // road count u32, then per road: kind u8 (0 paved, 1 unpaved, 2 water), width f32,
    // point count u32, points (x, z) f32,
    // building count u32, then per building: levels f32 (0: unknown), point count u32,
    // outline points (x, z) f32.
    let base = heights.iter().copied().fold(f64::INFINITY, f64::min);
    let spawn = spawn.map_or((0.0, 0.0), |(lat, lon)| frame.to_local(lat, lon));
    let mut bytes = Vec::new();
    let mut put = |v: &[u8]| bytes.extend_from_slice(v);
    put(b"CSTR");
    put(&5u32.to_le_bytes());
    put(&(cells_x as u32).to_le_bytes());
    put(&(cells_z as u32).to_le_bytes());
    put(&(CELL as f32).to_le_bytes());
    put(&(CHUNK_CELLS as u32).to_le_bytes());
    put(&(spawn.0 as f32).to_le_bytes());
    put(&(spawn.1 as f32).to_le_bytes());
    for cz in 0..chunks_z {
        for cx in 0..chunks_x {
            let s = steps[cz * chunks_x + cx];
            let m = CHUNK_CELLS / s;
            let (i0, j0) = (cx * CHUNK_CELLS, cz * CHUNK_CELLS);
            put(&[s as u8]);
            for j in 0..=m {
                for i in 0..=m {
                    put(&((heights[(j0 + j * s) * n + i0 + i * s] - base) as f32).to_le_bytes());
                }
            }
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
    let count = |s: usize| steps.iter().filter(|&&step| step == s).count();
    eprintln!(
        "chunks: {} at {CELL} m, {} at {} m, {} at {} m",
        count(STEP_DETAIL),
        count(STEP_TRACKS),
        CELL * STEP_TRACKS as f64,
        count(STEP_EMPTY),
        CELL * STEP_EMPTY as f64
    );
    eprintln!(
        "wrote {out}: {cells_x}x{cells_z} cells of {CELL} m, {} roads, {} buildings, {} areas, relief {:.1} m, {} KiB",
        roads.len(),
        buildings.len(),
        areas.len(),
        heights.iter().copied().fold(f64::NEG_INFINITY, f64::max) - base,
        bytes.len() / 1024
    );
    Ok(())
}
