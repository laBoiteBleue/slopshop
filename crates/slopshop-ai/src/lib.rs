//! SlopShop's AI helper (ADR 0025): the models run in a separate `slopshop-ai` process, through
//! ONNX Runtime, so that their native libraries, their crashes and their VRAM stay out of the
//! editor. This library is the editor's side: the protocol and a client that starts the helper
//! and talks to it over its standard input and output. It depends on nothing.
//!
//! Protocol: frames of a little-endian `u32` length then that many bytes. A request starts with
//! an operation code; a response with a status (0: done, 1: failed, then a UTF-8 message).
//! Images and masks travel as raw samples, never as text.

use std::io::{self, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// The helper's protocol version, checked by [`Request::Hello`].
pub const PROTOCOL_VERSION: u32 = 1;

/// Largest frame accepted (an RGB image of 2048² is 12 MB).
pub const MAX_FRAME: usize = 64 << 20;

/// Largest image side SAM is given (the helper resizes to its 1024² input).
pub const MAX_IMAGE_SIDE: u32 = 2048;

/// Side of the masks SAM returns.
pub const MASK_SIDE: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// The helper's protocol version and the execution provider it runs on.
    Hello,
    /// Encode an image for SAM (8-bit RGB, `width × height`, row-major), kept under `key` for
    /// the prompts that follow; the previous image is forgotten.
    SamEncode {
        key: u64,
        width: u32,
        height: u32,
        rgb: Vec<u8>,
    },
    /// SAM's mask for prompts on the image encoded under `key`: points in its pixels, positive
    /// or negative, and an optional box `[x0, y0, x1, y1]`.
    SamDecode {
        key: u64,
        points: Vec<Point>,
        boxed: Option<[f32; 4]>,
    },
    /// Stop.
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    /// Part of the object (a click, a brush stroke) or not (Alt).
    pub positive: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    Hello {
        version: u32,
        /// `cuda`, `directml` or `cpu`.
        provider: String,
    },
    Done,
    /// SAM's best mask: [`MASK_SIDE`]² logits over the whole encoded image (positive: inside),
    /// and its predicted quality.
    SamMask {
        logits: Vec<f32>,
        score: f32,
    },
    Failed(String),
}

