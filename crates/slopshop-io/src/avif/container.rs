//! The AVIF container (HEIF in ISOBMFF boxes): which item is the image, where its data is, and
//! its properties. Read from the whole file in memory (AVIF files are compressed), every offset
//! and length checked.

use std::collections::HashMap;

/// An item's properties that SlopShop uses.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Properties {
    /// `ispe`: the image size.
    pub size: Option<(u32, u32)>,
    /// `colr` of type `nclx`: H.273 primaries, transfer, matrix, full range.
    pub nclx: Option<(u8, u8, u8, bool)>,
    /// `colr` of type `prof` or `rICC`: an ICC profile.
    pub icc: Option<Vec<u8>>,
    /// `auxC`: the auxiliary type (alpha planes have one).
    pub aux_type: Option<String>,
    /// `irot` and `imir`, in the order the item lists them.
    pub transforms: Vec<Transform>,
    /// `clap`: the clean aperture, as (width, height, horizontal offset, vertical offset)
    /// fractions.
    pub clean_aperture: Option<[(i64, i64); 4]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Transform {
    /// Quarter turns, anticlockwise.
    Rotate(u8),
    /// Mirror about the vertical axis (left and right swap).
    MirrorHorizontally,
    /// Mirror about the horizontal axis (top and bottom swap).
    MirrorVertically,
}

/// An item: its type, data and properties.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Item {
    pub id: u32,
    pub kind: [u8; 4],
    pub data: Vec<u8>,
    pub properties: Properties,
}

/// What the image of an AVIF file is made of.
#[derive(Debug)]
pub(super) struct Avif {
    /// The primary item: an `av01` image, or a `grid` of them.
    pub primary: Item,
    /// The tiles of a grid, row by row.
    pub tiles: Vec<Item>,
    /// The alpha plane (an auxiliary `av01` or `grid` item), with its tiles.
    pub alpha: Option<(Item, Vec<Item>)>,
    /// The alpha is premultiplied (`prem` reference).
    pub premultiplied: bool,
    /// The file also holds an image sequence (an animated AVIF).
    pub animated: bool,
}

const ALPHA_URNS: [&str; 2] = [
    "urn:mpeg:mpegB:cicp:systems:auxiliary:alpha",
    "urn:mpeg:hevc:2015:auxid:1",
];

type Result<T> = std::result::Result<T, String>;

/// An item reference: its type, the item it is from, the items it is to.
type Reference = ([u8; 4], u32, Vec<u32>);

/// A cursor over big-endian box data.
#[derive(Clone, Copy)]
struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, at: 0 }
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.data.len());
        let end = end.ok_or("truncated AVIF box")?;
        let out = &self.data[self.at..end];
        self.at = end;
        Ok(out)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.bytes(1)?[0])
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.bytes(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// An unsigned integer of `n` bytes (0, 4 or 8, as `iloc` sizes).
    fn uint(&mut self, n: u8) -> Result<u64> {
        match n {
            0 => Ok(0),
            4 => Ok(u64::from(self.u32()?)),
            8 => {
                let b = self.bytes(8)?;
                Ok(u64::from_be_bytes(b.try_into().map_err(|_| "bad length")?))
            }
            _ => Err(format!("unsupported field size {n}")),
        }
    }

    /// A full box's version, skipping its flags.
    fn version_flags(&mut self) -> Result<(u8, u32)> {
        let v = self.u32()?;
        Ok(((v >> 24) as u8, v & 0x00ff_ffff))
    }

    fn rest(&mut self) -> &'a [u8] {
        let out = &self.data[self.at..];
        self.at = self.data.len();
        out
    }

    fn is_empty(&self) -> bool {
        self.at >= self.data.len()
    }

    /// A null-terminated string.
    fn string(&mut self) -> Result<String> {
        let rest = &self.data[self.at..];
        let len = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        let s = String::from_utf8_lossy(&rest[..len]).into_owned();
        self.at += (len + 1).min(rest.len());
        Ok(s)
    }
}

/// The boxes in `data`: their type and content.
fn boxes(data: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    let mut out = Vec::new();
    let mut r = Reader::new(data);
    while !r.is_empty() {
        let start = r.at;
        let size = u64::from(r.u32()?);
        let kind: [u8; 4] = r.bytes(4)?.try_into().map_err(|_| "bad box type")?;
        let size = match size {
            0 => (data.len() - start) as u64,
            1 => r.uint(8)?,
            size => size,
        };
        let header = (r.at - start) as u64;
        let content = size
            .checked_sub(header)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or("bad AVIF box size")?;
        out.push((kind, r.bytes(content)?));
    }
    Ok(out)
}

