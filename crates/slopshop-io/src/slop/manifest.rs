//! The manifest: a JSON description of the document (nodes, stack, images), written through
//! explicit DTOs so that core types never carry serialization concerns (ADR 0009).
//!
//! Unknown fields are kept (`extra`, flattened) and unknown top-level `sections` too, so that a
//! newer minor version's data survives a save by this version.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use slopshop_core::color::{
    AlphaMode, ChannelLayout, ColorSpace, PixelFormat, RgbPrimaries, SampleType, TransferFunction,
};

use super::FileError;

/// Schema version written by this code; readers accept any 0.x.
pub(crate) const SCHEMA_MAJOR: u32 = 0;
pub(crate) const SCHEMA_MINOR: u32 = 1;

/// The pyramid algorithm of `RasterImage` (box filter, linear light, premultiplied alpha).
/// Stored levels tagged with another algorithm are rebuilt on load.
pub(crate) const PYRAMID_ALGORITHM: &str = "slopshop.pyramid.box-linear-premul@1";

pub(crate) const NODE_RASTER: &str = "slopshop.raster";
pub(crate) const NODE_FILL: &str = "slopshop.fill";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub schema: Schema,
    pub writer: Writer,
    pub document: DocumentDto,
    /// Keyed by node id (decimal).
    pub nodes: BTreeMap<String, NodeDto>,
    /// Keyed by image key (`b3:…`).
    pub images: BTreeMap<String, ImageDto>,
    /// Optional data of other versions, kept verbatim.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub sections: Map<String, Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Schema {
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Writer {
    pub app: String,
    pub version: String,
}

