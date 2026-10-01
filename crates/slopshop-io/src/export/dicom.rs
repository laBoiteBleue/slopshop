//! DICOM writer: a Secondary Capture image (SOP Class 1.2.840.10008.5.1.4.1.1.7), Explicit VR
//! Little Endian, uncompressed. Gray is MONOCHROME2, color RGB (interleaved: Planar
//! Configuration 0), 8 or 16 bits per sample, unsigned. The samples are display values,
//! sRGB-encoded by convention (DICOM has no color tagging here), as our importer declares them.
//! No alpha (flattened over the matte).
//!
//! The attributes the Secondary Capture IOD requires are written with dicom-object: new UIDs for
//! the study, series and instance (UUID-derived, `2.25.<integer>`), modality OT, conversion type
//! WSD (workstation), the patient and study attributes of type 2 empty. 16-bit gray images get
//! the window of their whole range (center 32768, width 65536), so that viewers show them as
//! the document showed them instead of stretching them to their darkest and lightest values;
//! 8-bit ones are shown as they are without one.
//!
//! The pixel data is the last element of the data set: its header is written after the other
//! attributes, then the rows stream into it as they come (top to bottom), so memory stays
//! bounded. Its length is a 32-bit field: at most [`MAX_PIXEL_BYTES`].

use std::fs::File;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use dicom_core::{DataElement, PrimitiveValue, Tag, VR};
use dicom_dictionary_std::{tags, uids};
use dicom_object::InMemDicomObject;
use dicom_object::meta::FileMetaTableBuilder;
use slopshop_core::Size;
use slopshop_core::color::{ChannelLayout, ColorSpace, PixelFormat, SampleType};

use super::bmp::rows_out_of_order;
use super::{ExportError, ExportNotice};

/// Rows and Columns are 16-bit attributes.
pub(super) const MAX_SIDE: u32 = u16::MAX as u32;

/// The pixel data's length is a 32-bit field (0xFFFFFFFF means "undefined"), and even.
const MAX_PIXEL_BYTES: u64 = 0xFFFF_FFFE;

pub(super) struct DicomWriter {
    out: BufWriter<File>,
    row_bytes: usize,
    next_row: u32,
    height: u32,
    /// The pixel data has an odd number of bytes: one padding byte follows it.
    pad: bool,
}