/// Whether `head` starts like an AVIF file (its `ftyp` box names an AVIF brand).
pub(crate) fn is_avif(head: &[u8]) -> bool {
    let Some(size) = head
        .get(0..4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    else {
        return false;
    };
    if head.get(4..8) != Some(b"ftyp") || size < 16 {
        return false;
    }
    let end = (size as usize).min(head.len());
    head[8..end]
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        // The major brand, then the compatible ones (after the minor version).
        .filter(|(i, _)| *i != 1)
        .any(|(_, brand)| brand == b"avif" || brand == b"avis")
}

/// Read the image structure of the AVIF file `data`.
pub(super) fn parse(data: &[u8]) -> Result<Avif> {
    let top = boxes(data)?;
    let meta = top
        .iter()
        .find(|(kind, _)| kind == b"meta")
        .ok_or("AVIF without a meta box")?
        .1;
    let animated = top.iter().any(|(kind, _)| kind == b"moov");
    let mut r = Reader::new(meta);
    r.version_flags()?;
    let children = boxes(r.rest())?;
    let child = |kind: &[u8; 4]| children.iter().find(|(k, _)| k == kind).map(|(_, d)| *d);

    let primary_id = {
        let mut r = Reader::new(child(b"pitm").ok_or("AVIF without a primary item")?);
        let (version, _) = r.version_flags()?;
        if version == 0 {
            u32::from(r.u16()?)
        } else {
            r.u32()?
        }
    };
    let kinds = item_kinds(child(b"iinf").ok_or("AVIF without item information")?)?;
    let locations = item_locations(child(b"iloc").ok_or("AVIF without item locations")?)?;
    let idat = child(b"idat").unwrap_or(&[]);
    let properties = match child(b"iprp") {
        Some(iprp) => item_properties(iprp)?,
        None => HashMap::new(),
    };
    let references = match child(b"iref") {
        Some(iref) => item_references(iref)?,
        None => Vec::new(),
    };
    let item = |id: u32| -> Result<Item> {
        let kind = *kinds
            .get(&id)
            .ok_or(format!("AVIF item {id} without a type"))?;
        let location = locations
            .get(&id)
            .ok_or(format!("AVIF item {id} without a location"))?;
        Ok(Item {
            id,
            kind,
            data: location.read(data, idat)?,
            properties: properties.get(&id).cloned().unwrap_or_default(),
        })
    };
    // A grid's tiles: its `dimg` references, in order.
    let tiles_of = |grid: &Item| -> Result<Vec<Item>> {
        if &grid.kind != b"grid" {
            return Ok(Vec::new());
        }
        let ids = references
            .iter()
            .find(|(kind, from, _)| kind == b"dimg" && *from == grid.id)
            .map(|(_, _, to)| to.clone())
            .ok_or("AVIF grid without tiles")?;
        ids.into_iter().map(item).collect()
    };
    let primary = item(primary_id)?;
    if &primary.kind != b"av01" && &primary.kind != b"grid" {
        return Err(format!(
            "AVIF primary item of type {}",
            String::from_utf8_lossy(&primary.kind)
        ));
    }
    let tiles = tiles_of(&primary)?;
    // The alpha plane: an auxiliary image (`auxl`) of the primary one, of an alpha type.
    let mut alpha = None;
    for (kind, from, to) in &references {
        if kind == b"auxl" && to.contains(&primary_id) {
            let candidate = item(*from)?;
            let is_alpha = candidate
                .properties
                .aux_type
                .as_deref()
                .is_some_and(|t| ALPHA_URNS.contains(&t));
            if is_alpha {
                let alpha_tiles = tiles_of(&candidate)?;
                alpha = Some((candidate, alpha_tiles));
                break;
            }
        }
    }
    let premultiplied = match &alpha {
        Some((a, _)) => references
            .iter()
            .any(|(kind, from, to)| kind == b"prem" && *from == primary_id && to.contains(&a.id)),
        None => false,
    };
    Ok(Avif {
        primary,
        tiles,
        alpha,
        premultiplied,
        animated,
    })
}

/// `iinf`: each item's type.
fn item_kinds(iinf: &[u8]) -> Result<HashMap<u32, [u8; 4]>> {
    let mut r = Reader::new(iinf);
    let (version, _) = r.version_flags()?;
    if version == 0 {
        r.u16()?;
    } else {
        r.u32()?;
    }
    let mut out = HashMap::new();
    for (kind, infe) in boxes(r.rest())? {
        if &kind != b"infe" {
            continue;
        }
        let mut r = Reader::new(infe);
        let (version, _) = r.version_flags()?;
        if version < 2 {
            // Versions 0 and 1 carry no item type.
            continue;
        }
        let id = if version == 2 {
            u32::from(r.u16()?)
        } else {
            r.u32()?
        };
        r.u16()?;
        let item_type: [u8; 4] = r.bytes(4)?.try_into().map_err(|_| "bad item type")?;
        out.insert(id, item_type);
    }
    Ok(out)
}

/// Where an item's data is: extents in the file, or in the `idat` box.
struct Location {
    in_idat: bool,
    extents: Vec<(u64, u64)>,
}

impl Location {
    fn read(&self, file: &[u8], idat: &[u8]) -> Result<Vec<u8>> {
        let source = if self.in_idat { idat } else { file };
        let mut out = Vec::new();
        for &(offset, length) in &self.extents {
            let start = usize::try_from(offset).map_err(|_| "AVIF offset too large")?;
            // A length of 0: up to the end of the source.
            let end = if length == 0 {
                source.len()
            } else {
                usize::try_from(length)
                    .ok()
                    .and_then(|l| start.checked_add(l))
                    .ok_or("AVIF length too large")?
            };
            let bytes = source
                .get(start..end)
                .ok_or("AVIF item data outside the file")?;
            out.extend_from_slice(bytes);
        }
        Ok(out)
    }
}

/// `iloc`: each item's location.
fn item_locations(iloc: &[u8]) -> Result<HashMap<u32, Location>> {
    let mut r = Reader::new(iloc);
    let (version, _) = r.version_flags()?;
    let sizes = r.u16()?;
    let (offset_size, length_size) = ((sizes >> 12) as u8, ((sizes >> 8) & 15) as u8);
    let base_offset_size = ((sizes >> 4) & 15) as u8;
    let index_size = if version >= 1 { (sizes & 15) as u8 } else { 0 };
    let count = if version < 2 {
        u32::from(r.u16()?)
    } else {
        r.u32()?
    };
    let mut out = HashMap::new();
    for _ in 0..count {
        let id = if version < 2 {
            u32::from(r.u16()?)
        } else {
            r.u32()?
        };
        let method = if version >= 1 { r.u16()? & 15 } else { 0 };
        r.u16()?;
        let base = r.uint(base_offset_size)?;
        let extent_count = r.u16()?;
        let mut extents = Vec::new();
        for _ in 0..extent_count {
            r.uint(index_size)?;
            let offset = r.uint(offset_size)?;
            let length = r.uint(length_size)?;
            let offset = base.checked_add(offset).ok_or("AVIF offset overflow")?;
            extents.push((offset, length));
        }
        if method > 1 {
            return Err("AVIF item stored by reference".to_owned());
        }
        out.insert(
            id,
            Location {
                in_idat: method == 1,
                extents,
            },
        );
    }
    Ok(out)
}

/// `iprp`: each item's properties (the `ipco` boxes their `ipma` associations name).
fn item_properties(iprp: &[u8]) -> Result<HashMap<u32, Properties>> {
    let children = boxes(iprp)?;
    let ipco = children
        .iter()
        .find(|(k, _)| k == b"ipco")
        .map(|(_, d)| boxes(d))
        .transpose()?
        .unwrap_or_default();
    let mut out: HashMap<u32, Properties> = HashMap::new();
    for (_, ipma) in children.iter().filter(|(k, _)| k == b"ipma") {
        let mut r = Reader::new(ipma);
        let (version, flags) = r.version_flags()?;
        let count = r.u32()?;
        for _ in 0..count {
            let id = if version < 1 {
                u32::from(r.u16()?)
            } else {
                r.u32()?
            };
            let associations = r.u8()?;
            let properties = out.entry(id).or_default();
            for _ in 0..associations {
                let index = if flags & 1 != 0 {
                    usize::from(r.u16()? & 0x7fff)
                } else {
                    usize::from(r.u8()? & 0x7f)
                };
                // 0: no property; indices start at 1.
                if let Some((kind, data)) = index.checked_sub(1).and_then(|i| ipco.get(i)) {
                    add_property(properties, kind, data)?;
                }
            }
        }
    }
    Ok(out)
}

fn add_property(p: &mut Properties, kind: &[u8; 4], data: &[u8]) -> Result<()> {
    let mut r = Reader::new(data);
    match kind {
        b"ispe" => {
            r.version_flags()?;
            p.size = Some((r.u32()?, r.u32()?));
        }
        b"colr" => match r.bytes(4)? {
            b"nclx" => {
                let primaries = r.u16()?;
                let transfer = r.u16()?;
                let matrix = r.u16()?;
                let full = r.u8()? & 0x80 != 0;
                let code = |v: u16| u8::try_from(v).unwrap_or(2);
                p.nclx = Some((code(primaries), code(transfer), code(matrix), full));
            }
            b"prof" | b"rICC" => p.icc = Some(r.rest().to_vec()),
            _ => {}
        },
        b"auxC" => {
            r.version_flags()?;
            p.aux_type = Some(r.string()?);
        }
        b"irot" => p.transforms.push(Transform::Rotate(r.u8()? & 3)),
        b"imir" => p.transforms.push(if r.u8()? & 1 == 0 {
            Transform::MirrorHorizontally
        } else {
            Transform::MirrorVertically
        }),
        b"clap" => {
            let mut fraction = || -> Result<(i64, i64)> {
                let n = i64::from(r.u32()? as i32);
                let d = i64::from(r.u32()? as i32);
                Ok((n, d))
            };
            p.clean_aperture = Some([fraction()?, fraction()?, fraction()?, fraction()?]);
        }
        _ => {}
    }
    Ok(())
}

/// `iref`: references as (type, from, to).
fn item_references(iref: &[u8]) -> Result<Vec<Reference>> {
    let mut r = Reader::new(iref);
    let (version, _) = r.version_flags()?;
    let mut out = Vec::new();
    for (kind, data) in boxes(r.rest())? {
        let mut r = Reader::new(data);
        let id = |r: &mut Reader| -> Result<u32> {
            if version == 0 {
                Ok(u32::from(r.u16()?))
            } else {
                r.u32()
            }
        };
        let from = id(&mut r)?;
        let count = r.u16()?;
        let to = (0..count)
            .map(|_| id(&mut r))
            .collect::<Result<Vec<u32>>>()?;
        out.push((kind, from, to));
    }
    Ok(out)
}

/// A `grid` item's layout: rows, columns, and the output size.
pub(super) fn grid_layout(data: &[u8]) -> Result<(u32, u32, u32, u32)> {
    let mut r = Reader::new(data);
    r.u8()?;
    let flags = r.u8()?;
    let rows = u32::from(r.u8()?) + 1;
    let columns = u32::from(r.u8()?) + 1;
    let (width, height) = if flags & 1 != 0 {
        (r.u32()?, r.u32()?)
    } else {
        (u32::from(r.u16()?), u32::from(r.u16()?))
    };
    Ok((rows, columns, width, height))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxed(kind: &[u8; 4], content: &[u8]) -> Vec<u8> {
        let mut out = ((content.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend(kind);
        out.extend(content);
        out
    }

    #[test]
    fn brands_are_recognized() {
        let ftyp = boxed(b"ftyp", b"avif\0\0\0\0mif1miaf");
        assert!(is_avif(&ftyp));
        let compatible = boxed(b"ftyp", b"mif1\0\0\0\0avifmiaf");
        assert!(is_avif(&compatible));
        assert!(!is_avif(&boxed(b"ftyp", b"heic\0\0\0\0mif1heic")));
        // The minor version is not a brand.
        assert!(!is_avif(&boxed(b"ftyp", b"mif1avif")));
    }

    #[test]
    fn color_properties_are_read() {
        let mut p = Properties::default();
        // nclx: Display P3 primaries (12), sRGB transfer (13), BT.709 matrix, full range.
        add_property(
            &mut p,
            b"colr",
            &[b'n', b'c', b'l', b'x', 0, 12, 0, 13, 0, 1, 0x80],
        )
        .unwrap();
        assert_eq!(p.nclx, Some((12, 13, 1, true)));
        add_property(&mut p, b"colr", b"profICC!").unwrap();
        assert_eq!(p.icc.as_deref(), Some(&b"ICC!"[..]));
        add_property(&mut p, b"irot", &[1]).unwrap();
        add_property(&mut p, b"imir", &[1]).unwrap();
        assert_eq!(
            p.transforms,
            [Transform::Rotate(1), Transform::MirrorVertically]
        );
        add_property(&mut p, b"ispe", &[0, 0, 0, 0, 0, 0, 0, 7, 0, 0, 0, 9]).unwrap();
        assert_eq!(p.size, Some((7, 9)));
    }

    #[test]
    fn damaged_structures_are_errors() {
        assert!(parse(&[]).is_err());
        assert!(parse(&boxed(b"ftyp", b"avif\0\0\0\0")).is_err());
        // A box claiming more bytes than there are.
        let mut bad = boxed(b"meta", &[0; 8]);
        bad[3] = 200;
        assert!(parse(&bad).is_err());
        assert!(grid_layout(&[0, 0, 1]).is_err());
    }
}
