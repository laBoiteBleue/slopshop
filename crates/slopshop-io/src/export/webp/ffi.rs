//! Lossy WebP encoding through libwebp (C, `libwebp-sys`): the only `unsafe` code of this crate
//! (ADR 0010).
//!
//! [`encode_lossy`] hands libwebp YUV 4:2:0 planes (plus alpha) that we own, and returns the
//! bytes libwebp produced (a complete RIFF file, which the caller remuxes to add the ICC
//! profile). Everything libwebp is given lives on the Rust side for the whole call: the planes
//! (borrowed mutably, since libwebp takes `*mut` pointers), the output writer and the cancel
//! token its progress hook reads.
//!
//! The configuration and picture are initialized with `WEBP_ENCODER_ABI_VERSION`: the helper
//! functions of `libwebp-sys` 0.14.4 (`WebPConfig::new`, `WebPPicture::new`) pass the *decoder*
//! ABI version, which libwebp's encoder rejects or misreads.
#![allow(unsafe_code)]

use std::ffi::{c_int, c_void};
use std::mem::MaybeUninit;
use std::panic::{AssertUnwindSafe, catch_unwind};

use libwebp_sys::{
    WEBP_ENCODER_ABI_VERSION, WebPConfig, WebPConfigInitInternal, WebPEncCSP, WebPEncode,
    WebPEncodingError, WebPMemoryWrite, WebPMemoryWriter, WebPMemoryWriterClear,
    WebPMemoryWriterInit, WebPPicture, WebPPictureFree, WebPPictureInitInternal, WebPPreset,
    WebPValidateConfig,
};
use slopshop_core::CancelToken;

/// YUV 4:2:0 planes of an image, with an optional alpha plane: Y and A are `width × height`,
/// U and V `ceil(width / 2) × ceil(height / 2)`, rows without padding.
pub(super) struct Planes<'a> {
    pub width: u32,
    pub height: u32,
    pub y: &'a mut [u8],
    pub u: &'a mut [u8],
    pub v: &'a mut [u8],
    pub a: Option<&'a mut [u8]>,
}

/// Encoder settings we choose; everything else keeps libwebp's defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LossySettings {
    /// 0 to 100.
    pub quality: f32,
    /// Speed/size trade-off, 0 (fast) to 6 (small).
    pub method: c_int,
    /// Segments (1 to 4), `None` for libwebp's default (4).
    pub segments: Option<c_int>,
    /// 0 to 100: how much the first partition may be degraded to fit its 512 KiB limit.
    pub partition_limit: c_int,
    /// Trade speed for memory.
    pub low_memory: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum EncodeError {
    /// The cancel token was set during encoding.
    Cancelled,
    /// The first partition (modes and headers) went over VP8's 512 KiB limit.
    Partition0Overflow,
    OutOfMemory,
    /// Anything else, with libwebp's error.
    Other(String),
}

