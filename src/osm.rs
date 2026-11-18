//! The map file: OSM XML as JOSM reads and writes it. `bake import` fills it from
//! OpenStreetMap, after which it is ours, edited in the map editor (`bin/editor.rs`) or
//! JOSM and committed; the bake reads only this file. Shared by the game and the bake
//! (`tools/bake` includes this file as its `osm` module), so it only uses `std` and
//! quick-xml.
//!
//! JOSM marks what it changed in a local file with `action` (`modify`, `delete`) and gives
//! new elements negative ids. Deleted elements stay in the file (so an import doesn't bring
//! them back) and are skipped by the bake. `upload='never'` makes JOSM refuse to upload
//! the file to OpenStreetMap.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use quick_xml::events::Event;

/// What went wrong reading the file.
pub type Error = String;

fn err(e: impl std::fmt::Display) -> Error {
    e.to_string()
}

pub type Tags = BTreeMap<String, String>;

/// Tags kept by `import`; the rest (addresses, sidewalks, sources, …) only bloat the file.
/// Everything the bake reads must be here.
const KEEP: &[&str] = &[
    "amenity",
    "area",
    "area:highway",
    "barrier",
    "depth",
    "diameter",
    "fence_type",
    "bridge",
    "building",
    "building:levels",
    "hazard",
    "height",
    "highway",
    "landuse",
    "lanes",
    "layer",
    "leisure",
    "maxspeed",
    "name",
    "natural",
    "oneway",
    "ref",
    "service",
    "smoothness",
    "surface",
    "tracktype",
    "traffic_calming",
    "tunnel",
    "type",
    "water",
    "waterway",
    "width",
];

/// What every element has besides its own data.
#[derive(Clone, Default)]
pub struct Meta {
    pub version: u32,
    /// JOSM's `action` attribute (`modify`, `delete`), if any.
    pub action: Option<String>,
    pub tags: Tags,
}

impl Meta {
    pub fn deleted(&self) -> bool {
        self.action.as_deref() == Some("delete")
    }
}

#[derive(Clone)]
pub struct Node {
    pub meta: Meta,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Clone)]
pub struct Way {
    pub meta: Meta,
    pub refs: Vec<i64>,
}

#[derive(Clone)]
pub struct Member {
    pub kind: String,
    pub id: i64,
    pub role: String,
}

#[derive(Clone)]
pub struct Relation {
    pub meta: Meta,
    pub members: Vec<Member>,
}

#[derive(Default)]
pub struct Osm {
    /// Imported boxes as (min lat, min lon, max lat, max lon); JOSM shows them.
    pub bounds: Vec<[f64; 4]>,
    pub nodes: BTreeMap<i64, Node>,
    pub ways: BTreeMap<i64, Way>,
    pub relations: BTreeMap<i64, Relation>,
}

enum Element {
    Node(Node),
    Way(Way),
    Relation(Relation),
}

impl Element {
    fn meta(&mut self) -> &mut Meta {
        match self {
            Element::Node(n) => &mut n.meta,
            Element::Way(w) => &mut w.meta,
            Element::Relation(r) => &mut r.meta,
        }
    }
}

