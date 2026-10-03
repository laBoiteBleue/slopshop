// Typed access to the engine over Tauri IPC.
// Mirrors app/src-tauri/src/ipc.rs — keep both in sync.
//
// The UI never computes image content: it sends intents (edits) and displays what the engine
// returns. Frames arrive as raw binary (ArrayBuffer), never as JSON.

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type LayerView = {
  id: number;
  name: string;
  visible: boolean;
  opacity: number;
  kind: "fill" | "raster" | "group" | "adjustment";
  /** sRGB-encoded RGBA in [0, 1], for display swatches only. */
  swatch: [number, number, number, number];
  /** Translated with the `blendMode.<id>` keys. */
  blendMode: BlendModeId;
  /** Changes when the layer's pixels change (0 for fills): time for a new thumbnail. */
  contentKey: number;
  /** A raster with transparency: a mask can be made from it. */
  hasAlpha: boolean;
  /** The layer's mask (ADR 0014); `contentKey` changes with its pixels. */
  mask: { enabled: boolean; contentKey: number } | null;
  /** A group's layers, bottom to top (ADR 0015); empty for other layers. */
  children: LayerView[];
  /** A group whose layers blend through it. */
  passThrough: boolean;
  /** Clipped to the layer below it (ADR 0016). */
  clipped: boolean;
  /** From the layer's content to its parent (ADR 0017): `[a, b, c, d, e, f]`. */
  transform: [number, number, number, number, number, number];
  /** Its pixels or its mask carry paint (ADR 0027): Layer > Delete Paint removes it. */
  painted: boolean;
  /** An adjustment layer's adjustment (ADR 0020): its identifier and five parameters. */
  /** `values`: all `ADJUSTMENT_PARAMS` parameters (`Adjustment::params` order). Curves:
   * `curves`, the points `[input, output]` (0–255) of the composite, red, green and blue
   * curves, and `curveSamples`, each curve's output (0–1) at evenly spaced inputs. */
  adjustment: {
    id: AdjustmentId;
    values: number[];
    curves: number[][][] | null;
    curveSamples: number[][] | null;
  } | null;
};

/** Adjustments of adjustment layers (Adjustment in crates/slopshop-core/src/adjust.rs). */
export type AdjustmentId =
  | "brightnessContrast"
  | "levels"
  | "curves"
  | "exposure"
  | "vibrance"
  | "hueSaturation"
  | "colorBalance"
  | "blackWhite"
  | "photoFilter"
  | "channelMixer"
  | "invert"
  | "posterize"
  | "threshold";

/** Adjustments in the order of Photoshop's New Adjustment Layer menu. */
export const ADJUSTMENTS: AdjustmentId[] = [
  "brightnessContrast",
  "levels",
  "curves",
  "exposure",
  "vibrance",
  "hueSaturation",
  "colorBalance",
  "blackWhite",
  "photoFilter",
  "channelMixer",
  "invert",
  "posterize",
  "threshold",
];

/** Number of parameters of an adjustment (`PARAM_COUNT` in crates/slopshop-core/src/adjust.rs). */
export const ADJUSTMENT_PARAMS = 16;

/** Turns and flips of Image > Image Rotation. */
export type ImageTurn =
  "clockwise" | "counterClockwise" | "halfTurn" | "flipHorizontal" | "flipVertical";

/** A 2D affine map `[a, b, c, d, e, f]`: (x, y) ↦ (a·x + c·y + e, b·x + d·y + f). */
export type Matrix = [number, number, number, number, number, number];

/** A rectangle in document pixels (right and bottom exclusive). */
export type Bounds = { left: number; top: number; right: number; bottom: number };

/** What moving layers can snap to (SnapTargets in lib.rs). */
export type SnapTargets = { moving: Bounds | null; others: Bounds[] };

/** Blend modes (BlendMode in crates/slopshop-core/src/blend.rs, ADR 0012). */
export type BlendModeId =
  | "normal"
  | "dissolve"
  | "darken"
  | "multiply"
  | "colorBurn"
  | "linearBurn"
  | "darkerColor"
  | "lighten"
  | "screen"
  | "colorDodge"
  | "linearDodge"
  | "lighterColor"
  | "overlay"
  | "softLight"
  | "hardLight"
  | "vividLight"
  | "linearLight"
  | "pinLight"
  | "hardMix"
  | "difference"
  | "exclusion"
  | "subtract"
  | "divide"
  | "hue"
  | "saturation"
  | "color"
  | "luminosity";

/** The modes in Photoshop's menu order, by group (the menu separates the groups). */
export const BLEND_MODE_GROUPS: BlendModeId[][] = [
  ["normal", "dissolve"],
  ["darken", "multiply", "colorBurn", "linearBurn", "darkerColor"],
  ["lighten", "screen", "colorDodge", "linearDodge", "lighterColor"],
  ["overlay", "softLight", "hardLight", "vividLight", "linearLight", "pinLight", "hardMix"],
  ["difference", "exclusion", "subtract", "divide"],
  ["hue", "saturation", "color", "luminosity"],
];

/** Where layers blend: perceptual (Photoshop's look, the default) or linear (physical, HDR). */
export type BlendSpaceId = "perceptual" | "linear";

export type DocumentView = {
  /** One document per tab; ids are never reused. */
  id: number;
  /** File name, or null for an untitled document. */
  name: string | null;
  width: number;
  height: number;
  /** Identifier, translated with the `colorSpace.<id>` i18n keys. */
  workingSpace: ColorSpaceId;
  blendSpace: BlendSpaceId;
  /** Pixels per inch (ADR 0028). */
  resolution: number;
  revision: number;
  canUndo: boolean;
  canRedo: boolean;
  /** Bottom to top. */
  layers: LayerView[];
  /** How the source file was interpreted; translated with the `open.warning.<id>` keys. */
  warnings: ImportWarning[];
  /** The `.slop` file the document was opened from or saved to, or null. */
  path: string | null;
  /** Changed since it was opened, created or last saved. */
  dirty: boolean;
  /** Identity of the selection (ADR 0024): a new value, a new outline; null: nothing selected. */
  selectionKey: number | null;
  /** Select > Reselect has a selection to bring back. */
  canReselect: boolean;
  /** The view shows Quick Mask (view state, not in the document or its history). */
  quickMask: boolean;
};

/** A shape to select, in document pixels. */
export type SelectionShape =
  | { kind: "rectangle" | "ellipse"; left: number; top: number; right: number; bottom: number }
  | { kind: "polygon"; points: [number, number][] };

/** A new layer mask: everything shown or hidden, or the selection shown or hidden. */
export type LayerMaskKind = "revealAll" | "hideAll" | "revealSelection" | "hideSelection";

/** Select > Color Range's samples (document pixels) and settings. */
export type ColorRangeRequest = {
  included: [number, number][];
  excluded: [number, number][];
  fuzziness: number;
  invert: boolean;
  /** Sample only this layer; null: the image as displayed. */
  layerId: number | null;
};

/** Select > Modify's changes. */
export type SelectionModify = "border" | "smooth" | "expand" | "contract" | "feather";

