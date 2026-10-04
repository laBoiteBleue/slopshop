//! Photoshop's action descriptors (structured data in tagged blocks such as `lfx2`), read into a
//! tree. The file is untrusted: every length is checked against the data, and nesting is
//! bounded. Items of types this reader does not know end the descriptor (`None`).

/// A value of a descriptor item.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    Object(Descriptor),
    List(Vec<Value>),
    Double(f64),
    /// A number with its unit (`#Prc`, `#Pxl`, `#Ang`…).
    Unit([u8; 4], f64),
    Integer(i64),
    Bool(bool),
    Text(String),
    /// An enumerated value: its type and the value.
    Enum(Vec<u8>, Vec<u8>),
    /// A reference, a class, an alias or raw data: read past, not kept.
    Other,
}

/// A descriptor: its class and its items, in order.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Descriptor {
    pub class: Vec<u8>,
    pub items: Vec<(Vec<u8>, Value)>,
}

impl Descriptor {
    pub fn get(&self, key: &[u8]) -> Option<&Value> {
        self.items.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn object(&self, key: &[u8]) -> Option<&Descriptor> {
        match self.get(key)? {
            Value::Object(d) => Some(d),
            _ => None,
        }
    }

    pub fn list(&self, key: &[u8]) -> Option<&[Value]> {
        match self.get(key)? {
            Value::List(values) => Some(values),
            _ => None,
        }
    }

    /// A number, whatever its unit.
    pub fn number(&self, key: &[u8]) -> Option<f64> {
        match self.get(key)? {
            Value::Double(v) | Value::Unit(_, v) => Some(*v),
            Value::Integer(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn bool(&self, key: &[u8]) -> Option<bool> {
        match self.get(key)? {
            Value::Bool(v) => Some(*v),
            _ => None,
        }
    }

    /// An enumerated value's value (its type aside).
    pub fn enumerated(&self, key: &[u8]) -> Option<&[u8]> {
        match self.get(key)? {
            Value::Enum(_, value) => Some(value),
            _ => None,
        }
    }
}

/// Descriptors nest at most this deep (Photoshop's styles need a few levels).
const MAX_DEPTH: usize = 32;

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let bytes = self.data.get(self.at..end)?;
        self.at = end;
        Some(bytes)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }

    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }

    fn key4(&mut self) -> Option<[u8; 4]> {
        self.take(4)?.try_into().ok()
    }

    /// A class or key id: a length then that many bytes, or 0 then 4 bytes.
    fn id(&mut self) -> Option<Vec<u8>> {
        let length = self.u32()? as usize;
        Some(self.take(if length == 0 { 4 } else { length })?.to_vec())
    }

    /// A Unicode string: its count of UTF-16 units, then the units.
    fn unicode(&mut self) -> Option<String> {
        let count = self.u32()? as usize;
        let bytes = self.take(count.checked_mul(2)?)?;
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|b| u16::from_be_bytes([b[0], b[1]]))
            .collect();
        Some(
            String::from_utf16_lossy(&units)
                .trim_end_matches('\0')
                .to_owned(),
        )
    }

    fn descriptor(&mut self, depth: usize) -> Option<Descriptor> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.unicode()?;
        let class = self.id()?;
        let count = self.u32()?;
        let mut items = Vec::new();
        for _ in 0..count {
            let key = self.id()?;
            let ty = self.key4()?;
            items.push((key, self.value(&ty, depth)?));
        }
        Some(Descriptor { class, items })
    }

    fn value(&mut self, ty: &[u8; 4], depth: usize) -> Option<Value> {
        Some(match ty {
            b"Objc" | b"GlbO" => Value::Object(self.descriptor(depth + 1)?),
            b"VlLs" => {
                let count = self.u32()?;
                let mut values = Vec::new();
                for _ in 0..count {
                    let ty = self.key4()?;
                    values.push(self.value(&ty, depth + 1)?);
                }
                Value::List(values)
            }
            b"doub" => Value::Double(self.f64()?),
            b"UntF" => {
                let unit = self.key4()?;
                Value::Unit(unit, self.f64()?)
            }
            b"UnFl" => {
                self.key4()?;
                let count = self.u32()? as usize;
                self.take(count.checked_mul(8)?)?;
                Value::Other
            }
            b"long" => Value::Integer(i64::from(self.u32()? as i32)),
            b"comp" => Value::Integer(i64::from_be_bytes(self.take(8)?.try_into().ok()?)),
            b"bool" => Value::Bool(self.take(1)?[0] != 0),
            b"TEXT" => Value::Text(self.unicode()?),
            b"enum" => {
                let ty = self.id()?;
                Value::Enum(ty, self.id()?)
            }
            b"type" | b"GlbC" => {
                self.unicode()?;
                self.id()?;
                Value::Other
            }
            b"alis" | b"tdta" => {
                let length = self.u32()? as usize;
                self.take(length)?;
                Value::Other
            }
            b"obj " => {
                let count = self.u32()?;
                for _ in 0..count {
                    let form = self.key4()?;
                    match &form {
                        b"prop" => {
                            self.unicode()?;
                            self.id()?;
                            self.id()?;
                        }
                        b"Clss" => {
                            self.unicode()?;
                            self.id()?;
                        }
                        b"Enmr" => {
                            self.unicode()?;
                            self.id()?;
                            self.id()?;
                            self.id()?;
                        }
                        b"rele" => {
                            self.unicode()?;
                            self.id()?;
                            self.u32()?;
                        }
                        b"Idnt" | b"indx" => {
                            self.u32()?;
                        }
                        b"name" => {
                            self.unicode()?;
                            self.id()?;
                            self.unicode()?;
                        }
                        _ => return None,
                    }
                }
                Value::Other
            }
            _ => return None,
        })
    }
}

