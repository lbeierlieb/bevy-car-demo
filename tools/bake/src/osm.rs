//! The map file: OSM XML as JOSM reads and writes it. `import` fills it from OpenStreetMap,
//! after which it is ours, edited in JOSM and committed; the bake reads only this file.
//!
//! JOSM marks what it changed in a local file with `action` (`modify`, `delete`) and gives
//! new elements negative ids. Deleted elements stay in the file (so an import doesn't bring
//! them back) and are skipped by the bake. `upload='never'` makes JOSM refuse to upload
//! the file to OpenStreetMap.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use quick_xml::events::Event;

use crate::Error;

pub type Tags = BTreeMap<String, String>;

/// Tags kept by `import`; the rest (addresses, sidewalks, sources, …) only bloat the file.
/// Everything the bake reads must be here.
const KEEP: &[&str] = &[
    "amenity",
    "area",
    "area:highway",
    "barrier",
    "bridge",
    "building",
    "building:levels",
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
            let event = reader.read_event()?;
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
                let a = a?;
                attrs.insert(
                    a.key.as_ref().to_owned(),
                    a.normalized_value(quick_xml::XmlVersion::Explicit1_0)?
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
                    lat: attr("lat").parse()?,
                    lon: attr("lon").parse()?,
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
                        ("nd", Some((_, Element::Way(way)))) => way.refs.push(attr("ref").parse()?),
                        ("member", Some((_, Element::Relation(relation)))) => {
                            relation.members.push(Member {
                                kind: attr("type").to_owned(),
                                id: attr("ref").parse()?,
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
                            attr("minlat").parse()?,
                            attr("minlon").parse()?,
                            attr("maxlat").parse()?,
                            attr("maxlon").parse()?,
                        ]),
                        _ => {}
                    }
                    continue;
                }
            };
            let id = attr("id").parse()?;
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
        out += "<osm version='0.6' upload='never' generator='car-sim bake'>\n";
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

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
        .replace('"', "&quot;")
}