/** How a new shape combines with the selection. */
export type SelectionMode = "replace" | "add" | "subtract" | "intersect";

/** The outline of the selection, polylines in document pixels: `[x0, y0, x1, y1, …]` each. */
export type SelectionOutline = Uint32Array[];

export type EditRequest =
  | { kind: "addFillLayer"; name: string; color: [number, number, number, number] }
  /** A canvas-sized, transparent 8-bit sRGB layer to paint on (ADR 0027). */
  | { kind: "addEmptyLayer"; name: string; parent: number | null; index: number }
  /** Layer > Delete Paint: the layers' (and their masks') originals show again. */
  | { kind: "deletePaint"; ids: number[] }
  | { kind: "removeLayer"; id: number }
  | { kind: "setLayerVisible"; id: number; visible: boolean }
  | { kind: "setLayerOpacity"; id: number; opacity: number }
  | { kind: "renameLayer"; id: number; name: string }
  /** `index` is the final position among the layers of `parent` (null: the top level), 0 = bottom. */
  | { kind: "moveLayer"; id: number; parent?: number | null; index: number }
  /** `index` counts the layers of `parent` that do not move (0 = below them all). */
  | { kind: "moveLayers"; ids: number[]; parent: number | null; index: number }
  | { kind: "addGroup"; name: string; parent: number | null; index: number }
  /** A new adjustment layer at its neutral parameters (`index` among `parent`'s layers). */
  | {
      kind: "addAdjustmentLayer";
      name: string;
      adjustment: AdjustmentId;
      parent: number | null;
      index: number;
    }
  /** An adjustment layer's parameters (`LayerView.adjustment.values` order, at most
   * `ADJUSTMENT_PARAMS`). */
  | {
      kind: "setAdjustment";
      id: number;
      adjustment: AdjustmentId;
      values: number[];
      /** Curves only: the points of the composite, red, green and blue curves. */
      curves?: number[][][];
    }
  /** Into a new group in the place of the topmost of them (Layer > Group Layers). */
  | { kind: "groupLayers"; ids: number[]; name: string }
  | { kind: "ungroup"; id: number }
  /** Copies right above their originals, named by `nameFormat` (`{name}`: the original's). */
  | { kind: "duplicateLayers"; ids: number[]; nameFormat: string }
  | { kind: "setGroupPassThrough"; id: number; passThrough: boolean }
  | { kind: "setLayerClipped"; id: number; clipped: boolean }
  /** Move layers by whole document pixels (a group moves whole). */
  | { kind: "translateLayers"; ids: number[]; dx: number; dy: number }
  /** Apply `matrix` ([a, b, c, d, e, f], in document pixels) to layers (Free Transform). */
  | { kind: "transformLayers"; ids: number[]; matrix: Matrix }
  /** Image > Image Size: the whole image resampled to this size. */
  | { kind: "resizeImage"; width: number; height: number; resolution?: number }
  /** Image > Canvas Size: `anchor` [x, y] in [0, 1] keeps the image there (0.5: centered). */
  | { kind: "canvasSize"; width: number; height: number; anchor: [number, number] }
  /** The Crop tool: keep this area of the canvas (it may extend past it). */
  | { kind: "crop"; x: number; y: number; width: number; height: number }
  /** Image > Image Rotation: exact turns and flips of the whole image. */
  | { kind: "rotateImage"; turn: ImageTurn }
  | { kind: "setLayerBlendMode"; id: number; mode: BlendModeId }
  | { kind: "setBlendSpace"; space: BlendSpaceId }
  | { kind: "setLayerMaskEnabled"; id: number; enabled: boolean }
  | { kind: "removeLayerMask"; id: number }
  /** Several edits as one undo entry, applied in order: all or none. */
  | { kind: "batch"; edits: EditRequest[] };

/** View changes; positions and deltas are in viewport device pixels. */
export type ViewRequest =
  | { kind: "fit" }
  | { kind: "setZoom"; zoom: number }
  | { kind: "zoomBy"; factor: number; x: number; y: number }
  | { kind: "step"; zoomIn: boolean; x: number | null; y: number | null }
  | { kind: "pan"; dx: number; dy: number };

/** Zoom range of the engine (MIN_ZOOM, MAX_ZOOM in crates/slopshop-core/src/view.rs). */
export const MIN_ZOOM = 0.001;
export const MAX_ZOOM = 64;

/** document = origin + output / zoom (output in device pixels). */
export type ViewInfo = {
  /** 1 = 100%. */
  zoom: number;
  origin: [number, number];
  fit: boolean;
};

/**
 * How the viewport reaches the screen (ADR 0002): `frames` are drawn by the UI in a canvas;
 * with `window`, the engine presents directly to the window, under the (transparent) page.
 */
export type PresenterMode = "frames" | "window";

/** A canvas area of the window, in physical pixels of its client area. */
export type DeviceRect = { x: number; y: number; width: number; height: number };

/** Outcome of a native present (see PresentInfo in app/src-tauri/src/ipc.rs). */
export type PresentInfo = {
  /** False when nothing was shown (window occluded, swapchain busy): present again later. */
  presented: boolean;
  /** False when part of the view was shown coarser while it is composited: present again. */
  complete: boolean;
  revision: number;
  zoom: number;
  /** The view presented: its document point at the top left of the canvas. */
  origin: [number, number];
  fit: boolean;
  renderMs: number;
};

/** What a paste found on the clipboard (Pasted in app/src-tauri/src/lib.rs). */
/** What a paste brought (Pasted in clipboard.rs). */
export type Pasted =
  /** Files copied in the file manager: the app opens them. */
  | { kind: "files"; paths: string[] }
  /** New top-level layers `ids` (the group of a Paste Into) in `document`, a new tab or not. */
  | { kind: "layers"; document: DocumentView; newTab: boolean; ids: number[] }
  | { kind: "nothing" }
  /** Paste Into without a selection. */
  | { kind: "noSelection" };

/** What Edit > Copy takes (CopyRequest in clipboard.rs). */
export type CopyRequest =
  | { kind: "layers"; ids: number[] }
  | { kind: "pixels"; layerId: number; target: "layer" | "mask" }
  | { kind: "merged"; name: string };

/** Edit > Paste, Paste in Place or Paste Into. */
export type PasteKind = "paste" | "inPlace" | "into";

/** What an open found in folders and zip archives (OpenSummary in lib.rs). */
export type OpenSummary = {
  /** Files SlopShop does not open. */
  skipped: number;
  /** Archives or folders that could not be read: name and technical detail. */
  failedArchives: [string, string][];
};

/** Where an opened image goes. */
export type OpenTarget = { kind: "newTab" } | { kind: "layer"; documentId: number };

/** An open in progress. */
export type Opening = { id: number; name: string; target: OpenTarget };

export type OpenFinished = { id: number; target: OpenTarget; document: DocumentView };
export type OpenFailed = {
  id: number;
  name: string;
  /** Translated with the `open.error.<code>` keys (`documentClosed` is not shown). */
  code: OpenErrorCode | "documentClosed";
  /** Technical detail inserted in the translated message (format name, decoder message). */
  detail: string;
};

