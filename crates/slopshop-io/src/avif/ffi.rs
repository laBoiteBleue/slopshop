//! AV1 decoding through rav1d (the Rust port of dav1d), whose API is dav1d's C one: the only
//! `unsafe` code of the AVIF import. [`decode`] decodes one AVIF item (one temporal unit) and
//! copies the planes of its final picture into memory we own; the decoder and its pictures
//! never outlive the call (a guard releases them on every path).
#![allow(unsafe_code)]

use std::mem::MaybeUninit;
use std::ptr::NonNull;

use rav1d::Dav1dResult;
use rav1d::include::dav1d::data::Dav1dData;
use rav1d::include::dav1d::dav1d::{Dav1dContext, Dav1dSettings};
use rav1d::include::dav1d::headers::{
    DAV1D_PIXEL_LAYOUT_I400, DAV1D_PIXEL_LAYOUT_I420, DAV1D_PIXEL_LAYOUT_I422,
    DAV1D_PIXEL_LAYOUT_I444,
};
use rav1d::include::dav1d::picture::Dav1dPicture;
use rav1d::src::lib::{
    dav1d_close, dav1d_data_create, dav1d_data_unref, dav1d_default_settings, dav1d_get_picture,
    dav1d_open, dav1d_picture_unref, dav1d_send_data,
};

/// How chroma is sampled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Chroma {
    /// No chroma planes.
    Monochrome,
    /// Half width and height.
    I420,
    /// Half width.
    I422,
    I444,
}

/// A decoded picture: its planes (Y, then U and V unless monochrome), every sample widened to
/// 16 bits, rows without padding, and the color description of its sequence header.
pub(super) struct Picture {
    pub width: u32,
    pub height: u32,
    /// Bits per sample: 8, 10 or 12.
    pub depth: u8,
    pub chroma: Chroma,
    pub planes: Vec<Vec<u16>>,
    /// H.273 code points.
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
}

impl Picture {
    /// The size of plane `i`.
    pub fn plane_size(&self, i: usize) -> (usize, usize) {
        let (w, h) = (self.width as usize, self.height as usize);
        match (i, self.chroma) {
            (0, _) | (_, Chroma::I444) | (_, Chroma::Monochrome) => (w, h),
            (_, Chroma::I420) => (w.div_ceil(2), h.div_ceil(2)),
            (_, Chroma::I422) => (w.div_ceil(2), h),
        }
    }
}

/// The decoder and the pictures it handed out, released when dropped.
struct Session {
    ctx: Option<Dav1dContext>,
    pictures: Vec<Dav1dPicture>,
}

impl Drop for Session {
    fn drop(&mut self) {
        for picture in &mut self.pictures {
            // SAFETY: every picture was filled by `dav1d_get_picture`, and is released once.
            unsafe { dav1d_picture_unref(Some(NonNull::from(picture))) };
        }
        // SAFETY: `ctx` was set by `dav1d_open` (or is `None`, which dav1d_close ignores).
        unsafe { dav1d_close(Some(NonNull::from(&mut self.ctx))) };
    }
}

fn check(result: Dav1dResult, what: &str) -> Result<(), String> {
    if result.0 == 0 {
        Ok(())
    } else {
        Err(format!(
            "AV1 {what} failed (error {})",
            result.0.unsigned_abs()
        ))
    }
}

