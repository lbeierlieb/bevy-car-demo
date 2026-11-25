//! The bytes on the network: a compact little-endian encoding, by hand. Reading checks
//! everything (lengths, float values, catalogue indices where it matters), so a bad or
//! hostile message is rejected rather than trusted.

use bevy::prelude::*;

use crate::catalogue::Shared;

/// Something that goes over the network.
pub trait Wire: Sized {
    fn put(&self, out: &mut Vec<u8>);
    fn get(r: &mut Reader) -> Option<Self>;
}

/// Reads values off the front of a message: cars by their places in a catalogue, which
/// the reader needs then.
pub struct Reader<'a> {
    bytes: &'a [u8],
    catalogue: Option<&'a Shared>,
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            catalogue: None,
        }
    }

    /// Reads cars as built from `catalogue`.
    pub fn with(bytes: &'a [u8], catalogue: Option<&'a Shared>) -> Self {
        Self { bytes, catalogue }
    }

    /// The catalogue cars are read with, if there's one (a car without one is malformed).
    pub fn catalogue(&self) -> Option<&'a Shared> {
        self.catalogue
    }

    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.bytes.len() < n {
            return None;
        }
        let (head, rest) = self.bytes.split_at(n);
        self.bytes = rest;
        Some(head)
    }

    pub fn get<T: Wire>(&mut self) -> Option<T> {
        T::get(self)
    }

    /// Whether everything was read (a message with bytes left over is malformed).
    pub fn done(&self) -> bool {
        self.bytes.is_empty()
    }
}

macro_rules! number {
    ($($t:ty),*) => {$(
        impl Wire for $t {
            fn put(&self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
            fn get(r: &mut Reader) -> Option<Self> {
                Some(<$t>::from_le_bytes(r.take(size_of::<$t>())?.try_into().ok()?))
            }
        }
    )*};
}
number!(u8, u16, u32, u64, i8, i16, i32);

impl Wire for f32 {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
    /// Only finite values.
    fn get(r: &mut Reader) -> Option<Self> {
        let x = f32::from_le_bytes(r.take(4)?.try_into().ok()?);
        x.is_finite().then_some(x)
    }
}

impl Wire for f64 {
    fn put(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.to_le_bytes());
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let x = f64::from_le_bytes(r.take(8)?.try_into().ok()?);
        x.is_finite().then_some(x)
    }
}