impl DicomWriter {
    pub(super) fn new(file: File, size: Size, target: PixelFormat) -> Result<Self, ExportError> {
        let (samples, photometric) = match target.layout {
            ChannelLayout::Gray => (1u16, "MONOCHROME2"),
            ChannelLayout::Rgb => (3, "RGB"),
            _ => return Err(invalid_target(&target)),
        };
        let bits: u16 = match target.sample {
            SampleType::U8 => 8,
            SampleType::U16 => 16,
            SampleType::F16 | SampleType::F32 => return Err(invalid_target(&target)),
        };
        if target.color_space != ColorSpace::SRGB {
            return Err(ExportError::UnsupportedSpace(target.color_space));
        }
        let too_large = || ExportError::TooLarge {
            width: size.width,
            height: size.height,
        };
        let (Ok(rows), Ok(columns)) = (u16::try_from(size.height), u16::try_from(size.width))
        else {
            return Err(too_large());
        };
        let pixel_bytes = size.pixel_count() * u64::from(samples) * u64::from(bits / 8);
        if pixel_bytes > MAX_PIXEL_BYTES {
            return Err(too_large());
        }
        let pad = pixel_bytes % 2 == 1;

        let mut object = InMemDicomObject::new_empty();
        let mut put = |tag: Tag, vr: VR, value: PrimitiveValue| {
            object.put(DataElement::new(tag, vr, value));
        };
        let study = new_uid();
        put(
            tags::SOP_CLASS_UID,
            VR::UI,
            uids::SECONDARY_CAPTURE_IMAGE_STORAGE.into(),
        );
        put(tags::SOP_INSTANCE_UID, VR::UI, new_uid().into());
        put(tags::STUDY_INSTANCE_UID, VR::UI, study.into());
        put(tags::SERIES_INSTANCE_UID, VR::UI, new_uid().into());
        put(tags::MODALITY, VR::CS, "OT".into());
        put(tags::CONVERSION_TYPE, VR::CS, "WSD".into());
        put(
            tags::SECONDARY_CAPTURE_DEVICE_MANUFACTURER,
            VR::LO,
            "SlopShop".into(),
        );
        // Type 2 attributes: present, empty.
        for (tag, vr) in [
            (tags::STUDY_DATE, VR::DA),
            (tags::STUDY_TIME, VR::TM),
            (tags::ACCESSION_NUMBER, VR::SH),
            (tags::REFERRING_PHYSICIAN_NAME, VR::PN),
            (tags::PATIENT_NAME, VR::PN),
            (tags::PATIENT_ID, VR::LO),
            (tags::PATIENT_BIRTH_DATE, VR::DA),
            (tags::PATIENT_SEX, VR::CS),
            (tags::STUDY_ID, VR::SH),
            (tags::SERIES_NUMBER, VR::IS),
            (tags::INSTANCE_NUMBER, VR::IS),
            (tags::PATIENT_ORIENTATION, VR::CS),
        ] {
            put(tag, vr, PrimitiveValue::Empty);
        }
        put(tags::SAMPLES_PER_PIXEL, VR::US, samples.into());
        put(tags::PHOTOMETRIC_INTERPRETATION, VR::CS, photometric.into());
        if samples == 3 {
            put(tags::PLANAR_CONFIGURATION, VR::US, 0u16.into());
        }
        put(tags::ROWS, VR::US, rows.into());
        put(tags::COLUMNS, VR::US, columns.into());
        put(tags::BITS_ALLOCATED, VR::US, bits.into());
        put(tags::BITS_STORED, VR::US, bits.into());
        put(tags::HIGH_BIT, VR::US, (bits - 1).into());
        put(tags::PIXEL_REPRESENTATION, VR::US, 0u16.into());
        if samples == 1 && bits == 16 {
            // The whole range, linearly (DICOM's LINEAR window: from c − 0.5 − (w − 1) / 2).
            put(tags::WINDOW_CENTER, VR::DS, "32768".into());
            put(tags::WINDOW_WIDTH, VR::DS, "65536".into());
        }
        let file_object = object
            .with_meta(FileMetaTableBuilder::new().transfer_syntax(uids::EXPLICIT_VR_LITTLE_ENDIAN))
            .map_err(|e| ExportError::Encode(format!("DICOM: {e}")))?;
        let mut header = Vec::new();
        file_object
            .write_all(&mut header)
            .map_err(|e| ExportError::Encode(format!("DICOM: {e}")))?;
        // The pixel data element: tag, VR, two reserved bytes, 32-bit length.
        let pixel_tag = tags::PIXEL_DATA;
        header.extend(pixel_tag.group().to_le_bytes());
        header.extend(pixel_tag.element().to_le_bytes());
        header.extend(if bits == 8 { b"OB" } else { b"OW" });
        header.extend([0, 0]);
        // Checked above: at most MAX_PIXEL_BYTES once padded.
        header.extend(((pixel_bytes + u64::from(pad)) as u32).to_le_bytes());

        let mut out = BufWriter::new(file);
        out.write_all(&header)?;
        Ok(Self {
            out,
            row_bytes: size.width as usize * target.bytes_per_pixel() as usize,
            next_row: 0,
            height: size.height,
            pad,
        })
    }

    pub(super) fn write_rows(&mut self, first_row: u32, rows: &[u8]) -> Result<(), ExportError> {
        if first_row != self.next_row || !rows.len().is_multiple_of(self.row_bytes) {
            return Err(rows_out_of_order(first_row, self.next_row, rows.len()));
        }
        // Interleaved, little-endian: as the rows come.
        self.out.write_all(rows)?;
        self.next_row += (rows.len() / self.row_bytes) as u32;
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<Vec<ExportNotice>, ExportError> {
        if self.next_row != self.height {
            return Err(ExportError::Encode(format!(
                "{} rows written out of {}",
                self.next_row, self.height
            )));
        }
        if self.pad {
            self.out.write_all(&[0])?;
        }
        self.out.flush()?;
        Ok(Vec::new())
    }
}

/// A new unique identifier: a random (version 4) UUID as an integer under the `2.25` root
/// (ITU-T X.667), from std's randomly keyed hasher fed with the time, the process and a
/// counter.
fn new_uid() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let half = |salt: u64| {
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u128(time);
        hasher.write_u32(std::process::id());
        hasher.write_u64(count);
        hasher.write_u64(salt);
        hasher.finish()
    };
    let mut uuid = (u128::from(half(0)) << 64) | u128::from(half(1));
    // Version 4 (random), variant 1 (RFC 9562).
    uuid = (uuid & !(0xf << 76)) | (0x4 << 76);
    uuid = (uuid & !(0x3 << 62)) | (0x2 << 62);
    format!("2.25.{uuid}")
}

fn invalid_target(target: &PixelFormat) -> ExportError {
    ExportError::InvalidSpec(format!("DICOM cannot store {target:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uids_are_unique_and_valid() {
        let a = new_uid();
        let b = new_uid();
        assert_ne!(a, b);
        for uid in [a, b] {
            // At most 64 characters, digits and dots, no leading zero in a component.
            assert!(uid.len() <= 64, "{uid}");
            assert!(uid.starts_with("2.25."));
            let last = &uid[5..];
            assert!(last.bytes().all(|c| c.is_ascii_digit()) && !last.starts_with('0'));
        }
    }
}