/// Encode `planes` as lossy WebP (VP8, plus ALPH when there is alpha). Blocking; returns early
/// with [`EncodeError::Cancelled`] once `cancel` is set (libwebp checks at each progress step).
pub(super) fn encode_lossy(
    planes: Planes<'_>,
    settings: LossySettings,
    cancel: &CancelToken,
) -> Result<Vec<u8>, EncodeError> {
    let width = c_int::try_from(planes.width).map_err(|_| other("width"))?;
    let height = c_int::try_from(planes.height).map_err(|_| other("height"))?;
    let chroma_width = planes.width.div_ceil(2) as usize;
    let chroma_len = chroma_width * planes.height.div_ceil(2) as usize;
    let luma_len = planes.width as usize * planes.height as usize;
    // libwebp reads these lengths through raw pointers: check them here.
    if planes.y.len() != luma_len
        || planes.u.len() != chroma_len
        || planes.v.len() != chroma_len
        || planes.a.as_ref().is_some_and(|a| a.len() != luma_len)
    {
        return Err(other("plane sizes do not match the image size"));
    }

    let mut config = MaybeUninit::<WebPConfig>::uninit();
    // SAFETY: `config` is valid for writes; WebPConfigInitInternal initializes every field
    // and returns 0 on failure (ABI version mismatch), in which case it is never read.
    let initialized = unsafe {
        WebPConfigInitInternal(
            config.as_mut_ptr(),
            WebPPreset::WEBP_PRESET_DEFAULT,
            settings.quality,
            WEBP_ENCODER_ABI_VERSION as c_int,
        )
    };
    if initialized == 0 {
        return Err(other("libwebp rejected the configuration version"));
    }
    // SAFETY: initialized just above.
    let mut config = unsafe { config.assume_init() };
    config.lossless = 0;
    config.quality = settings.quality;
    config.method = settings.method;
    if let Some(segments) = settings.segments {
        config.segments = segments;
    }
    config.partition_limit = settings.partition_limit;
    config.low_memory = c_int::from(settings.low_memory);
    config.thread_level = 1;
    config.use_sharp_yuv = 0;
    config.alpha_compression = 1;
    config.alpha_quality = 100;
    // Keep the color of transparent pixels as given (no smoothing of the alpha-0 areas).
    config.exact = 1;
    // SAFETY: `config` is a fully initialized value that outlives the call.
    if unsafe { WebPValidateConfig(&config) } == 0 {
        return Err(other("invalid configuration"));
    }

    let mut picture = MaybeUninit::<WebPPicture>::uninit();
    // SAFETY: as for the configuration: every field is initialized when it returns non-zero.
    if unsafe { WebPPictureInitInternal(picture.as_mut_ptr(), WEBP_ENCODER_ABI_VERSION as c_int) }
        == 0
    {
        return Err(other("libwebp rejected the picture version"));
    }
    // SAFETY: initialized just above.
    let mut picture = unsafe { picture.assume_init() };
    let mut output = MaybeUninit::<WebPMemoryWriter>::uninit();
    // SAFETY: WebPMemoryWriterInit initializes the writer (empty buffer).
    let mut output = unsafe {
        WebPMemoryWriterInit(output.as_mut_ptr());
        output.assume_init()
    };

    picture.use_argb = 0;
    picture.width = width;
    picture.height = height;
    picture.y = planes.y.as_mut_ptr();
    picture.u = planes.u.as_mut_ptr();
    picture.v = planes.v.as_mut_ptr();
    picture.y_stride = width;
    picture.uv_stride = chroma_width as c_int;
    match planes.a {
        Some(a) => {
            picture.colorspace = WebPEncCSP::WEBP_YUV420A;
            picture.a = a.as_mut_ptr();
            picture.a_stride = width;
        }
        None => picture.colorspace = WebPEncCSP::WEBP_YUV420,
    }
    picture.writer = Some(WebPMemoryWrite);
    picture.custom_ptr = (&raw mut output).cast::<c_void>();
    picture.progress_hook = Some(progress);
    picture.user_data = std::ptr::from_ref(cancel).cast_mut().cast::<c_void>();

    // SAFETY: `config` is valid; `picture` points to planes of the sizes checked above (with
    // the strides set to match), to `output` and to `cancel`, all of which outlive the call and
    // are not otherwise accessed during it. libwebp calls `progress` and `WebPMemoryWrite` on
    // this thread or on its own worker thread, and joins the latter before returning.
    let encoded = unsafe { WebPEncode(&config, &mut picture) } != 0;
    let result = if encoded {
        Ok(if output.size == 0 || output.mem.is_null() {
            Vec::new()
        } else {
            // SAFETY: after a successful encode, `output.mem` holds `output.size` initialized
            // bytes allocated by libwebp.
            unsafe { std::slice::from_raw_parts(output.mem, output.size) }.to_vec()
        })
    } else if cancel.is_cancelled() {
        Err(EncodeError::Cancelled)
    } else {
        Err(match picture.error_code {
            WebPEncodingError::VP8_ENC_ERROR_PARTITION0_OVERFLOW => EncodeError::Partition0Overflow,
            WebPEncodingError::VP8_ENC_ERROR_OUT_OF_MEMORY
            | WebPEncodingError::VP8_ENC_ERROR_BITSTREAM_OUT_OF_MEMORY => EncodeError::OutOfMemory,
            WebPEncodingError::VP8_ENC_ERROR_USER_ABORT => EncodeError::Cancelled,
            code => EncodeError::Other(format!("{code:?}")),
        })
    };
    // SAFETY: frees the buffer libwebp allocated in `output` (the bytes were copied), and
    // anything WebPEncode allocated in `picture`. The planes are ours: `memory_` is null since
    // the picture was never allocated by libwebp, so WebPPictureFree does not touch them.
    unsafe {
        WebPMemoryWriterClear(&mut output);
        WebPPictureFree(&mut picture);
    }
    result
}

/// libwebp's progress hook: continue (1) unless the export was cancelled.
unsafe extern "C" fn progress(_percent: c_int, picture: *const WebPPicture) -> c_int {
    // A panic must not unwind into C.
    catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: libwebp passes the picture given to WebPEncode, whose `user_data` is the
        // `CancelToken` borrowed by `encode_lossy` for the whole call.
        let cancel = unsafe { &*((*picture).user_data as *const CancelToken) };
        c_int::from(!cancel.is_cancelled())
    }))
    .unwrap_or(0)
}

fn other(what: &str) -> EncodeError {
    EncodeError::Other(what.to_owned())
}