export type ColorSpaceId =
  | "srgb"
  | "linear-srgb"
  | "display-p3"
  | "adobe-rgb"
  | "prophoto"
  | "rec2020"
  | "linear-rec2020"
  | "rec2100-pq"
  | "rec2100-hlg"
  | "custom";

export type ImportWarning =
  | "iccProfileUnsupported"
  | "iccCurveApproximated"
  | "firstFrameOnly"
  | "firstPageOnly"
  | "precisionReduced"
  | "nonFiniteSamples"
  | "colorInfoUnsupported"
  | "layersFlattened"
  | "adjustmentLayersSkipped"
  | "adjustmentsApproximated"
  | "layerStylesIgnored"
  | "layersRasterized"
  | "pixelsOutsideCanvas"
  | "masksSimplified"
  | "pdfContentSkipped"
  | "dicomWindowApproximated"
  | "fitsValuesScaled"
  | "blendSpaceDiffers";

/** A PDF page's or an SVG's size in points (1/72 inch), its rotation applied. */
export type PageSize = { width: number; height: number };

/** A PDF or an SVG, as the import dialog shows it. */
export type VectorInfo = {
  kind: "pdf" | "svg";
  /** 300 for PDF, 96 for SVG (its own size). */
  defaultDpi: number;
  pages: PageSize[];
};

export type OpenErrorCode =
  | "io"
  | "decode"
  | "notYetSupported"
  | "heic"
  | "psdWithoutComposite"
  | "unsupportedPixels"
  | "tooLarge"
  | "unrecognized"
  | "notASlopFile"
  | "newerVersion"
  | "unsupportedFeatures"
  | "corrupt"
  | "internal";

/** Extension of SlopShop documents (ADR 0009). */
export const DOCUMENT_EXTENSION = "slop";

export type SaveErrorCode =
  | "io"
  | "notASlopFile"
  | "newerVersion"
  | "unsupportedFeatures"
  | "corrupt"
  | "conflict"
  | "readOnly"
  | "busy"
  | "documentClosed"
  | "internal";

/** Why a save failed (the rejection of `saveDocument`). */
export type SaveFailed = {
  /** Translated with the `save.error.<code>` keys. */
  code: SaveErrorCode;
  /** Technical detail inserted in the translated message. */
  detail: string;
};

/** A rendered viewport frame (see FrameHeader in app/src-tauri/src/ipc.rs). */
export type Frame = {
  width: number;
  height: number;
  fit: boolean;
  revision: number;
  /** Low 32 bits of the document id. */
  documentId: number;
  zoom: number;
  origin: [number, number];
  /** Engine-side render time (GPU + readback), in ms. */
  renderMs: number;
  /** RGBA8 sRGB, a view on the received buffer (no copy). */
  pixels: Uint8ClampedArray<ArrayBuffer>;
};

const FRAME_HEADER_LEN = 56;
const FRAME_VERSION = 2;

export function parseFrame(buffer: ArrayBuffer): Frame {
  if (buffer.byteLength < FRAME_HEADER_LEN) throw new Error("frame too short");
  const view = new DataView(buffer);
  const version = view.getUint32(0, true);
  if (version !== FRAME_VERSION) throw new Error(`unsupported frame version ${version}`);
  const width = view.getUint32(4, true);
  const height = view.getUint32(8, true);
  const pixelBytes = width * height * 4;
  if (buffer.byteLength !== FRAME_HEADER_LEN + pixelBytes) {
    throw new Error(`frame size mismatch: ${buffer.byteLength} bytes for ${width}x${height}`);
  }
  return {
    width,
    height,
    fit: (view.getUint32(12, true) & 1) === 1,
    revision: Number(view.getBigUint64(16, true)),
    zoom: view.getFloat64(24, true),
    renderMs: view.getFloat32(32, true),
    documentId: view.getUint32(36, true),
    origin: [view.getFloat64(40, true), view.getFloat64(48, true)],
    pixels: new Uint8ClampedArray(buffer, FRAME_HEADER_LEN, pixelBytes),
  };
}

/** Export file formats. */
/** `psd` and `psb` keep the layers (export_psd in crates/slopshop-io/src/export/psd.rs). */
export type ExportFormat =
  | "png"
  | "tiff"
  | "exr"
  | "jpeg"
  | "webp"
  | "avif"
  | "jxl"
  | "psd"
  | "psb"
  | "bmp"
  | "tga"
  | "pnm"
  | "pfm"
  | "qoi"
  | "ff"
  | "hdr"
  | "ico"
  | "gif"
  | "dds"
  | "fits"
  | "dcm"
  | "pdf"
  | "jp2";
/** Sample types of exported files: 8/16-bit integers, 16/32-bit floats. */
export type ExportSample = "u8" | "u16" | "f16" | "f32";
export type ExportCompression =
  "fast" | "small" | "none" | "deflate" | "lzw" | "lossy" | "lossless" | "rle";
export type ExportSubsampling = "444" | "422" | "420";

/** Export settings (see ExportSpecDto in app/src-tauri/src/ipc.rs). */
export type ExportSpec = {
  format: ExportFormat;
  sample: ExportSample;
  /** `null` for EXR, whose compression is fixed (lossless), and for JPEG. */
  compression: ExportCompression | null;
  /** JPEG (1 to 100), lossy WebP and AVIF (0 to 100); `null` otherwise. */
  quality: number | null;
  /** JPEG only; `null` for the other formats. */
  subsampling: ExportSubsampling | null;
  /** A named space, or `custom` for the document's own unnamed space. */
  space: ColorSpaceId;
  keepAlpha: boolean;
  /** Color transparency is flattened over without alpha: sRGB-encoded [r, g, b] in [0, 1]. */
  matte: [number, number, number];
  /** Only applies to 8-bit samples. */
  dither: boolean;
  /** Gray samples (formats with `gray` in EXPORT_FORMATS): the luminance of the image. */
  gray: boolean;
};

/**
 * What the engine accepts for each format (ExportSpecDto::to_spec): file extensions (the first
 * one is the default), sample types, compressions and JPEG subsamplings, in the order the UI
 * lists them, whether alpha can be kept or dropped (JPEG never keeps it, a layered PSD always
 * does) and whether the format can write gray samples
 * (`has_gray` in crates/slopshop-io/src/export/mod.rs).
 */
export const EXPORT_FORMATS: Record<
  ExportFormat,
  {
    extensions: string[];
    samples: ExportSample[];
    compressions: ExportCompression[];
    subsamplings: ExportSubsampling[];
    alpha: boolean;
    gray: boolean;
  }
