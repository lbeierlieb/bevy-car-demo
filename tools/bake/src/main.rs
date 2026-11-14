//! Bakes a square of real-world terrain and drivable roads into the binary format that
//! car-sim embeds (see `src/terrain.rs` in the game for the reader).
//!
//! Usage: `cargo run --release -- <lat> <lon> <size_m> <out_file>`
//!
//! Elevation comes from the AWS Terrain Tiles (Terrarium encoding), roads from the
//! OpenStreetMap API. Roads are smoothed along their length and the terrain is flattened
//! under them, because ~30 m elevation data is far too lumpy to drive on directly.

use std::collections::HashMap;
use std::f64::consts::PI;
use std::io::Cursor;

use quick_xml::events::Event;

/// Terrain grid spacing in metres.
const CELL: f64 = 4.0;
const TILE_ZOOM: u32 = 15;
const EARTH_RADIUS: f64 = 6_371_000.0;
/// Spacing of resampled road points, and the half-length of the window used to smooth
/// the road's height profile.
const ROAD_STEP: f64 = 2.0;
const PROFILE_SMOOTHING: f64 = 20.0;
/// Terrain within this distance of a road edge is flattened to road height, then blends
/// back into the natural terrain over `BLEND` metres.
const SHOULDER: f64 = 1.5;
const BLEND: f64 = 8.0;
const USER_AGENT: &str = "car-sim-bake/0.1 (Bevy car simulation demo)";

type Error = Box<dyn std::error::Error>;

/// Maps between lat/lon and local metres (x east, z south) around the map centre.
/// An equirectangular projection is plenty accurate over a few kilometres.
struct Projection {
    lat0: f64,
    lon0: f64,
}

impl Projection {
    fn to_local(&self, lat: f64, lon: f64) -> (f64, f64) {
        let x = (lon - self.lon0).to_radians() * EARTH_RADIUS * self.lat0.to_radians().cos();
        let z = -(lat - self.lat0).to_radians() * EARTH_RADIUS;
        (x, z)
    }

