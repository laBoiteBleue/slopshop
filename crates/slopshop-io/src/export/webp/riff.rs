//! The WebP container (RIFF) around libwebp's lossy bitstream: libwebp writes `RIFF`, then
//! either `VP8 ` alone or `VP8X`, `ALPH`, `VP8 `, and cannot add an ICC profile itself (that
//! takes libwebpmux, and one more copy of the file). We keep its `ALPH` and `VP8 ` chunks and
//! write our own extended header around them, with the profile:
//!
//! `RIFF` size `WEBP` · `VP8X` (ICC and alpha flags, canvas size) · `ICCP` · [`ALPH`] · `VP8 `
//!
//! Chunks are little-endian `fourcc, u32 size, data`, padded to an even size (the padding is
//! not counted in the size).

use super::super::ExportError;

/// A chunk: its fourcc and its data (without the padding byte).
pub(super) type Chunk<'a> = ([u8; 4], &'a [u8]);

/// Chunks of a RIFF WebP file, in file order.
pub(super) fn chunks(file: &[u8]) -> Result<Vec<Chunk<'_>>, ExportError> {
    let invalid = |what: &str| ExportError::Encode(format!("invalid WebP from libwebp: {what}"));
    if file.len() < 12 || &file[..4] != b"RIFF" || &file[8..12] != b"WEBP" {
        return Err(invalid("no RIFF WEBP header"));
    }
    let declared = u32::from_le_bytes([file[4], file[5], file[6], file[7]]) as usize;
    let end = declared
        .checked_add(8)
        .filter(|&end| end <= file.len())
        .ok_or_else(|| invalid("truncated file"))?;
    let mut chunks = Vec::new();
    let mut at = 12;
    while at < end {
        let header = file
            .get(at..at + 8)
            .ok_or_else(|| invalid("truncated chunk header"))?;
        let fourcc = [header[0], header[1], header[2], header[3]];
        let len = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let data = at
            .checked_add(8 + len)
            .and_then(|stop| file.get(at + 8..stop))
            .ok_or_else(|| invalid("truncated chunk"))?;
        chunks.push((fourcc, data));
        at += 8 + len + len % 2;
    }
    Ok(chunks)
}

/// A complete WebP file: `bitstream` (a file written by libwebp) with `icc` added.
pub(super) fn with_icc_profile(
    bitstream: &[u8],
    width: u32,
    height: u32,
    icc: &[u8],
) -> Result<Vec<u8>, ExportError> {
    let chunks = chunks(bitstream)?;
    let find = |fourcc: &[u8; 4]| chunks.iter().find(|(f, _)| f == fourcc).map(|(_, d)| *d);
    let vp8 = find(b"VP8 ")
        .ok_or_else(|| ExportError::Encode("libwebp wrote no VP8 chunk".to_owned()))?;
    let alpha = find(b"ALPH");

    let mut flags = 0x20; // ICC profile
    if alpha.is_some() {
        flags |= 0x10;
    }
    let mut vp8x = vec![flags, 0, 0, 0];
    vp8x.extend_from_slice(&(width - 1).to_le_bytes()[..3]);
    vp8x.extend_from_slice(&(height - 1).to_le_bytes()[..3]);

    let mut body: Vec<Chunk<'_>> = vec![(*b"VP8X", &vp8x), (*b"ICCP", icc)];
    if let Some(alpha) = alpha {
        body.push((*b"ALPH", alpha));
    }
    body.push((*b"VP8 ", vp8));

    let chunk_len = |data: &[u8]| 8 + data.len() + data.len() % 2;
    let riff_len = 4 + body.iter().map(|(_, d)| chunk_len(d)).sum::<usize>();
    let riff_size = u32::try_from(riff_len)
        .map_err(|_| ExportError::Encode("the WebP file would exceed 4 GiB".to_owned()))?;
    let mut file = Vec::with_capacity(8 + riff_len);
    file.extend_from_slice(b"RIFF");
    file.extend_from_slice(&riff_size.to_le_bytes());
    file.extend_from_slice(b"WEBP");
    for (fourcc, data) in body {
        file.extend_from_slice(&fourcc);
        // Fits: the whole file size does.
        file.extend_from_slice(&(data.len() as u32).to_le_bytes());
        file.extend_from_slice(data);
        if data.len() % 2 == 1 {
            file.push(0);
        }
    }
    Ok(file)
}