impl Writer {
    pub(crate) fn current() -> Self {
        Self {
            app: "slopshop".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DocumentDto {
    pub size: [u32; 2],
    pub working_space: ColorSpaceDto,
    pub next_node_id: u64,
    /// Node ids, bottom to top.
    pub stack: Vec<u64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct NodeDto {
    #[serde(rename = "type")]
    pub kind: String,
    /// Version of this node type's parameters.
    pub version: u32,
    pub name: String,
    pub visible: bool,
    pub opacity: f32,
    pub params: Map<String, Value>,
    #[serde(default)]
    pub inputs: Vec<u64>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ImageDto {
    pub size: [u32; 2],
    pub tile_size: u32,
    pub source_format: FormatDto,
    /// Tile tables of each pyramid level, finest first.
    pub levels: Vec<LevelDto>,
    pub pyramid_algorithm: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LevelDto {
    /// Key of the level's tile table.
    pub table: String,
    /// Computed from level 0 (can be rebuilt).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub derived: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FormatDto {
    pub layout: String,
    pub sample: String,
    pub alpha: String,
    pub color_space: ColorSpaceDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ColorSpaceDto {
    pub primaries: PrimariesDto,
    pub transfer: TransferDto,
    /// A hint for readers; the numbers are what counts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct PrimariesDto {
    pub r: [f64; 2],
    pub g: [f64; 2],
    pub b: [f64; 2],
    pub w: [f64; 2],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum TransferDto {
    Linear,
    Srgb,
    Gamma {
        gamma: f32,
    },
    Rec709,
    Parametric {
        g: f32,
        a: f32,
        b: f32,
        c: f32,
        d: f32,
        e: f32,
        f: f32,
    },
    Pq,
    Hlg,
}

impl ColorSpaceDto {
    pub(crate) fn new(space: &ColorSpace) -> Self {
        let p = &space.primaries;
        Self {
            primaries: PrimariesDto {
                r: p.red,
                g: p.green,
                b: p.blue,
                w: p.white,
            },
            transfer: match space.transfer {
                TransferFunction::Linear => TransferDto::Linear,
                TransferFunction::Srgb => TransferDto::Srgb,
                TransferFunction::Gamma(gamma) => TransferDto::Gamma { gamma },
                TransferFunction::Rec709 => TransferDto::Rec709,
                TransferFunction::Parametric {
                    g,
                    a,
                    b,
                    c,
                    d,
                    e,
                    f,
                } => TransferDto::Parametric {
                    g,
                    a,
                    b,
                    c,
                    d,
                    e,
                    f,
                },
                TransferFunction::Pq => TransferDto::Pq,
                TransferFunction::Hlg => TransferDto::Hlg,
            },
            id_hint: space.id().map(str::to_owned),
        }
    }

    pub(crate) fn to_space(&self) -> ColorSpace {
        let p = &self.primaries;
        ColorSpace {
            primaries: RgbPrimaries {
                red: p.r,
                green: p.g,
                blue: p.b,
                white: p.w,
            },
            transfer: match self.transfer {
                TransferDto::Linear => TransferFunction::Linear,
                TransferDto::Srgb => TransferFunction::Srgb,
                TransferDto::Gamma { gamma } => TransferFunction::Gamma(gamma),
                TransferDto::Rec709 => TransferFunction::Rec709,
                TransferDto::Parametric {
                    g,
                    a,
                    b,
                    c,
                    d,
                    e,
                    f,
                } => TransferFunction::Parametric {
                    g,
                    a,
                    b,
                    c,
                    d,
                    e,
                    f,
                },
                TransferDto::Pq => TransferFunction::Pq,
                TransferDto::Hlg => TransferFunction::Hlg,
            },
        }
    }
}

impl FormatDto {
    pub(crate) fn new(format: &PixelFormat) -> Self {
        Self {
            layout: match format.layout {
                ChannelLayout::Gray => "gray",
                ChannelLayout::GrayAlpha => "gray-alpha",
                ChannelLayout::Rgb => "rgb",
                ChannelLayout::Rgba => "rgba",
            }
            .to_owned(),
            sample: match format.sample {
                SampleType::U8 => "u8",
                SampleType::U16 => "u16",
                SampleType::F16 => "f16",
                SampleType::F32 => "f32",
            }
            .to_owned(),
            alpha: match format.alpha {
                AlphaMode::Straight => "straight",
                AlphaMode::Premultiplied => "premultiplied",
            }
            .to_owned(),
            color_space: ColorSpaceDto::new(&format.color_space),
        }
    }

    pub(crate) fn to_format(&self) -> Result<PixelFormat, FileError> {
        let invalid = |what: &str, value: &str| {
            FileError::Corrupt(format!("unknown {what} `{value}` in an image format"))
        };
        Ok(PixelFormat {
            layout: match self.layout.as_str() {
                "gray" => ChannelLayout::Gray,
                "gray-alpha" => ChannelLayout::GrayAlpha,
                "rgb" => ChannelLayout::Rgb,
                "rgba" => ChannelLayout::Rgba,
                other => return Err(invalid("layout", other)),
            },
            sample: match self.sample.as_str() {
                "u8" => SampleType::U8,
                "u16" => SampleType::U16,
                "f16" => SampleType::F16,
                "f32" => SampleType::F32,
                other => return Err(invalid("sample type", other)),
            },
            alpha: match self.alpha.as_str() {
                "straight" => AlphaMode::Straight,
                "premultiplied" => AlphaMode::Premultiplied,
                other => return Err(invalid("alpha mode", other)),
            },
            color_space: self.color_space.to_space(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_spaces_round_trip_exactly() {
        for space in [
            ColorSpace::SRGB,
            ColorSpace::LINEAR_SRGB,
            ColorSpace::DISPLAY_P3,
            ColorSpace::ADOBE_RGB,
            ColorSpace::PROPHOTO,
            ColorSpace::REC2020,
            ColorSpace::LINEAR_REC2020,
            ColorSpace::REC2100_PQ,
            ColorSpace::REC2100_HLG,
            ColorSpace {
                primaries: RgbPrimaries::ADOBE_RGB,
                transfer: TransferFunction::Parametric {
                    g: 2.4,
                    a: 0.947_867_3,
                    b: 0.052_132_7,
                    c: 0.077_399_4,
                    d: 0.040_45,
                    e: 0.0,
                    f: 0.0,
                },
            },
        ] {
            let json = serde_json::to_string(&ColorSpaceDto::new(&space)).unwrap();
            let back: ColorSpaceDto = serde_json::from_str(&json).unwrap();
            assert_eq!(back.to_space(), space, "{json}");
        }
    }

    #[test]
    fn unknown_fields_are_kept() {
        let json = r#"{"type":"slopshop.fill","version":1,"name":"a","visible":true,
            "opacity":0.5,"params":{"color":[1,0,0,1]},"inputs":[],"future":{"x":1}}"#;
        let node: NodeDto = serde_json::from_str(json).unwrap();
        assert_eq!(node.extra.get("future"), Some(&serde_json::json!({"x": 1})));
        let again = serde_json::to_value(&node).unwrap();
        assert_eq!(again["future"], serde_json::json!({"x": 1}));
    }
}