> = {
  png: {
    extensions: ["png"],
    samples: ["u8", "u16"],
    compressions: ["fast", "small"],
    subsamplings: [],
    alpha: true,
    gray: true,
  },
  tiff: {
    extensions: ["tif", "tiff"],
    samples: ["u8", "u16", "f32"],
    compressions: ["deflate", "lzw", "none"],
    subsamplings: [],
    alpha: true,
    gray: true,
  },
  exr: {
    extensions: ["exr"],
    samples: ["f32", "f16"],
    compressions: [],
    subsamplings: [],
    alpha: true,
    gray: false,
  },
  jpeg: {
    extensions: ["jpg", "jpeg"],
    samples: ["u8"],
    compressions: [],
    subsamplings: ["444", "422", "420"],
    alpha: false,
    gray: true,
  },
  webp: {
    extensions: ["webp"],
    samples: ["u8"],
    compressions: ["lossy", "lossless"],
    subsamplings: [],
    alpha: true,
    gray: false,
  },
  psd: {
    extensions: ["psd"],
    samples: ["u8", "u16"],
    compressions: [],
    subsamplings: [],
    alpha: false,
    gray: false,
  },
  psb: {
    extensions: ["psb"],
    samples: ["u8", "u16"],
    compressions: [],
    subsamplings: [],
    alpha: false,
    gray: false,
  },
  bmp: {
    extensions: ["bmp"],
    samples: ["u8"],
    compressions: [],
    subsamplings: [],
    alpha: true,
    gray: false,
  },
  tga: {
    extensions: ["tga"],
    samples: ["u8"],
    compressions: ["rle", "none"],
    subsamplings: [],
    alpha: true,
    gray: false,
  },
  pnm: {
    extensions: ["ppm", "pgm", "pam", "pnm"],
    samples: ["u8", "u16"],
    compressions: [],
    subsamplings: [],
    alpha: true,
    gray: true,
  },
  avif: {
    extensions: ["avif"],
    samples: ["u8", "u16"],
    compressions: [],
    subsamplings: [],
    alpha: true,
    gray: true,
  },
  jxl: {
    extensions: ["jxl"],
    samples: ["u8", "u16"],
    compressions: [],
    subsamplings: [],
    alpha: true,
    gray: true,
  },
  pfm: {
    extensions: ["pfm"],
    samples: ["f32"],
    compressions: [],
    subsamplings: [],
    alpha: false,
    gray: true,
  },
  qoi: {
    extensions: ["qoi"],
    samples: ["u8"],
    compressions: [],
    subsamplings: [],
    alpha: true,
    gray: false,
  },
  ff: {
    extensions: ["ff"],
    samples: ["u16"],
    compressions: [],
    subsamplings: [],
    alpha: true,
    gray: false,
  },
  hdr: {
    extensions: ["hdr"],
    samples: ["f32"],
    compressions: [],
    subsamplings: [],
    alpha: false,
    gray: false,
  },
  ico: {
    extensions: ["ico"],
    samples: ["u8"],
    compressions: [],
    subsamplings: [],
    alpha: true,
    gray: false,
  },
  gif: {
    extensions: ["gif"],
    samples: ["u8"],
    compressions: [],
    subsamplings: [],
    alpha: true,
    gray: false,
  },
  dds: {
    extensions: ["dds"],
    samples: ["u8"],
    compressions: [],
    subsamplings: [],
    alpha: true,
    gray: false,
  },
  fits: {
    extensions: ["fits", "fit", "fts"],
    samples: ["u8", "u16", "f32"],
    compressions: [],
    subsamplings: [],
    alpha: false,
    gray: true,
  },
  dcm: {
    extensions: ["dcm", "dicom"],
    samples: ["u8", "u16"],
    compressions: [],
    subsamplings: [],
    alpha: false,
    gray: true,
  },
  pdf: {
    extensions: ["pdf"],
    samples: ["u8"],
    compressions: [],
    subsamplings: [],
    alpha: true,
    gray: true,
  },
  jp2: {
    extensions: ["jp2", "jpf"],
    samples: ["u8", "u16"],
    compressions: ["lossless", "lossy"],
    subsamplings: [],
    alpha: true,
    gray: true,
  },
};

/** The quality a lossy compression starts at, when it is chosen in the dialog. */
export const DEFAULT_QUALITY = 90;

/** An export job has started. */
export type ExportStarted = { id: number; documentId: number; path: string; name: string };
/** Export progress, in rows. */
export type ExportProgress = { id: number; done: number; total: number };

export type ExportNoticeId =
  | "clippedHigh"
  | "clippedLow"
  | "nonFinite"
  | "halfOverflow"
  | "precisionReduced"
  | "bigTiff"
  | "pixelsOutsideCanvas"
  | "alphaFlattened"
  | "colorDiscarded"
  | "colorsQuantized";

/** A report entry, translated with the `export.report.<id>` keys. */
export type ExportNotice = { id: ExportNoticeId; count: number | null };
export type ExportFinished = { id: number; path: string; notices: ExportNotice[] };

export type ExportErrorCode =
  | "io"
  | "source"
  | "cancelled"
  | "unsupportedSpace"
  | "tooLarge"
  | "invalidSpec"
  | "encode"
  | "contentTooComplex"
  | "documentClosed"
  | "internal";

/**
 * Why an export failed: the `export-failed` event (with the job id), or the rejection of
 * `exportDocument` when no job could start (without).
 */
export type ExportFailed = {
  id?: number;
  /** Translated with the `export.error.<code>` keys. */
  code: ExportErrorCode;
  /** Technical detail inserted in the translated message. */
  detail: string;
};

export type GpuInfo = {
  name: string;
  backend: string;
  deviceType: string;
  driver: string;
};

// Tauri runs async commands concurrently, so two quick requests could reach the engine in the
// wrong order. Mutations go through a queue that keeps them in submission order; view
// requests have their own queue (their order relative to edits does not matter). Opens take
// seconds: they run outside the queues so they never block edits.
function makeQueue() {
  let queue: Promise<unknown> = Promise.resolve();
  return function serial<T>(task: () => Promise<T>): Promise<T> {
    const run = queue.then(task, task);
    queue = run.catch(() => undefined);
    return run;
  };
}

const serial = makeQueue();
const serialView = makeQueue();

// Live edits (slider drags) are coalesced: only the latest value still waiting is sent, and
// only within the same document.
let waitingLive: { documentId: number; edit: EditRequest; replace: boolean } | null = null;
// So are the requests of a drag moving selected pixels: each holds the whole move so far.
let waitingPixels: { documentId: number; request: MovePixelsRequest } | null = null;

// Pans and zooms are coalesced into the last request still waiting in the queue (same
// document): deltas add up, zoom factors multiply (around the latest anchor), absolute zooms
// replace each other, so fast input never builds a backlog. Requests that cannot be merged are
// queued after it, in order.
let waitingView: {
  documentId: number;
  request: ViewRequest;
  /** Settles once the (possibly merged) request has been answered. */
  answered: Promise<unknown>;
} | null = null;

function merge(pending: ViewRequest, next: ViewRequest): ViewRequest | null {
  if (pending.kind === "pan" && next.kind === "pan") {
    return { kind: "pan", dx: pending.dx + next.dx, dy: pending.dy + next.dy };
  }
  if (pending.kind === "zoomBy" && next.kind === "zoomBy") {
    return { ...next, factor: pending.factor * next.factor };
  }
  if (pending.kind === "setZoom" && next.kind === "setZoom") return next;
  return null;
}