impl Osm {
    pub fn parse(xml: &str) -> Result<Self, Error> {
        let mut osm = Osm::default();
        let mut current: Option<(i64, Element)> = None;
        let mut reader = quick_xml::Reader::from_str(xml);
        loop {
            let event = reader.read_event().map_err(err)?;
            let (e, empty) = match &event {
                Event::Start(e) => (e, false),
                Event::Empty(e) => (e, true),
                Event::End(e) => {
                    if matches!(e.name().as_ref(), "node" | "way" | "relation")
                        && let Some((id, element)) = current.take()
                    {
                        osm.insert(id, element);
                    }
                    continue;
                }
                Event::Eof => break,
                _ => continue,
            };
            let mut attrs = BTreeMap::new();
            for a in e.attributes() {
                let a = a.map_err(err)?;
                attrs.insert(
                    a.key.as_ref().to_owned(),
                    a.normalized_value(quick_xml::XmlVersion::Explicit1_0)
                        .map_err(err)?
                        .into_owned(),
                );
            }
            let attr = |key: &str| attrs.get(key).map(String::as_str).unwrap_or_default();
            let meta = || Meta {
                version: attr("version").parse().unwrap_or(1),
                action: attrs.get("action").cloned(),
                tags: Tags::new(),
            };
            let element = match e.name().as_ref() {
                "node" => Element::Node(Node {
                    meta: meta(),
                    lat: attr("lat").parse().map_err(err)?,
                    lon: attr("lon").parse().map_err(err)?,
                }),
                "way" => Element::Way(Way {
                    meta: meta(),
                    refs: Vec::new(),
                }),
                "relation" => Element::Relation(Relation {
                    meta: meta(),
                    members: Vec::new(),
                }),
                child => {
                    match (child, &mut current) {
                        ("nd", Some((_, Element::Way(way)))) => {
                            way.refs.push(attr("ref").parse().map_err(err)?)
                        }
                        ("member", Some((_, Element::Relation(relation)))) => {
                            relation.members.push(Member {
                                kind: attr("type").to_owned(),
                                id: attr("ref").parse().map_err(err)?,
                                role: attr("role").to_owned(),
                            })
                        }
                        ("tag", Some((_, element))) => {
                            element
                                .meta()
                                .tags
                                .insert(attr("k").to_owned(), attr("v").to_owned());
                        }
                        ("bounds", _) => osm.bounds.push([
                            attr("minlat").parse().map_err(err)?,
                            attr("minlon").parse().map_err(err)?,
                            attr("maxlat").parse().map_err(err)?,
                            attr("maxlon").parse().map_err(err)?,
                        ]),
                        _ => {}
                    }
                    continue;
                }
            };
            let id = attr("id").parse().map_err(err)?;
            if empty {
                osm.insert(id, element);
            } else {
                current = Some((id, element));
            }
        }
        Ok(osm)
    }

    fn insert(&mut self, id: i64, element: Element) {
        match element {
            Element::Node(n) => drop(self.nodes.insert(id, n)),
            Element::Way(w) => drop(self.ways.insert(id, w)),
            Element::Relation(r) => drop(self.relations.insert(id, r)),
        }
    }

    /// Drops the tags not in `KEEP`.
    pub fn strip_tags(&mut self) {
        let strip = |meta: &mut Meta| meta.tags.retain(|k, _| KEEP.contains(&k.as_str()));
        self.nodes.values_mut().for_each(|n| strip(&mut n.meta));
        self.ways.values_mut().for_each(|w| strip(&mut w.meta));
        self.relations.values_mut().for_each(|r| strip(&mut r.meta));
    }

    /// Adds the elements of `other` that aren't here yet (also not as deleted ones):
    /// whatever is already in the file stays as it was edited. Returns how many were added.
    pub fn merge(&mut self, other: Osm) -> usize {
        let before = self.nodes.len() + self.ways.len() + self.relations.len();
        for (id, node) in other.nodes {
            self.nodes.entry(id).or_insert(node);
        }
        for (id, way) in other.ways {
            self.ways.entry(id).or_insert(way);
        }
        for (id, relation) in other.relations {
            self.relations.entry(id).or_insert(relation);
        }
        for bounds in other.bounds {
            if !self.bounds.contains(&bounds) {
                self.bounds.push(bounds);
            }
        }
        self.nodes.len() + self.ways.len() + self.relations.len() - before
    }