#[derive(Debug)]
pub enum ProtocolError {
    Io(io::Error),
    /// A frame or a field that does not make sense.
    Malformed(&'static str),
    /// The helper answered something else than the request expects.
    Unexpected,
    /// The helper reported an error.
    Failed(String),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtocolError::Io(e) => write!(f, "AI helper: {e}"),
            ProtocolError::Malformed(what) => write!(f, "AI helper: malformed {what}"),
            ProtocolError::Unexpected => write!(f, "AI helper: unexpected answer"),
            ProtocolError::Failed(message) => write!(f, "AI helper: {message}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

impl From<io::Error> for ProtocolError {
    fn from(e: io::Error) -> Self {
        ProtocolError::Io(e)
    }
}

const OP_HELLO: u8 = 1;
const OP_SAM_ENCODE: u8 = 2;
const OP_SAM_DECODE: u8 = 3;
const OP_QUIT: u8 = 4;

const RESPONSE_HELLO: u8 = 0x81;
const RESPONSE_DONE: u8 = 0x82;
const RESPONSE_SAM_MASK: u8 = 0x83;
const RESPONSE_FAILED: u8 = 0xff;

/// Reads little-endian fields from a frame.
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8], ProtocolError> {
        if self.0.len() < n {
            return Err(ProtocolError::Malformed("frame"));
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn u8(&mut self) -> Result<u8, ProtocolError> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, ProtocolError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, ProtocolError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    fn f32(&mut self) -> Result<f32, ProtocolError> {
        Ok(f32::from_bits(self.u32()?))
    }

    fn end(&self) -> Result<(), ProtocolError> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(ProtocolError::Malformed("frame length"))
        }
    }
}

impl Request {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Request::Hello => out.push(OP_HELLO),
            Request::SamEncode {
                key,
                width,
                height,
                rgb,
            } => {
                out.push(OP_SAM_ENCODE);
                out.extend_from_slice(&key.to_le_bytes());
                out.extend_from_slice(&width.to_le_bytes());
                out.extend_from_slice(&height.to_le_bytes());
                out.extend_from_slice(rgb);
            }
            Request::SamDecode { key, points, boxed } => {
                out.push(OP_SAM_DECODE);
                out.extend_from_slice(&key.to_le_bytes());
                out.extend_from_slice(&(points.len() as u32).to_le_bytes());
                for p in points {
                    out.extend_from_slice(&p.x.to_le_bytes());
                    out.extend_from_slice(&p.y.to_le_bytes());
                    out.push(u8::from(p.positive));
                }
                match boxed {
                    Some(b) => {
                        out.push(1);
                        for v in b {
                            out.extend_from_slice(&v.to_le_bytes());
                        }
                    }
                    None => out.push(0),
                }
            }
            Request::Quit => out.push(OP_QUIT),
        }
        out
    }

    pub fn decode(frame: &[u8]) -> Result<Self, ProtocolError> {
        let mut f = Fields(frame);
        let request = match f.u8()? {
            OP_HELLO => Request::Hello,
            OP_SAM_ENCODE => {
                let key = f.u64()?;
                let (width, height) = (f.u32()?, f.u32()?);
                if width == 0 || height == 0 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
                    return Err(ProtocolError::Malformed("image size"));
                }
                let rgb = f.take(width as usize * height as usize * 3)?.to_vec();
                Request::SamEncode {
                    key,
                    width,
                    height,
                    rgb,
                }
            }
            OP_SAM_DECODE => {
                let key = f.u64()?;
                let count = f.u32()? as usize;
                if count > 4096 {
                    return Err(ProtocolError::Malformed("point count"));
                }
                let points = (0..count)
                    .map(|_| {
                        Ok(Point {
                            x: f.f32()?,
                            y: f.f32()?,
                            positive: f.u8()? != 0,
                        })
                    })
                    .collect::<Result<Vec<_>, ProtocolError>>()?;
                let boxed = match f.u8()? {
                    0 => None,
                    _ => Some([f.f32()?, f.f32()?, f.f32()?, f.f32()?]),
                };
                Request::SamDecode { key, points, boxed }
            }
            OP_QUIT => Request::Quit,
            _ => return Err(ProtocolError::Malformed("operation")),
        };
        f.end()?;
        Ok(request)
    }
}

impl Response {
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Response::Hello { version, provider } => {
                out.push(RESPONSE_HELLO);
                out.extend_from_slice(&version.to_le_bytes());
                out.extend_from_slice(provider.as_bytes());
            }
            Response::Done => out.push(RESPONSE_DONE),
            Response::SamMask { logits, score } => {
                out.push(RESPONSE_SAM_MASK);
                out.extend_from_slice(&score.to_le_bytes());
                for v in logits {
                    out.extend_from_slice(&v.to_le_bytes());
                }
            }
            Response::Failed(message) => {
                out.push(RESPONSE_FAILED);
                out.extend_from_slice(message.as_bytes());
            }
        }
        out
    }

    pub fn decode(frame: &[u8]) -> Result<Self, ProtocolError> {
        let mut f = Fields(frame);
        let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let response = match f.u8()? {
            RESPONSE_HELLO => {
                let version = f.u32()?;
                Response::Hello {
                    version,
                    provider: text(f.take(f.0.len())?),
                }
            }
            RESPONSE_DONE => Response::Done,
            RESPONSE_SAM_MASK => {
                let score = f.f32()?;
                let logits = (0..MASK_SIDE * MASK_SIDE)
                    .map(|_| f.f32())
                    .collect::<Result<Vec<_>, _>>()?;
                Response::SamMask { logits, score }
            }
            RESPONSE_FAILED => Response::Failed(text(f.take(f.0.len())?)),
            _ => return Err(ProtocolError::Malformed("response")),
        };
        f.end()?;
        Ok(response)
    }
}