    fn to_geo(&self, x: f64, z: f64) -> (f64, f64) {
        let lat = self.lat0 + (-z / EARTH_RADIUS).to_degrees();
        let lon = self.lon0 + (x / (EARTH_RADIUS * self.lat0.to_radians().cos())).to_degrees();
        (lat, lon)
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

/// Elevation from Terrarium-encoded web-mercator tiles, sampled bilinearly.
struct Elevation {
    tiles: HashMap<(u32, u32), Vec<f32>>,
}

impl Elevation {
    /// Fractional global pixel coordinates of a lat/lon at `TILE_ZOOM`.
    fn pixel(lat: f64, lon: f64) -> (f64, f64) {
        let n = (1u32 << TILE_ZOOM) as f64 * 256.0;
        let x = (lon + 180.0) / 360.0 * n;
        let lat = lat.to_radians();
        let y = (1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / PI) / 2.0 * n;
        (x, y)
    }

    fn fetch(lat_range: (f64, f64), lon_range: (f64, f64)) -> Result<Self, Error> {
        let (x0, y0) = Self::pixel(lat_range.1, lon_range.0);
        let (x1, y1) = Self::pixel(lat_range.0, lon_range.1);
        let mut tiles = HashMap::new();
        for ty in (y0 as u32 / 256)..=(y1 as u32 / 256 + 1) {
            for tx in (x0 as u32 / 256)..=(x1 as u32 / 256 + 1) {
                let url = format!(
                    "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/{TILE_ZOOM}/{tx}/{ty}.png"
                );
                tiles.insert((tx, ty), decode_terrarium(&get(&url)?)?);
            }
        }
        Ok(Self { tiles })
    }

    fn at_pixel(&self, x: i64, y: i64) -> f64 {
        let tile = &self.tiles[&((x / 256) as u32, (y / 256) as u32)];
        tile[(y % 256 * 256 + x % 256) as usize] as f64
    }

    fn sample(&self, lat: f64, lon: f64) -> f64 {
        // Pixel centres sit at half-integer coordinates.
        let (x, y) = Self::pixel(lat, lon);
        let (x, y) = (x - 0.5, y - 0.5);
        let (ix, iy) = (x.floor() as i64, y.floor() as i64);
        let (fx, fy) = (x - ix as f64, y - iy as f64);
        let top = self.at_pixel(ix, iy) * (1.0 - fx) + self.at_pixel(ix + 1, iy) * fx;
        let bottom = self.at_pixel(ix, iy + 1) * (1.0 - fx) + self.at_pixel(ix + 1, iy + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// Terrarium encoding: height = R·256 + G + B/256 − 32768 metres.
fn decode_terrarium(png_bytes: &[u8]) -> Result<Vec<f32>, Error> {
    let mut reader = png::Decoder::new(Cursor::new(png_bytes)).read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("png too large")?];
    let info = reader.next_frame(&mut buf)?;
    let channels = info.color_type.samples();
    Ok(buf[..info.buffer_size()]
        .chunks(channels)
        .map(|p| p[0] as f32 * 256.0 + p[1] as f32 + p[2] as f32 / 256.0 - 32768.0)
        .collect())
}

struct Road {
    paved: bool,
    width: f64,
    /// Points in local metres.
    points: Vec<(f64, f64)>,
}

/// Lane width for each drivable OSM highway type; anything else is skipped.
fn road_width(highway: &str) -> Option<f64> {
    Some(match highway {
        "motorway" | "trunk" | "primary" => 7.0,
        "secondary" => 6.5,
        "tertiary" | "motorway_link" | "trunk_link" | "primary_link" | "secondary_link" => 6.0,
        "tertiary_link" | "unclassified" | "residential" => 5.0,
        "living_street" => 4.5,
        "service" => 3.5,
        "track" => 3.0,
        _ => return None,
    })
}

fn fetch_roads(projection: &Projection, half_size: f64) -> Result<Vec<Road>, Error> {
    let (lat_min, lon_min) = projection.to_geo(-half_size, half_size);
    let (lat_max, lon_max) = projection.to_geo(half_size, -half_size);
    let url = format!(
        "https://api.openstreetmap.org/api/0.6/map?bbox={lon_min:.6},{lat_min:.6},{lon_max:.6},{lat_max:.6}"
    );
    let xml = String::from_utf8(get(&url)?)?;

    let mut nodes = HashMap::new();
    let mut ways = Vec::new();
    // The way currently being read: its node refs and tags.
    let mut way: Option<(Vec<u64>, HashMap<String, String>)> = None;
    let mut reader = quick_xml::Reader::from_str(&xml);
    loop {
        let event = reader.read_event()?;
        let (Event::Start(e) | Event::Empty(e)) = &event else {
            match event {
                Event::End(e) if e.name().as_ref() == "way" => ways.extend(way.take()),
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
                nodes.insert(attr("id").parse::<u64>()?, projection.to_local(lat, lon));
            }
            "way" => way = Some((Vec::new(), HashMap::new())),
            "nd" => {
                if let Some((refs, _)) = &mut way {
                    refs.push(attr("ref").parse()?);
                }
            }
            "tag" => {
                if let Some((_, tags)) = &mut way {
                    tags.insert(attr("k"), attr("v"));
                }
            }
            _ => {}
        }
    }

    let inside = |&(x, z): &(f64, f64)| x.abs() < half_size && z.abs() < half_size;
    let mut roads = Vec::new();
    for (refs, tags) in ways {
        let Some(width) = tags.get("highway").and_then(|h| road_width(h)) else {
            continue;
        };
        let paved = tags["highway"] != "track";
        let points: Vec<_> = refs.iter().filter_map(|r| nodes.get(r).copied()).collect();
        // Split the way wherever it leaves the map.
        for run in points.split(|p| !inside(p)).filter(|run| run.len() >= 2) {
            roads.push(Road {
                paved,
                width,
                points: run.to_vec(),
            });
        }
    }
    Ok(roads)
}

/// Resamples a polyline at roughly `ROAD_STEP` spacing, keeping the original vertices.
fn resample(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
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

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().collect();
    let [_, lat, lon, size, out] = args.as_slice() else {
        return Err("usage: bake <lat> <lon> <size_m> <out_file>".into());
    };
    let projection = Projection {
        lat0: lat.parse()?,
        lon0: lon.parse()?,
    };
    let cells = (size.parse::<f64>()? / CELL).round() as usize;
    let half_size = cells as f64 * CELL / 2.0;
    let n = cells + 1;

    // Raw terrain heights on the grid.
    let (lat_min, lon_min) = projection.to_geo(-half_size, half_size);
    let (lat_max, lon_max) = projection.to_geo(half_size, -half_size);
    let elevation = Elevation::fetch((lat_min, lat_max), (lon_min, lon_max))?;
    let height_at = |x: f64, z: f64| {
        let (lat, lon) = projection.to_geo(x, z);
        elevation.sample(lat, lon)
    };
    let grid_x = |i: usize| i as f64 * CELL - half_size;
    let mut heights: Vec<f64> = (0..n * n)
        .map(|i| height_at(grid_x(i % n), grid_x(i / n)))
        .collect();

    // Roads: resample, then smooth each road's height profile along its length.
    let mut roads = fetch_roads(&projection, half_size)?;
    let mut profiles = Vec::new();
    for road in &mut roads {
        road.points = resample(&road.points);
        let raw: Vec<f64> = road.points.iter().map(|&(x, z)| height_at(x, z)).collect();
        let k = (PROFILE_SMOOTHING / ROAD_STEP) as usize;
        profiles.push(
            (0..raw.len())
                .map(|i| {
                    let window = &raw[i.saturating_sub(k)..(i + k + 1).min(raw.len())];
                    window.iter().sum::<f64>() / window.len() as f64
                })
                .collect::<Vec<f64>>(),
        );
    }

    // Flatten the terrain towards the nearest road's height, segment by segment.
    // Per grid vertex: (distance to the nearest road edge, that road's height there).
    let mut nearest = vec![(f64::INFINITY, 0.0); n * n];
    for (road, profile) in roads.iter().zip(&profiles) {
        let reach = road.width / 2.0 + SHOULDER + BLEND;
        for (i, pair) in road.points.windows(2).enumerate() {
            let ((x0, z0), (x1, z1)) = (pair[0], pair[1]);
            let (dx, dz) = (x1 - x0, z1 - z0);
            let len2 = (dx * dx + dz * dz).max(1e-9);
            let to_index = |v: f64| ((v + half_size) / CELL).clamp(0.0, cells as f64) as usize;
            for gz in to_index(z0.min(z1) - reach)..=to_index(z0.max(z1) + reach) {
                for gx in to_index(x0.min(x1) - reach)..=to_index(x0.max(x1) + reach) {
                    let (px, pz) = (grid_x(gx), grid_x(gz));
                    let t = (((px - x0) * dx + (pz - z0) * dz) / len2).clamp(0.0, 1.0);
                    let edge = (px - x0 - dx * t).hypot(pz - z0 - dz * t) - road.width / 2.0;
                    let slot = &mut nearest[gz * n + gx];
                    if edge < slot.0 {
                        *slot = (edge, profile[i] * (1.0 - t) + profile[i + 1] * t);
                    }
                }
            }
        }
    }
    for (h, &(edge, road_height)) in heights.iter_mut().zip(&nearest) {
        let w = 1.0 - smoothstep((edge - SHOULDER) / BLEND);
        *h += (road_height - *h) * w;
    }

    // Write (all little-endian): b"CSTR", version u32, cells u32, cell size f32,
    // (cells + 1)² heights f32 relative to the lowest point (row-major, rows along z),
    // road count u32, then per road: paved u8, width f32, point count u32, points (x, z) f32.
    let base = heights.iter().copied().fold(f64::INFINITY, f64::min);
    let mut bytes = Vec::new();
    let mut put = |v: &[u8]| bytes.extend_from_slice(v);
    put(b"CSTR");
    put(&1u32.to_le_bytes());
    put(&(cells as u32).to_le_bytes());
    put(&(CELL as f32).to_le_bytes());
    for h in &heights {
        put(&((h - base) as f32).to_le_bytes());
    }
    put(&(roads.len() as u32).to_le_bytes());
    for road in &roads {
        put(&[road.paved as u8]);
        put(&(road.width as f32).to_le_bytes());
        put(&(road.points.len() as u32).to_le_bytes());
        for &(x, z) in &road.points {
            put(&(x as f32).to_le_bytes());
            put(&(z as f32).to_le_bytes());
        }
    }
    std::fs::write(out, &bytes)?;
    eprintln!(
        "wrote {out}: {cells}x{cells} cells of {CELL} m, {} roads, relief {:.1} m, {} KiB",
        roads.len(),
        heights.iter().copied().fold(f64::NEG_INFINITY, f64::max) - base,
        bytes.len() / 1024
    );
    Ok(())
}