impl Wire for bool {
    fn put(&self, out: &mut Vec<u8>) {
        out.push(u8::from(*self));
    }
    fn get(r: &mut Reader) -> Option<Self> {
        match r.get::<u8>()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
}

/// Indices into catalogues, as one byte.
impl Wire for usize {
    fn put(&self, out: &mut Vec<u8>) {
        out.push(u8::try_from(*self).expect("an index over 255"));
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(usize::from(r.get::<u8>()?))
    }
}

/// Bytes of a big message (engine maps), up to `MAX_BLOB`.
#[derive(Clone, Debug, PartialEq)]
pub struct Blob(pub Vec<u8>);

/// The most bytes in a `Blob`.
pub const MAX_BLOB: usize = 8 << 20;

impl Wire for Blob {
    fn put(&self, out: &mut Vec<u8>) {
        u32::try_from(self.0.len()).expect("too long").put(out);
        out.extend_from_slice(&self.0);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let n = r.get::<u32>()? as usize;
        (n <= MAX_BLOB).then_some(())?;
        Some(Self(r.take(n)?.to_vec()))
    }
}

/// Text up to `MAX_TEXT` bytes.
pub const MAX_TEXT: usize = 200;

impl Wire for String {
    fn put(&self, out: &mut Vec<u8>) {
        let mut end = self.len().min(MAX_TEXT);
        while !self.is_char_boundary(end) {
            end -= 1;
        }
        (end as u8).put(out);
        out.extend_from_slice(&self.as_bytes()[..end]);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let n = usize::from(r.get::<u8>()?);
        String::from_utf8(r.take(n)?.to_vec()).ok()
    }
}

impl<T: Wire> Wire for Option<T> {
    fn put(&self, out: &mut Vec<u8>) {
        self.is_some().put(out);
        if let Some(x) = self {
            x.put(out);
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(if r.get::<bool>()? {
            Some(r.get()?)
        } else {
            None
        })
    }
}

impl<T: Wire> Wire for Vec<T> {
    fn put(&self, out: &mut Vec<u8>) {
        u16::try_from(self.len()).expect("too long").put(out);
        for x in self {
            x.put(out);
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let n = r.get::<u16>()?;
        (0..n).map(|_| r.get()).collect()
    }
}

impl<T: Wire, const N: usize> Wire for [T; N] {
    fn put(&self, out: &mut Vec<u8>) {
        for x in self {
            x.put(out);
        }
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let items: Option<Vec<T>> = (0..N).map(|_| r.get()).collect();
        items?.try_into().ok()
    }
}

impl Wire for Vec2 {
    fn put(&self, out: &mut Vec<u8>) {
        self.to_array().put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Vec2::from_array(r.get()?))
    }
}

impl Wire for crate::terrain::Spawn {
    fn put(&self, out: &mut Vec<u8>) {
        self.at.put(out);
        self.heading.put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Self {
            at: r.get()?,
            heading: r.get()?,
        })
    }
}

impl Wire for Vec3 {
    fn put(&self, out: &mut Vec<u8>) {
        self.to_array().put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        Some(Vec3::from_array(r.get()?))
    }
}

/// A rotation, normalised when read (a zero one is rejected).
impl Wire for Quat {
    fn put(&self, out: &mut Vec<u8>) {
        self.to_array().put(out);
    }
    fn get(r: &mut Reader) -> Option<Self> {
        let q = Quat::from_array(r.get()?);
        (q.length_squared() > 0.5).then(|| q.normalize())
    }
}

/// Writes `value` into a new message.
pub fn encode<T: Wire>(value: &T) -> Vec<u8> {
    let mut out = Vec::new();
    value.put(&mut out);
    out
}

/// Reads a whole message, or nothing if it's malformed.
pub fn decode<T: Wire>(bytes: &[u8]) -> Option<T> {
    decode_with(bytes, None)
}

/// Reads a whole message with cars built from `catalogue`, or nothing if it's malformed.
pub fn decode_with<T: Wire>(bytes: &[u8], catalogue: Option<&Shared>) -> Option<T> {
    let mut r = Reader::with(bytes, catalogue);
    let value = r.get()?;
    r.done().then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_read_back() {
        let v = (
            7u8,
            -0.5f32,
            Vec3::new(1.0, 2.0, 3.0),
            "Grüße".to_string(),
            vec![1u16, 2, 3],
        );
        let mut out = Vec::new();
        v.0.put(&mut out);
        v.1.put(&mut out);
        v.2.put(&mut out);
        v.3.put(&mut out);
        v.4.put(&mut out);
        let mut r = Reader::new(&out);
        assert_eq!(r.get::<u8>(), Some(v.0));
        assert_eq!(r.get::<f32>(), Some(v.1));
        assert_eq!(r.get::<Vec3>(), Some(v.2));
        assert_eq!(r.get::<String>(), Some(v.3));
        assert_eq!(r.get::<Vec<u16>>(), Some(v.4));
        assert!(r.done());
    }

    #[test]
    fn bad_values_are_rejected() {
        assert_eq!(decode::<f32>(&f32::NAN.to_le_bytes()), None);
        assert_eq!(decode::<bool>(&[2]), None);
        assert_eq!(decode::<u32>(&[1, 2, 3]), None);
        assert_eq!(decode::<u8>(&[1, 2]), None, "bytes left over");
        assert_eq!(decode::<String>(&[5, b'a']), None);
    }
}