/// Write one frame.
pub fn write_frame(out: &mut impl Write, body: &[u8]) -> io::Result<()> {
    let len = u32::try_from(body.len()).map_err(|_| io::Error::other("frame too large"))?;
    out.write_all(&len.to_le_bytes())?;
    out.write_all(body)?;
    out.flush()
}

/// Read one frame; `None` at the end of the stream.
pub fn read_frame(input: &mut impl Read) -> Result<Option<Vec<u8>>, ProtocolError> {
    let mut len = [0u8; 4];
    match input.read_exact(&mut len) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len > MAX_FRAME {
        return Err(ProtocolError::Malformed("frame length"));
    }
    let mut body = vec![0u8; len];
    input.read_exact(&mut body)?;
    Ok(Some(body))
}

/// How to start the helper.
#[derive(Debug, Clone)]
pub struct Launch<'a> {
    /// The `slopshop-ai` executable.
    pub executable: &'a Path,
    /// The ONNX Runtime library to load.
    pub runtime: &'a Path,
    /// Where the models are (one folder per repository, as downloaded).
    pub models: &'a Path,
    /// `auto`, `cuda`, `directml` or `cpu`.
    pub provider: &'a str,
    /// Folders added to the helper's library search path (NVIDIA's runtime libraries).
    pub library_paths: &'a [&'a Path],
}

/// A running helper.
pub struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
    /// Its execution provider, from its greeting.
    pub provider: String,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("pid", &self.child.id())
            .field("provider", &self.provider)
            .finish()
    }
}