/** Returned by the engine when a request targets a document that has been closed. */
export const DOCUMENT_CLOSED = "document-closed";

/** Every document request names its document (one per tab). */
/** What the user asks AI for (mirrors `ai::Feature`). */
export type AiFeature = "segmentation" | "subject";

/** A license an AI component comes under. */
export type AiLicense = {
  name: string;
  url: string;
  commercial: boolean;
  /** Must be accepted explicitly before the download (not permissive open source). */
  accept: boolean;
};

/** An AI component: a runtime or a model's files (ADR 0025). */
export type AiComponent = {
  /** Stable id, translated with `ai.component.<id>`. */
  id: string;
  downloadSize: number;
  installedSize: number;
  installed: boolean;
  licenses: AiLicense[];
};

/** Why an install or a removal failed: `code` is translated with `ai.error.<code>`. */
export type AiFailure = { code: string; detail: string };

export type AiProgress = { done: number; total: number };

/** The options bar's brush (see `paint::BrushRequest`): shares between 0 and 1. */
export type BrushRequest = {
  /** Diameter, document pixels. */
  size: number;
  hardness: number;
  /** Distance between dabs, as a share of the diameter. */
  spacing: number;
  flow: number;
  opacity: number;
  pressureSize: boolean;
  pressureOpacity: boolean;
};

/** What a stroke paints (see `paint::PaintTarget`): masks and the selection in gray. */
export type PaintTarget = "layer" | "mask" | "selection";

/** A file a document comes from (File > Document Info); `bytes` null when it is gone. */
export type FileInfo = { path: string; bytes: number | null };

/** File > Document Info (see `info::DocumentInfo`): identifiers and numbers to translate. */
export type DocumentInfo = {
  name: string | null;
  width: number;
  height: number;
  workingSpace: ColorSpaceId;
  blendSpace: "perceptual" | "linear";
  /** Pixels per inch. */
  resolution: number;
  layers: { raster: number; fill: number; adjustment: number; group: number; masks: number };
  /** The raster layers' pixel formats, the most used first. */
  formats: {
    bits: number;
    float: boolean;
    channels: "gray" | "grayAlpha" | "rgb" | "rgba";
    space: ColorSpaceId;
    layers: number;
  }[];
  memoryBytes: number;
  source: FileInfo | null;
  file: FileInfo | null;
};

/** A batch of a Brush or Eraser stroke (see `paint::PaintRequest`). */
export type PaintRequest = {
  /** Batches of one stroke share its id. */
  stroke: number;
  /** What is painted: the layer's pixels, its mask, or the selection (Quick Mask). */
  target: PaintTarget;
  /** The layer painted, or whose mask is painted; unused for the selection. */
  layerId: number;
  brush: BrushRequest;
  /** The Brush's color, sRGB-encoded RGB in [0, 1]; null for the Eraser. */
  color: [number, number, number] | null;
  /** Pointer samples since the last batch: `[x, y, pressure]`, document pixels. */
  samples: [number, number, number][];
  /** The last batch: the stroke is committed (one undo entry). */
  end: boolean;
};

/** A request of a Move tool drag moving selected pixels (see `move_pixels::MovePixelsRequest`). */
export type MovePixelsRequest = {
  /** Requests of one drag share its id. */
  drag: number;
  /** The layer's pixels or its mask. */
  target: Exclude<PaintTarget, "selection">;
  layerId: number;
  /** The move since the drag began, whole document pixels. */
  dx: number;
  dy: number;
  /** Alt: copied, not cut. */
  copy: boolean;
  /** The drag is over (one undo entry). */
  end: boolean;
};

/** A Quick Selection stroke, sent again while painted (see `selection::QuickRequest`). */
export type QuickRequest = {
  /** Requests of one stroke share its image and the selection before it. */
  stroke: number;
  /** The brush's path and radius, document pixels. */
  points: [number, number][];
  radius: number;
  /** The region worked on, document pixels `[x, y, width, height]`. */
  region: [number, number, number, number];
  /** Only that layer, else the composited document. */
  layerId: number | null;
  /** `replace` (a new selection), `add` or `subtract`. */
  mode: SelectionMode;
  /** The stroke goes on (shown, replaced by the next request); else done: one undo entry. */
  live: boolean;
};

