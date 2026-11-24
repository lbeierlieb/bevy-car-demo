//! What the ground under a tire is made of: how well it grips and how much it holds the
//! wheel back. Roads grip well; everything off them deliberately poorly (it's not an
//! off-road game).

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Surface {
    #[default]
    Asphalt,
    /// Paving stones, setts, concrete lanes with grass between.
    Cobbles,
    Gravel,
    /// Unpaved earth and grass tracks.
    Dirt,
    /// Meadows, gardens, verges.
    Grass,
    Field,
    Forest,
    Water,
}

impl Surface {
    pub const ALL: [Surface; 8] = [
        Surface::Asphalt,
        Surface::Cobbles,
        Surface::Gravel,
        Surface::Dirt,
        Surface::Grass,
        Surface::Field,
        Surface::Forest,
        Surface::Water,
    ];

    /// Share of the tire's grip on dry asphalt.
    pub fn grip(self) -> f32 {
        match self {
            Surface::Asphalt => 1.0,
            Surface::Cobbles => 0.85,
            Surface::Gravel => 0.65,
            Surface::Dirt => 0.6,
            Surface::Grass => 0.45,
            Surface::Field => 0.45,
            Surface::Forest => 0.5,
            Surface::Water => 0.3,
        }
    }

    /// Rolling resistance, times the tire's on asphalt (soft ground gives way under it).
    pub fn rolling(self) -> f32 {
        match self {
            Surface::Asphalt => 1.0,
            Surface::Cobbles => 1.5,
            Surface::Gravel => 2.5,
            Surface::Dirt => 4.0,
            Surface::Grass => 6.0,
            Surface::Field => 10.0,
            Surface::Forest => 6.0,
            Surface::Water => 15.0,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Surface::Asphalt => "asphalt",
            Surface::Cobbles => "cobbles",
            Surface::Gravel => "gravel",
            Surface::Dirt => "dirt",
            Surface::Grass => "grass",
            Surface::Field => "field",
            Surface::Forest => "forest",
            Surface::Water => "water",
        }
    }
}

impl crate::snapshot::Snapshot for Surface {
    fn save(&self, out: &mut Vec<u8>) {
        out.push(*self as u8);
    }
    fn load(&mut self, r: &mut crate::net::wire::Reader) -> Option<()> {
        *self = *Self::ALL.get(usize::from(r.get::<u8>()?))?;
        Some(())
    }
}