/// The descriptor at the start of `data` (after its version, read by the caller).
pub(crate) fn read(data: &[u8]) -> Option<Descriptor> {
    Reader { data, at: 0 }.descriptor(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A descriptor as Photoshop writes it: an empty name, `class`, `items`.
    pub(crate) fn object(class: &[u8], items: &[(&[u8], &[u8; 4], Vec<u8>)]) -> Vec<u8> {
        let mut out = 0u32.to_be_bytes().to_vec();
        let id = |out: &mut Vec<u8>, id: &[u8]| {
            out.extend((if id.len() == 4 { 0 } else { id.len() as u32 }).to_be_bytes());
            out.extend(id);
        };
        id(&mut out, class);
        out.extend((items.len() as u32).to_be_bytes());
        for (key, ty, value) in items {
            id(&mut out, key);
            out.extend(*ty);
            out.extend(value);
        }
        out
    }

    #[test]
    fn nested_objects_lists_units_and_enums_are_read() {
        let color = object(
            b"RGBC",
            &[(b"Rd  ", b"doub", 12.5f64.to_be_bytes().to_vec())],
        );
        let mut unit = b"#Prc".to_vec();
        unit.extend(75.0f64.to_be_bytes());
        let mut mode = 0u32.to_be_bytes().to_vec();
        mode.extend(b"BlnM");
        mode.extend(0u32.to_be_bytes());
        mode.extend(b"Mltp");
        let mut list = 2u32.to_be_bytes().to_vec();
        list.extend(b"bool");
        list.push(1);
        list.extend(b"long");
        list.extend((-3i32).to_be_bytes());
        let data = object(
            b"null",
            &[
                (b"Clr ", b"Objc", color),
                (b"Opct", b"UntF", unit),
                (b"Md  ", b"enum", mode),
                (b"dropShadowMulti", b"VlLs", list),
                (b"enab", b"bool", vec![0]),
            ],
        );
        let d = read(&data).unwrap();
        assert_eq!(d.class, b"null");
        assert_eq!(d.object(b"Clr ").unwrap().number(b"Rd  "), Some(12.5));
        assert_eq!(d.number(b"Opct"), Some(75.0));
        assert_eq!(d.enumerated(b"Md  "), Some(&b"Mltp"[..]));
        assert_eq!(
            d.list(b"dropShadowMulti").unwrap(),
            [Value::Bool(true), Value::Integer(-3)]
        );
        assert_eq!(d.bool(b"enab"), Some(false));
    }

    #[test]
    fn damaged_or_unknown_data_ends_the_descriptor() {
        let good = object(b"null", &[(b"enab", b"bool", vec![1])]);
        for cut in 0..good.len() {
            assert_eq!(read(&good[..cut]), None, "cut at {cut}");
        }
        assert_eq!(read(&object(b"null", &[(b"what", b"????", vec![])])), None);
        // Nested too deep: refused, not a stack overflow.
        let mut deep = object(b"null", &[]);
        for _ in 0..40 {
            deep = object(b"null", &[(b"next", b"Objc", deep)]);
        }
        assert_eq!(read(&deep), None);
        // A huge length is not an allocation.
        let mut huge = 0u32.to_be_bytes().to_vec();
        huge.extend(u32::MAX.to_be_bytes());
        assert_eq!(read(&huge), None);
    }
}
