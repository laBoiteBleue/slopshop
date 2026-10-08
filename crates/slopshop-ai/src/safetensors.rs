//! Reading `.safetensors` files, the format model weights are published in: an 8-byte header
//! length, a JSON header (each tensor's type, shape and byte range), then the data. The header's
//! JSON is a fixed shape, read by a small parser of its own (the crate depends on nothing).

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

/// Element types met in model weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dtype {
    F32,
    F16,
    Bf16,
    I64,
}

impl Dtype {
    pub fn size(self) -> usize {
        match self {
            Dtype::F32 => 4,
            Dtype::F16 | Dtype::Bf16 => 2,
            Dtype::I64 => 8,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TensorInfo {
    pub dtype: Dtype,
    pub shape: Vec<usize>,
    /// Byte range in the data section.
    pub start: usize,
    pub end: usize,
}

impl TensorInfo {
    pub fn len(&self) -> usize {
        self.shape.iter().product()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// An open `.safetensors` file: its header, the data read on demand.
#[derive(Debug)]
pub struct SafeTensors {
    file: File,
    data_start: u64,
    pub tensors: HashMap<String, TensorInfo>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

impl SafeTensors {
    pub fn open(path: &Path) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let mut len = [0; 8];
        file.read_exact(&mut len)?;
        let len = u64::from_le_bytes(len);
        if len > 100 << 20 {
            return Err(invalid("safetensors header too large"));
        }
        let mut header = vec![0; len as usize];
        file.read_exact(&mut header)?;
        let header = std::str::from_utf8(&header).map_err(|_| invalid("header not UTF-8"))?;
        let tensors = parse_header(header)?;
        let size = file.metadata()?.len() - 8 - len;
        if tensors.values().any(|t| t.end as u64 > size) {
            return Err(invalid("tensor beyond the end of the file"));
        }
        Ok(Self {
            file,
            data_start: 8 + len,
            tensors,
        })
    }

    pub fn info(&self, name: &str) -> io::Result<&TensorInfo> {
        self.tensors
            .get(name)
            .ok_or_else(|| invalid(format!("missing tensor {name}")))
    }

    /// The raw little-endian bytes of a tensor.
    pub fn bytes(&mut self, name: &str) -> io::Result<Vec<u8>> {
        let info = self.info(name)?.clone();
        let mut out = vec![0; info.end - info.start];
        self.file
            .seek(SeekFrom::Start(self.data_start + info.start as u64))?;
        self.file.read_exact(&mut out)?;
        Ok(out)
    }

    /// A tensor as single precision values (from f32, f16 or bf16).
    pub fn f32s(&mut self, name: &str) -> io::Result<Vec<f32>> {
        let dtype = self.info(name)?.dtype;
        let raw = self.bytes(name)?;
        Ok(match dtype {
            Dtype::F32 => raw
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| f32::from_le_bytes(*b))
                .collect(),
            Dtype::Bf16 => raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| crate::numeric::bf16_to_f32(u16::from_le_bytes(*b)))
                .collect(),
            Dtype::F16 => raw
                .as_chunks::<2>()
                .0
                .iter()
                .map(|b| crate::numeric::f16_to_f32(u16::from_le_bytes(*b)))
                .collect(),
            Dtype::I64 => return Err(invalid(format!("{name} is not a float tensor"))),
        })
    }
}

/// The header's JSON: `{"name": {"dtype": "BF16", "shape": [..], "data_offsets": [a, b]}, …}`,
/// plus an optional `__metadata__` object of strings (skipped).
fn parse_header(json: &str) -> io::Result<HashMap<String, TensorInfo>> {
    let mut p = Parser {
        s: json.as_bytes(),
        at: 0,
    };
    let mut out = HashMap::new();
    p.expect(b'{')?;
    if p.peek() == Some(b'}') {
        return Ok(out);
    }
    loop {
        let name = p.string()?;
        p.expect(b':')?;
        if name == "__metadata__" {
            p.skip_string_object()?;
        } else {
            let (mut dtype, mut shape, mut offsets) = (None, None, None);
            p.expect(b'{')?;
            loop {
                let field = p.string()?;
                p.expect(b':')?;
                match field.as_str() {
                    "dtype" => {
                        dtype = Some(match p.string()?.as_str() {
                            "F32" => Dtype::F32,
                            "F16" => Dtype::F16,
                            "BF16" => Dtype::Bf16,
                            "I64" => Dtype::I64,
                            other => return Err(invalid(format!("unsupported dtype {other}"))),
                        })
                    }
                    "shape" => shape = Some(p.numbers()?),
                    "data_offsets" => offsets = Some(p.numbers()?),
                    _ => return Err(invalid(format!("unexpected field {field}"))),
                }
                if !p.comma_or(b'}')? {
                    break;
                }
            }
            let (Some(dtype), Some(shape), Some(offsets)) = (dtype, shape, offsets) else {
                return Err(invalid(format!("incomplete entry {name}")));
            };
            let [start, end] = offsets[..] else {
                return Err(invalid("data_offsets must hold two numbers"));
            };
            let info = TensorInfo {
                dtype,
                shape,
                start,
                end,
            };
            if end < start || end - start != info.len() * dtype.size() {
                return Err(invalid(format!("{name}: size does not match its shape")));
            }
            out.insert(name, info);
        }
        if !p.comma_or(b'}')? {
            break;
        }
    }
    Ok(out)
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn skip_space(&mut self) {
        while self.at < self.s.len() && self.s[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn peek(&mut self) -> Option<u8> {
        self.skip_space();
        self.s.get(self.at).copied()
    }

    fn expect(&mut self, c: u8) -> io::Result<()> {
        if self.peek() == Some(c) {
            self.at += 1;
            Ok(())
        } else {
            Err(invalid(format!("expected '{}' at {}", c as char, self.at)))
        }
    }

    /// After a value: true on a comma (more follows), false on `close`.
    fn comma_or(&mut self, close: u8) -> io::Result<bool> {
        match self.peek() {
            Some(b',') => {
                self.at += 1;
                Ok(true)
            }
            Some(c) if c == close => {
                self.at += 1;
                Ok(false)
            }
            _ => Err(invalid(format!("expected ',' or '{}'", close as char))),
        }
    }

    fn string(&mut self) -> io::Result<String> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            let c = *self
                .s
                .get(self.at)
                .ok_or_else(|| invalid("unterminated string"))?;
            self.at += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.s.get(self.at).ok_or_else(|| invalid("bad escape"))?;
                    self.at += 1;
                    match e {
                        b'"' | b'\\' | b'/' => out.push(e),
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'u' => {
                            let hex = self
                                .s
                                .get(self.at..self.at + 4)
                                .ok_or_else(|| invalid("bad escape"))?;
                            let code = u32::from_str_radix(
                                std::str::from_utf8(hex).map_err(|_| invalid("bad escape"))?,
                                16,
                            )
                            .map_err(|_| invalid("bad escape"))?;
                            self.at += 4;
                            let ch = char::from_u32(code).unwrap_or('\u{fffd}');
                            out.extend_from_slice(ch.to_string().as_bytes());
                        }
                        _ => return Err(invalid("bad escape")),
                    }
                }
                _ => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| invalid("string not UTF-8"))
    }

    fn numbers(&mut self) -> io::Result<Vec<usize>> {
        self.expect(b'[')?;
        let mut out = Vec::new();
        if self.peek() == Some(b']') {
            self.at += 1;
            return Ok(out);
        }
        loop {
            self.skip_space();
            let start = self.at;
            while self.at < self.s.len() && self.s[self.at].is_ascii_digit() {
                self.at += 1;
            }
            let digits = std::str::from_utf8(&self.s[start..self.at]).unwrap_or("");
            out.push(digits.parse().map_err(|_| invalid("expected a number"))?);
            if !self.comma_or(b']')? {
                break;
            }
        }
        Ok(out)
    }

    fn skip_string_object(&mut self) -> io::Result<()> {
        self.expect(b'{')?;
        if self.peek() == Some(b'}') {
            self.at += 1;
            return Ok(());
        }
        loop {
            self.string()?;
            self.expect(b':')?;
            self.string()?;
            if !self.comma_or(b'}')? {
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(header: &str, data: &[u8]) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "slopshop-safetensors-{}-{}.safetensors",
            std::process::id(),
            header.len()
        ));
        let mut bytes = (header.len() as u64).to_le_bytes().to_vec();
        bytes.extend_from_slice(header.as_bytes());
        bytes.extend_from_slice(data);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn reads_tensors_of_each_type() {
        let header = r#"{"__metadata__":{"format":"pt"},"a":{"dtype":"F32","shape":[2],"data_offsets":[0,8]},
            "b": {"dtype":"BF16","shape":[1,2],"data_offsets":[8,12]}, "c":{"dtype":"F16","shape":[],"data_offsets":[12,14]}}"#;
        let mut data = Vec::new();
        data.extend_from_slice(&1.5f32.to_le_bytes());
        data.extend_from_slice(&(-2.0f32).to_le_bytes());
        data.extend_from_slice(&0x3f80u16.to_le_bytes()); // 1.0
        data.extend_from_slice(&0xc000u16.to_le_bytes()); // -2.0
        data.extend_from_slice(&0x3c00u16.to_le_bytes()); // 1.0 in half precision
        let path = file(header, &data);
        let mut st = SafeTensors::open(&path).unwrap();
        assert_eq!(st.f32s("a").unwrap(), vec![1.5, -2.0]);
        assert_eq!(st.f32s("b").unwrap(), vec![1.0, -2.0]);
        assert_eq!(st.info("b").unwrap().shape, vec![1, 2]);
        assert_eq!(st.f32s("c").unwrap(), vec![1.0]);
        assert!(st.f32s("missing").is_err());
        drop(st);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_inconsistent_headers() {
        for header in [
            r#"{"a":{"dtype":"F32","shape":[3],"data_offsets":[0,8]}}"#, // size mismatch
            r#"{"a":{"dtype":"F32","shape":[2],"data_offsets":[0,80]}}"#, // past the end
            r#"{"a":{"dtype":"U8","shape":[2],"data_offsets":[0,2]}}"#,
            r#"{"a":{"dtype":"F32","shape":[2]"#,
        ] {
            let path = file(header, &[0; 8]);
            assert!(SafeTensors::open(&path).is_err(), "{header}");
            std::fs::remove_file(path).unwrap();
        }
    }
}