export const engine = {
  /** Open documents, in tab order. */
  documents: () => invoke<DocumentView[]>("documents"),
  document: (documentId: number) => invoke<DocumentView>("document", { documentId }),
  /**
   * A new document (File > New): one layer named `layerName`, filled with `background` (sRGB in
   * [0, 1]) or transparent with `null`. A `null` name keeps the tab untitled.
   */
  newDocument: (settings: {
    name: string | null;
    width: number;
    height: number;
    background: [number, number, number] | null;
    layerName: string;
    /** Pixels per inch. */
    resolution: number;
  }) => invoke<DocumentView>("new_document", settings),
  closeDocument: (documentId: number) =>
    serial(() => invoke<void>("close_document", { documentId })),
  /** Rename a document (its tab); the file on disk keeps its name. */
  renameDocument: (documentId: number, name: string) =>
    serial(() => invoke<DocumentView>("rename_document", { documentId, name })),
  /** Move a tab to `index` among the other tabs (the engine keeps the tab order). */
  moveDocument: (documentId: number, index: number) =>
    serial(() => invoke<void>("move_document", { documentId, index })),
  /** Copy every layer of `sourceId` on top of `targetId` (one undo entry in the target). */
  /** Every layer of the source (grouped if several), or only `layerIds`. */
  copyLayers: (sourceId: number, targetId: number, layerIds: number[] | null = null) =>
    serial(() => invoke<DocumentView>("copy_layers", { sourceId, targetId, layerIds })),
  /** Opens in progress, and recent failures (for when their events were missed). */
  openings: () => invoke<Opening[]>("openings"),
  openFailures: () => invoke<OpenFailed[]>("open_failures"),
  /**
   * Decode images in parallel (seconds for large ones), each into a new tab or, with a
   * `documentId`, each as a new top layer of that document; tabs and layers come in the order
   * of `paths`. Outcomes arrive as `open-*` events; resolves once every image is done.
   */
  openImages: (paths: string[], documentId: number | null) =>
    invoke<OpenSummary>("open_images", { paths, documentId }),
  /**
   * A PDF's or an SVG's pages, for the import dialog; the file stays parsed for its next calls,
   * until `openVectorPages` or `closeVector`. Rejects if the file cannot be read.
   */
  vectorInfo: (path: string) => invoke<VectorInfo>("vector_info", { path }),
  /** A page (from 0), at most `maxSide` pixels on its longer side (RGBA8 sRGB, straight). */
  vectorThumbnail: async (path: string, page: number, maxSide: number) => {
    const buffer = await invoke<ArrayBuffer>("vector_thumbnail", { path, page, maxSide });
    const view = new DataView(buffer);
    const width = view.getUint32(0, true);
    const height = view.getUint32(4, true);
    const pixels = new Uint8ClampedArray(buffer, 8, width * height * 4);
    return new ImageData(pixels, width, height);
  },
  /**
   * File > Import from Device (Windows): Windows' scanning dialog, then the image in a new
   * untitled tab. `openFailed`: the open's own events reported why.
   */
  acquireImage: () => invoke<"opened" | "cancelled" | "noDevice" | "openFailed">("acquire_image"),
  /** File > Print's page: a JPEG of the document as displayed, over white paper. */
  printPage: (documentId: number) => invoke<ArrayBuffer>("print_page", { documentId }),
  /** File > Document Info. */
  documentInfo: (documentId: number) => invoke<DocumentInfo>("document_info", { documentId }),
  /** File > Open Recent: files and folders opened or saved, newest first (existing ones). */
  recentFiles: () => invoke<string[]>("recent_files"),
  /** File > Open Recent > Clear Recent File List. */
  clearRecentFiles: () => invoke<void>("clear_recent_files"),
  /**
   * The welcome page's thumbnail of a recent entry (RGBA8 sRGB); rejects when there is none
   * (folders, archives).
   */
  recentThumbnail: async (path: string) => {
    const buffer = await invoke<ArrayBuffer>("recent_thumbnail", { path });
    const view = new DataView(buffer);
    const width = view.getUint32(0, true);
    const height = view.getUint32(4, true);
    const pixels = new Uint8ClampedArray(buffer, 8, width * height * 4);
    return new ImageData(pixels, width, height);
  },
  /** The import dialog was cancelled. */
  closeVector: (path: string) => invoke<void>("close_vector", { path }),
  /**
   * Open pages (from 0) of a PDF or an SVG rendered at `dpi` as one document: a new tab or,
   * with a `documentId`, a group on top of that document. A PDF's pages are one group over a
   * white fill named `background`. The outcome arrives as `open-*` events, like `openImages`.
   */
  openVectorPages: (
    path: string,
    pages: number[],
    dpi: number,
    documentId: number | null,
    background: string,
  ) => invoke<void>("open_vector_pages", { path, pages, dpi, documentId, background }),
  /**
   * Save a document to its `.slop` file (incremental), or to `path` (Save As: a new compact
   * file the document continues with). Rejects with a `SaveFailed`. Queued after the edits
   * already sent, so the save includes them.
   */
  saveDocument: (documentId: number, path: string | null) =>
    serial(() => invoke<DocumentView>("save_document", { documentId, path })),
  /** Close the window even with unsaved changes (after asking the user). */
  quit: () => invoke<void>("quit"),
  /**
   * Thumbnail of a raster layer, at most `maxSide` pixels on its longer side: RGBA8 sRGB with
   * straight alpha, ready for a canvas (raw binary: width, height, then the pixels).
   */
  layerThumbnail: async (documentId: number, layerId: number, maxSide: number, mask = false) => {
    const buffer = await invoke<ArrayBuffer>("layer_thumbnail", {
      documentId,
      layerId,
      maxSide,
      mask,
    });
    const view = new DataView(buffer);
    const width = view.getUint32(0, true);
    const height = view.getUint32(4, true);
    const pixels = new Uint8ClampedArray(buffer, 8, width * height * 4);
    return new ImageData(pixels, width, height);
  },
  /** Add a mask made from the transparency of a raster layer (one undo entry). */
  /** The layer showing a pixel at document pixel (x, y): the Move tool's Auto-Select. */
  layerAt: (documentId: number, x: number, y: number) =>
    invoke<number | null>("layer_at", { documentId, x, y }),
  /**
   * The selection's bounds when document point (x, y) is inside it (as its outline shows),
   * else null: a Move tool drag from there moves the selected pixels.
   */
  selectionBoundsAt: (documentId: number, x: number, y: number) =>
    invoke<Bounds | null>("selection_bounds_at", { documentId, x, y }),
  /**
   * Move the selected pixels (the Move tool inside a selection), live; the request with `end`
   * makes the drag one undo entry. Resolves to null when merged into a request of the same drag
   * still waiting (which then sends this one's move).
   */
  movePixels: (documentId: number, request: MovePixelsRequest): Promise<DocumentView | null> => {
    if (
      waitingPixels &&
      waitingPixels.documentId === documentId &&
      waitingPixels.request.drag === request.drag
    ) {
      waitingPixels.request = request;
      return Promise.resolve(null);
    }
    const slot = { documentId, request };
    waitingPixels = slot;
    return serial(() => {
      if (waitingPixels === slot) waitingPixels = null;
      return invoke<DocumentView>("move_selected_pixels", { documentId, request: slot.request });
    });
  },
  /** What moving `ids` can snap to (bounds in document pixels). */
  moveSnapTargets: (documentId: number, ids: number[]) =>
    invoke<SnapTargets>("move_snap_targets", { documentId, ids }),
  addMaskFromTransparency: (documentId: number, layerId: number) =>
    serial(() => invoke<DocumentView>("add_mask_from_transparency", { documentId, layerId })),
  /** Select `shape` combined with the selection by `mode` (ADR 0024); feather in pixels. */
  selectShape: (
    documentId: number,
    shape: SelectionShape,
    mode: SelectionMode,
    antiAlias: boolean,
    feather: number,
  ) =>
    serial(() =>
      invoke<DocumentView>("select_shape", { documentId, shape, mode, antiAlias, feather }),
    ),
  selectAll: (documentId: number) =>
    serial(() => invoke<DocumentView>("select_all", { documentId })),
  deselect: (documentId: number) => serial(() => invoke<DocumentView>("deselect", { documentId })),
  reselect: (documentId: number) => serial(() => invoke<DocumentView>("reselect", { documentId })),
  invertSelection: (documentId: number) =>
    serial(() => invoke<DocumentView>("invert_selection", { documentId })),
  /**
   * Layer > Layer Mask: Reveal All, Hide All, Reveal Selection or Hide Selection on the layers
   * without a mask among `layerIds`, as one undo entry (a mask from the selection deselects).
   */
  addLayerMasks: (documentId: number, layerIds: number[], kind: LayerMaskKind) =>
    serial(() => invoke<DocumentView>("add_layer_masks", { documentId, layerIds, kind })),
  /**
   * The Magic Wand at document pixel (x, y): colors within `tolerance` (0–255) of it, connected
   * or not; it samples the composited document, or only `layerId`'s layer.
   */
  magicWand: (
    documentId: number,
    at: { x: number; y: number },
    options: { tolerance: number; contiguous: boolean; antiAlias: boolean },
    layerId: number | null,
    mode: SelectionMode,
  ) =>
    serial(() =>
      invoke<DocumentView>("magic_wand", { documentId, ...at, ...options, layerId, mode }),
    ),
  /** Select > Color Range's preview: width, height and 8-bit coverage per pixel. */
  colorRangePreview: async (documentId: number, request: ColorRangeRequest, maxSide: number) => {
    const buffer = await invoke<ArrayBuffer>("color_range_preview", {
      documentId,
      request,
      maxSide,
    });
    const view = new DataView(buffer);
    const width = view.getUint32(0, true);
    const height = view.getUint32(4, true);
    return { width, height, gray: new Uint8Array(buffer, 8, width * height) };
  },
  /** Select > Color Range: the sampled colors, within the selection if any. */
  colorRange: (documentId: number, request: ColorRangeRequest) =>
    serial(() => invoke<DocumentView>("color_range", { documentId, request })),
  /** Select > Modify: the whole selection changed by `amount` pixels. */
  modifySelection: (documentId: number, kind: SelectionModify, amount: number) =>
    serial(() => invoke<DocumentView>("modify_selection", { documentId, kind, amount })),
  /** Image > Crop with a selection: the canvas becomes the selection's bounds. */
  cropToSelection: (documentId: number) =>
    serial(() => invoke<DocumentView>("crop_to_selection", { documentId })),
  /** Select > Edit in Quick Mask Mode (Q): the view tints what the selection leaves out. */
  setQuickMask: (documentId: number, on: boolean) =>
    serial(() => invoke<DocumentView>("set_quick_mask", { documentId, on })),
  /**
   * The marching ants over a region (document pixels) at `zoom` (screen pixels per document
   * pixel), sized to the view, as polylines in document pixels.
   */
  selectionOutline: async (
    documentId: number,
    region: { x: number; y: number; width: number; height: number },
    zoom: number,
  ): Promise<SelectionOutline> => {
    const buffer = await invoke<ArrayBuffer>("selection_outline", { documentId, ...region, zoom });
    const words = new Uint32Array(buffer);
    const lines: SelectionOutline = [];
    let at = 1;
    for (let i = 0; i < (words[0] ?? 0); i++) {
      const n = words[at++];
      lines.push(words.subarray(at, at + 2 * n));
      at += 2 * n;
    }
    return lines;
  },
  /**
   * Paste into `documentId` (a new tab without one): SlopShop's own copy while the system
   * clipboard still holds the image it got, else what another application put there (files are
   * returned for the app to open, an image becomes a layer named `name`). `view` is the part
   * of the document shown, where a paste out of sight lands. `name` also names the group of a
   * Paste Into. One undo entry. Queued after the edits already sent.
   */
  paste: (
    documentId: number | null,
    name: string,
    kind: PasteKind,
    view: [number, number, number, number] | null,
  ) => serial(() => invoke<Pasted>("paste", { documentId, name, kind, view })),
  /**
   * Edit > Copy (layers, or the selected pixels of a layer or its mask) and Copy Merged: kept
   * whole for Paste, and an 8-bit image of it for other applications. Resolves with whether
   * anything was copied (`false`: the selection holds nothing of the layer). Queued after the
   * edits already sent.
   */
  /** File > New's Clipboard preset: the size of the document a paste would make, if any. */
  clipboardSize: () => invoke<[number, number] | null>("clipboard_size"),
  copy: (documentId: number, request: CopyRequest) =>
    serial(() => invoke<boolean>("copy", { documentId, request })),
  /** Show a file (e.g. an exported one) selected in the system's file manager. */
  revealInFolder: (path: string) => invoke<void>("reveal_in_folder", { path }),
  /**
   * The AI components `feature` needs on this machine, or without a feature every one it can
   * use (and any other still installed). `null`: AI is not offered on this platform yet.
   */
  /** The AI runtime on this machine (`directml`, `coreml`, `cpu`), or null where AI is not offered. */
  aiRuntime: () => invoke<string | null>("ai_runtime"),
  aiComponents: (feature: AiFeature | null) =>
    invoke<AiComponent[] | null>("ai_components", { feature }),
  /** Downloads components in turn; one install at a time. Rejects with an `AiFailure`. */
  aiInstall: (ids: string[], onProgress: (progress: AiProgress) => void) => {
    const progress = new Channel<AiProgress>();
    progress.onmessage = onProgress;
    return invoke<void>("ai_install", { ids, progress });
  },
  /** Stops the install running; what it fetched is kept for the next attempt. */
  aiCancelInstall: () => invoke<void>("ai_cancel_install"),
  aiRemove: (id: string) => invoke<void>("ai_remove", { id }),
  /** Opens one of the components' licenses in the browser. */
  aiOpenLicense: (url: string) => invoke<void>("ai_open_license", { url }),
  /**
   * Object Selection's hover: the object under document point (`x`, `y`) as the model's mask
   * over `region` (`side`² cells, 255 inside), or null when there is none.
   */
  aiObjectHover: async (
    documentId: number,
    x: number,
    y: number,
    region: [number, number, number, number],
    layerId: number | null,
  ): Promise<{ side: number; mask: Uint8Array } | null> => {
    const bytes = await invoke<ArrayBuffer>("ai_object_hover", {
      documentId,
      x,
      y,
      region,
      layerId,
    });
    const side = new DataView(bytes).getUint32(0, true);
    return side > 0 ? { side, mask: new Uint8Array(bytes, 4, side * side) } : null;
  },
  /** Object Selection: the object at a point, or in a box, as one undo entry. */
  aiObjectSelect: (
    documentId: number,
    request: {
      point: [number, number] | null;
      /** `[left, top, right, bottom]`, document pixels. */
      box: [number, number, number, number] | null;
      region: [number, number, number, number];
      layerId: number | null;
      mode: SelectionMode;
      /** Refine the edge at full resolution (ViTMatte). */
      refine: boolean;
      /** Follows the request (`onAiProgress`) and cancels it (`aiCancel`). */
      task: number;
    },
  ) => serial(() => invoke<DocumentView>("ai_object_select", { documentId, request })),
  /** Select > Subject: the image's main subject (BiRefNet), as one undo entry. */
  aiSelectSubject: (
    documentId: number,
    layerId: number | null,
    mode: SelectionMode,
    refine: boolean,
    task: number,
  ) =>
    serial(() =>
      invoke<DocumentView>("ai_select_subject", { documentId, layerId, mode, refine, task }),
    ),
  /** Select > Refine Edge: the selection's edge matted within `radius` pixels (ViTMatte). */
  aiRefineSelection: (documentId: number, radius: number, layerId: number | null, task: number) =>
    serial(() =>
      invoke<DocumentView>("ai_refine_selection", { documentId, radius, layerId, task }),
    ),
  /** Cancels an AI request: it stops at its next step, changing nothing (code `cancelled`). */
  aiCancel: (task: number) => invoke<void>("ai_cancel", { task }),
  /**
   * Paint a batch of a stroke (ADR 0027): shown at once, the view redraws; resolves to the
   * document once the stroke is committed, else `null`.
   */
  paintStroke: (documentId: number, request: PaintRequest) =>
    serial(() => invoke<DocumentView | null>("paint_stroke", { documentId, request })),
  /**
   * Delete with a selection: the selected part of a raster layer erased (`color` null) or
   * filled with `color` (sRGB-encoded RGB in [0, 1]), as paint (ADR 0027); of its mask
   * (`target` "mask"): hidden, or filled with the color's gray.
   */
  /**
   * The color picker's eyedropper: the color shown at a document point (every visible layer),
   * whole 8-bit sRGB values; null outside the canvas or where nothing is shown.
   */
  sampleColor: (documentId: number, x: number, y: number) =>
    serial(() => invoke<[number, number, number] | null>("sample_color", { documentId, x, y })),
  fillSelection: (
    documentId: number,
    layerId: number,
    target: Exclude<PaintTarget, "selection">,
    color: [number, number, number] | null,
  ) => serial(() => invoke<DocumentView>("fill_selection", { documentId, layerId, target, color })),
  /** Quick Selection: the stroke so far, shown live, or done (one undo entry). */
  quickSelect: (documentId: number, request: QuickRequest) =>
    serial(() => invoke<DocumentView>("quick_select", { documentId, request })),
  perform: (documentId: number, edit: EditRequest) =>
    serial(() => invoke<DocumentView>("perform", { documentId, edit })),
  /**
   * Apply an edit immediately as part of a gesture (one undo entry for the whole gesture).
   * `replace`: the edit is the whole gesture so far (e.g. a move since the drag began), which
   * replaces what the gesture applied before. Resolves to `null` when merged into a live edit
   * that was already waiting (which is then replaced: only use with absolute edits).
   */
  performLive: (
    documentId: number,
    edit: EditRequest,
    replace = false,
  ): Promise<DocumentView | null> => {
    if (waitingLive && waitingLive.documentId === documentId) {
      waitingLive.edit = edit;
      waitingLive.replace = replace;
      return Promise.resolve(null);
    }
    const slot = { documentId, edit, replace };
    waitingLive = slot;
    return serial(() => {
      if (waitingLive === slot) waitingLive = null;
      return invoke<DocumentView>("perform_live", {
        documentId,
        edit: slot.edit,
        replace: slot.replace,
      });
    });
  },
  endGesture: (documentId: number) =>
    serial(() => invoke<DocumentView>("end_gesture", { documentId })),
  /** Revert the gesture in progress, leaving no undo entry (Esc during a transform). */
  cancelGesture: (documentId: number) =>
    serial(() => invoke<DocumentView>("cancel_gesture", { documentId })),
  undo: (documentId: number) => serial(() => invoke<DocumentView>("undo", { documentId })),
  redo: (documentId: number) => serial(() => invoke<DocumentView>("redo", { documentId })),
  gpuInfo: () => invoke<GpuInfo>("gpu_info"),
  presenterMode: () => invoke<PresenterMode>("presenter_mode"),
  /** Native presentation: show a document's current view in `rect` of the window. */
  presentView: (documentId: number, rect: DeviceRect) =>
    invoke<PresentInfo>("present_view", { documentId, ...rect }),
  /**
   * Change a document's view. Resolves to `null` when merged into a request that was already
   * waiting: that one resolves with the combined result, and this one right after it, so that
   * awaiting any request means the view has taken it into account.
   */
  view: (documentId: number, request: ViewRequest): Promise<ViewInfo | null> => {
    if (waitingView && waitingView.documentId === documentId) {
      const merged = merge(waitingView.request, request);
      if (merged) {
        waitingView.request = merged;
        return waitingView.answered.then(
          () => null,
          () => null,
        );
      }
    }
    const slot: NonNullable<typeof waitingView> = {
      documentId,
      request,
      answered: Promise.resolve(),
    };
    waitingView = slot;
    const answer = serialView(() => {
      if (waitingView === slot) waitingView = null;
      return invoke<ViewInfo>("view", { documentId, request: slot.request });
    });
    slot.answered = answer;
    return answer;
  },
  /** The settings an export of a document to `format` starts with. */
  exportDefaults: (documentId: number, format: ExportFormat) =>
    invoke<ExportSpec>("export_defaults", { documentId, format }),
  /** The named color spaces `format` can store and tag. */
  /** Named spaces the format can store and tag, for color or gray samples. */
  exportSpaces: (format: ExportFormat, gray: boolean) =>
    invoke<ColorSpaceId[]>("export_spaces", { format, gray }),
  /** The largest width or height `format` can store (`null`: no limit). */
  exportMaxSide: (format: ExportFormat) => invoke<number | null>("export_max_side", { format }),
  /**
   * Start exporting a document to `path` (overwritten): resolves to the job id at once, and
   * rejects with an `ExportFailed` (no id) when the export cannot start. Queued after the edits
   * already requested, so that they are exported; later edits are not.
   */
  exportDocument: (documentId: number, path: string, spec: ExportSpec) =>
    serial(() => invoke<number>("export_document", { documentId, path, spec })),
  /** Ask an export to stop; it then fails with the code `cancelled` (unless already done). */
  cancelExport: (jobId: number) => invoke<void>("cancel_export", { jobId }),
  /** Render a document's current view into a width × height (device pixels) viewport. */
  renderView: async (documentId: number, width: number, height: number): Promise<Frame> =>
    parseFrame(await invoke<ArrayBuffer>("render_view", { documentId, width, height })),
};