/// Decode the AV1 temporal unit `data` (an AVIF item), refusing frames of more than
/// `max_pixels`. A layered item gives one picture per layer: the last one is the image.
pub(super) fn decode(data: &[u8], max_pixels: u32) -> Result<Picture, String> {
    let mut session = Session {
        ctx: None,
        pictures: Vec::new(),
    };
    let mut settings = MaybeUninit::<Dav1dSettings>::uninit();
    // SAFETY: `dav1d_default_settings` writes a whole `Dav1dSettings`, so it is initialized
    // afterwards.
    let settings = unsafe {
        dav1d_default_settings(NonNull::from(&mut settings).cast());
        settings.assume_init_mut()
    };
    // One thread per logical CPU; a still image has no frames to pipeline.
    settings.n_threads = 0;
    settings.max_frame_delay = 1;
    // Film grain is part of the image (libavif applies it too).
    settings.apply_grain = 1;
    settings.frame_size_limit = max_pixels;
    // SAFETY: both pointers are valid for the call; `ctx` receives the context, closed by the
    // session's `Drop`.
    let opened = unsafe {
        dav1d_open(
            Some(NonNull::from(&mut session.ctx)),
            Some(NonNull::from(settings)),
        )
    };
    check(opened, "decoder setup")?;
    let ctx = session.ctx.ok_or("AV1 decoder setup failed")?;

    let mut input = Dav1dData::default();
    // SAFETY: `input` is a valid, empty `Dav1dData`; the returned buffer holds `data.len()`
    // bytes owned by `input`.
    let buffer = unsafe { dav1d_data_create(Some(NonNull::from(&mut input)), data.len()) };
    let Some(buffer) = NonNull::new(buffer) else {
        return Err("AV1 input buffer allocation failed".to_owned());
    };
    // SAFETY: `buffer` holds `data.len()` writable bytes that do not overlap `data`.
    unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), buffer.as_ptr(), data.len()) };
    let sent = loop {
        // SAFETY: `ctx` is open and `input` holds the data created above.
        let result = unsafe { dav1d_send_data(Some(ctx), Some(NonNull::from(&mut input))) };
        if result.0 == 0 {
            break Ok(());
        }
        // The decoder holds a picture of an earlier layer: take it, then send the rest.
        let mut pending = Dav1dPicture::default();
        // SAFETY: `ctx` is open and `pending` is a valid, empty picture.
        let got = unsafe { dav1d_get_picture(Some(ctx), Some(NonNull::from(&mut pending))) };
        if got.0 != 0 {
            break check(result, "decoding");
        }
        session.pictures.push(pending);
    };
    // SAFETY: `input` was created by `dav1d_data_create`; the decoder took what it needed.
    unsafe { dav1d_data_unref(Some(NonNull::from(&mut input))) };
    sent?;
    loop {
        let mut picture = Dav1dPicture::default();
        // SAFETY: as above.
        let got = unsafe { dav1d_get_picture(Some(ctx), Some(NonNull::from(&mut picture))) };
        if got.0 != 0 {
            break;
        }
        session.pictures.push(picture);
    }
    let picture = session.pictures.last().ok_or("no AV1 picture decoded")?;
    copy(picture)
}

/// The planes and color description of `picture`, copied.
fn copy(picture: &Dav1dPicture) -> Result<Picture, String> {
    let p = &picture.p;
    let chroma = match p.layout {
        DAV1D_PIXEL_LAYOUT_I400 => Chroma::Monochrome,
        DAV1D_PIXEL_LAYOUT_I420 => Chroma::I420,
        DAV1D_PIXEL_LAYOUT_I422 => Chroma::I422,
        DAV1D_PIXEL_LAYOUT_I444 => Chroma::I444,
        _ => return Err("unknown AV1 chroma layout".to_owned()),
    };
    let depth = match p.bpc {
        8 => 8,
        10 => 10,
        12 => 12,
        other => return Err(format!("AV1 depth of {other} bits")),
    };
    let (Ok(width), Ok(height)) = (u32::try_from(p.w), u32::try_from(p.h)) else {
        return Err("invalid AV1 picture size".to_owned());
    };
    if width == 0 || height == 0 {
        return Err("empty AV1 picture".to_owned());
    }
    let Some(header) = picture.seq_hdr else {
        return Err("AV1 picture without its sequence header".to_owned());
    };
    // SAFETY: the sequence header lives as long as the picture, which we hold.
    let header = unsafe { header.as_ref() };
    let mut out = Picture {
        width,
        height,
        depth,
        chroma,
        planes: Vec::new(),
        primaries: u8::try_from(header.pri).unwrap_or(2),
        transfer: u8::try_from(header.trc).unwrap_or(2),
        matrix: u8::try_from(header.mtrx).unwrap_or(2),
        full_range: header.color_range != 0,
    };
    let sample_bytes = if depth == 8 { 1 } else { 2 };
    let planes = if chroma == Chroma::Monochrome { 1 } else { 3 };
    for i in 0..planes {
        let (w, h) = out.plane_size(i);
        let Some(data) = picture.data[i] else {
            return Err("AV1 picture without its planes".to_owned());
        };
        // One stride for luma, one shared by both chroma planes.
        let stride = picture.stride[usize::from(i != 0)];
        let stride = usize::try_from(stride).map_err(|_| "negative AV1 stride".to_owned())?;
        if stride < w * sample_bytes {
            return Err("AV1 stride shorter than a row".to_owned());
        }
        let mut plane = Vec::with_capacity(w * h);
        for row in 0..h {
            // SAFETY: dav1d allocates `stride` bytes for each of the plane's `h` rows, and the
            // row's first `w` samples are within them; the picture is alive.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    data.as_ptr().cast::<u8>().add(row * stride),
                    w * sample_bytes,
                )
            };
            if sample_bytes == 1 {
                plane.extend(bytes.iter().map(|&v| u16::from(v)));
            } else {
                plane.extend(
                    bytes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .map(|b| u16::from_ne_bytes(*b)),
                );
            }
        }
        out.planes.push(plane);
    }
    Ok(out)
}