impl Client {
    /// Start the helper and greet it (its protocol version must match).
    pub fn start(launch: &Launch<'_>) -> Result<Self, ProtocolError> {
        let mut command = Command::new(launch.executable);
        command
            .arg("--runtime")
            .arg(launch.runtime)
            .arg("--models")
            .arg(launch.models)
            .arg("--provider")
            .arg(launch.provider)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if !launch.library_paths.is_empty() {
            let mut paths: Vec<std::path::PathBuf> = launch
                .library_paths
                .iter()
                .map(|p| p.to_path_buf())
                .collect();
            if let Some(existing) = std::env::var_os("PATH") {
                paths.extend(std::env::split_paths(&existing));
            }
            if let Ok(joined) = std::env::join_paths(paths) {
                command.env("PATH", joined);
            }
        }
        let mut child = command.spawn()?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(ProtocolError::Malformed("helper pipes"));
        };
        let mut client = Self {
            child,
            stdin,
            stdout,
            provider: String::new(),
        };
        match client.call(&Request::Hello)? {
            Response::Hello { version, provider } if version == PROTOCOL_VERSION => {
                client.provider = provider;
                Ok(client)
            }
            Response::Hello { .. } => Err(ProtocolError::Malformed("protocol version")),
            _ => Err(ProtocolError::Unexpected),
        }
    }

    fn call(&mut self, request: &Request) -> Result<Response, ProtocolError> {
        write_frame(&mut self.stdin, &request.encode())?;
        let frame = read_frame(&mut self.stdout)?
            .ok_or(ProtocolError::Io(io::ErrorKind::UnexpectedEof.into()))?;
        match Response::decode(&frame)? {
            Response::Failed(message) => Err(ProtocolError::Failed(message)),
            response => Ok(response),
        }
    }

    /// Encode an image for SAM under `key`.
    pub fn sam_encode(
        &mut self,
        key: u64,
        width: u32,
        height: u32,
        rgb: Vec<u8>,
    ) -> Result<(), ProtocolError> {
        let request = Request::SamEncode {
            key,
            width,
            height,
            rgb,
        };
        match self.call(&request)? {
            Response::Done => Ok(()),
            _ => Err(ProtocolError::Unexpected),
        }
    }

    /// SAM's mask for `points` (and a box) on the image encoded under `key`.
    pub fn sam_decode(
        &mut self,
        key: u64,
        points: Vec<Point>,
        boxed: Option<[f32; 4]>,
    ) -> Result<(Vec<f32>, f32), ProtocolError> {
        match self.call(&Request::SamDecode { key, points, boxed })? {
            Response::SamMask { logits, score } => Ok((logits, score)),
            _ => Err(ProtocolError::Unexpected),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Ask it to stop; if it does not, it is killed.
        let _ = write_frame(&mut self.stdin, &Request::Quit.encode());
        if !matches!(self.child.try_wait(), Ok(Some(_))) {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if !matches!(self.child.try_wait(), Ok(Some(_))) {
                let _ = self.child.kill();
            }
        }
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let requests = [
            Request::Hello,
            Request::SamEncode {
                key: 7,
                width: 2,
                height: 1,
                rgb: vec![1, 2, 3, 4, 5, 6],
            },
            Request::SamDecode {
                key: 7,
                points: vec![
                    Point {
                        x: 1.5,
                        y: 2.0,
                        positive: true,
                    },
                    Point {
                        x: 0.0,
                        y: 9.0,
                        positive: false,
                    },
                ],
                boxed: Some([0.0, 1.0, 2.0, 3.0]),
            },
            Request::SamDecode {
                key: 1,
                points: Vec::new(),
                boxed: None,
            },
            Request::Quit,
        ];
        for request in requests {
            assert_eq!(Request::decode(&request.encode()).unwrap(), request);
        }
    }

    #[test]
    fn responses_round_trip() {
        let responses = [
            Response::Hello {
                version: PROTOCOL_VERSION,
                provider: "cuda".into(),
            },
            Response::Done,
            Response::SamMask {
                logits: (0..MASK_SIDE * MASK_SIDE).map(|i| i as f32 - 3.0).collect(),
                score: 0.9,
            },
            Response::Failed("no model".into()),
        ];
        for response in responses {
            assert_eq!(Response::decode(&response.encode()).unwrap(), response);
        }
    }

    #[test]
    fn malformed_frames_are_errors() {
        assert!(Request::decode(&[]).is_err());
        assert!(Request::decode(&[99]).is_err());
        // An image larger than allowed, or shorter than announced.
        let mut big = vec![OP_SAM_ENCODE];
        big.extend_from_slice(&1u64.to_le_bytes());
        big.extend_from_slice(&(MAX_IMAGE_SIDE + 1).to_le_bytes());
        big.extend_from_slice(&1u32.to_le_bytes());
        assert!(Request::decode(&big).is_err());
        let mut short = Request::SamEncode {
            key: 1,
            width: 4,
            height: 4,
            rgb: vec![0; 48],
        }
        .encode();
        short.pop();
        assert!(Request::decode(&short).is_err());
        // Trailing bytes.
        let mut long = Request::Hello.encode();
        long.push(0);
        assert!(Request::decode(&long).is_err());
        assert!(Response::decode(&[RESPONSE_SAM_MASK, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn frames_round_trip() {
        let mut buffer = Vec::new();
        write_frame(&mut buffer, b"abc").unwrap();
        write_frame(&mut buffer, b"").unwrap();
        let mut input = buffer.as_slice();
        assert_eq!(read_frame(&mut input).unwrap(), Some(b"abc".to_vec()));
        assert_eq!(read_frame(&mut input).unwrap(), Some(Vec::new()));
        assert_eq!(read_frame(&mut input).unwrap(), None);
        let huge = ((MAX_FRAME + 1) as u32).to_le_bytes();
        assert!(read_frame(&mut huge.as_slice()).is_err());
    }
}