/**
 * Open progress and outcomes, whoever started them (e.g. files opened at startup). Resolves
 * once the listeners are registered, so state read afterwards cannot miss an event.
 */
export async function onOpenEvents(handlers: {
  started: (opening: Opening) => void;
  finished: (finished: OpenFinished) => void;
  failed: (failed: OpenFailed) => void;
}): Promise<() => void> {
  const unlisten = await Promise.all([
    listen<Opening>("open-started", (e) => handlers.started(e.payload)),
    listen<OpenFinished>("open-finished", (e) => handlers.finished(e.payload)),
    listen<OpenFailed>("open-failed", (e) => handlers.failed(e.payload)),
  ]);
  return () => unlisten.forEach((stop) => stop());
}

/** The steps an AI request has done, of those it knows of (`AiProgress` in segment.rs). */
export type AiTaskProgress = {
  task: number;
  /** `select` (the model), then `refine` (Refine Edge's windows). */
  stage: "select" | "refine";
  done: number;
  total: number;
};

/** The recent files changed (opened, saved, cleared). Resolves once the listener is registered. */
export async function onRecentFiles(handler: (paths: string[]) => void): Promise<() => void> {
  return listen<string[]>("recent-files", (e) => handler(e.payload));
}

/** AI requests' progress. Resolves once the listener is registered. */
export async function onAiProgress(
  handler: (progress: AiTaskProgress) => void,
): Promise<() => void> {
  return listen<AiTaskProgress>("ai-progress", (e) => handler(e.payload));
}

/** Export progress and outcomes. Resolves once the listeners are registered. */
export async function onExportEvents(handlers: {
  started: (started: ExportStarted) => void;
  progress: (progress: ExportProgress) => void;
  finished: (finished: ExportFinished) => void;
  failed: (failed: ExportFailed) => void;
}): Promise<() => void> {
  const unlisten = await Promise.all([
    listen<ExportStarted>("export-started", (e) => handlers.started(e.payload)),
    listen<ExportProgress>("export-progress", (e) => handlers.progress(e.payload)),
    listen<ExportFinished>("export-finished", (e) => handlers.finished(e.payload)),
    listen<ExportFailed>("export-failed", (e) => handlers.failed(e.payload)),
  ]);
  return () => unlisten.forEach((stop) => stop());
}