    /// The file as JOSM writes it: single quotes, nodes, then ways, then relations.
    pub fn write(&self) -> String {
        let mut out = String::from("<?xml version='1.0' encoding='UTF-8'?>\n");
        out += "<osm version='0.6' upload='never' generator='car-sim'>\n";
        for [minlat, minlon, maxlat, maxlon] in &self.bounds {
            let _ = writeln!(
                out,
                "  <bounds minlat='{minlat}' minlon='{minlon}' maxlat='{maxlat}' maxlon='{maxlon}' />"
            );
        }
        let open = |out: &mut String, kind: &str, id: i64, meta: &Meta| {
            let _ = write!(out, "  <{kind} id='{id}'");
            if let Some(action) = &meta.action {
                let _ = write!(out, " action='{}'", escape(action));
            }
            let _ = write!(out, " version='{}' visible='true'", meta.version);
        };
        let tags = |out: &mut String, meta: &Meta| {
            for (k, v) in &meta.tags {
                let _ = writeln!(out, "    <tag k='{}' v='{}' />", escape(k), escape(v));
            }
        };
        for (&id, node) in &self.nodes {
            open(&mut out, "node", id, &node.meta);
            let _ = write!(out, " lat='{}' lon='{}'", node.lat, node.lon);
            if node.meta.tags.is_empty() {
                out += " />\n";
            } else {
                out += ">\n";
                tags(&mut out, &node.meta);
                out += "  </node>\n";
            }
        }
        for (&id, way) in &self.ways {
            open(&mut out, "way", id, &way.meta);
            out += ">\n";
            for r in &way.refs {
                let _ = writeln!(out, "    <nd ref='{r}' />");
            }
            tags(&mut out, &way.meta);
            out += "  </way>\n";
        }
        for (&id, relation) in &self.relations {
            open(&mut out, "relation", id, &relation.meta);
            out += ">\n";
            for m in &relation.members {
                let _ = writeln!(
                    out,
                    "    <member type='{}' ref='{}' role='{}' />",
                    escape(&m.kind),
                    m.id,
                    escape(&m.role)
                );
            }
            tags(&mut out, &relation.meta);
            out += "  </relation>\n";
        }
        out += "</osm>\n";
        out
    }
}

/// UTM zone 32 (ETRS89) easting and northing of a latitude and longitude (Krüger's series).
pub fn utm32(lat: f64, lon: f64) -> (f64, f64) {
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

/// The map frame: UTM shifted to the map centre, with z pointing south (metres).
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub east: f64,
    pub north: f64,
}

impl Frame {
    /// The frame centred on (lat, lon), rounded to a whole metre so grid vertices line up
    /// with lidar pixel edges.
    pub fn around(lat: f64, lon: f64) -> Self {
        let (east, north) = utm32(lat, lon);
        Self {
            east: east.round(),
            north: north.round(),
        }
    }

    pub fn to_local(self, lat: f64, lon: f64) -> (f64, f64) {
        let (e, n) = utm32(lat, lon);
        (e - self.east, self.north - n)
    }

    /// Latitude and longitude of a local point (Newton's method on `to_local`).
    pub fn to_geo(self, p: (f64, f64)) -> (f64, f64) {
        let mut geo = (50.0, 10.0);
        let step = 1e-6;
        for _ in 0..8 {
            let at = self.to_local(geo.0, geo.1);
            let (dx, dz) = (p.0 - at.0, p.1 - at.1);
            let by_lat = self.to_local(geo.0 + step, geo.1);
            let by_lon = self.to_local(geo.0, geo.1 + step);
            let (a, c) = ((by_lat.0 - at.0) / step, (by_lat.1 - at.1) / step);
            let (b, d) = ((by_lon.0 - at.0) / step, (by_lon.1 - at.1) / step);
            let det = a * d - b * c;
            geo.0 += (d * dx - b * dz) / det;
            geo.1 += (a * dz - c * dx) / det;
        }
        geo
    }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_reads_back_as_written() {
        let xml = "<?xml version='1.0' encoding='UTF-8'?>\n<osm version='0.6'>\n\
            <node id='1' version='2' lat='50.01' lon='10.84'><tag k='hazard' v='pothole' /></node>\n\
            <node id='-2' action='modify' version='1' lat='50.02' lon='10.85' />\n\
            <way id='5' action='delete' version='3'><nd ref='1' /><nd ref='-2' />\
            <tag k='name' v='A &amp; B' /></way>\n</osm>\n";
        let osm = Osm::parse(xml).unwrap();
        let again = Osm::parse(&osm.write()).unwrap();
        assert_eq!(again.write(), osm.write());
        assert_eq!(again.nodes[&1].meta.tags["hazard"], "pothole");
        assert_eq!(again.nodes[&-2].meta.action.as_deref(), Some("modify"));
        assert!(again.ways[&5].meta.deleted());
        assert_eq!(again.ways[&5].refs, [1, -2]);
        assert_eq!(again.ways[&5].meta.tags["name"], "A & B");
    }

    #[test]
    fn geo_and_local_points_convert_both_ways() {
        let frame = Frame::around(50.0118569, 10.8455963);
        let p = frame.to_local(50.0305556, 10.8046389);
        let (lat, lon) = frame.to_geo(p);
        assert!((lat - 50.0305556).abs() < 1e-8 && (lon - 10.8046389).abs() < 1e-8);
    }
}
