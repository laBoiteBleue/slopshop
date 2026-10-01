// Typed access to the engine over Tauri IPC.
// Mirrors app/src-tauri/src/ipc.rs — keep both in sync.
//
// The UI never computes image content: it sends intents (edits) and displays what the engine
// returns. Frames arrive as raw binary (ArrayBuffer), never as JSON.

import { invoke } from "@tauri-apps/api/core";
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
};

export type EditRequest =
  | { kind: "addFillLayer"; name: string; color: [number, number, number, number] }
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
  | { kind: "resizeImage"; width: number; height: number }
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
  revision: number;
  zoom: number;
  fit: boolean;
  renderMs: number;
};

/** What a paste found on the clipboard (Pasted in app/src-tauri/src/lib.rs). */
export type Pasted =
  | { kind: "files" }
  | { kind: "image"; document: DocumentView; newTab: boolean }
  | { kind: "nothing" };

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
  | "dds";
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
export const engine = {
  /** Open documents, in tab order. */
  documents: () => invoke<DocumentView[]>("documents"),
  document: (documentId: number) => invoke<DocumentView>("document", { documentId }),
  newDocument: () => invoke<DocumentView>("new_document"),
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
  /** What moving `ids` can snap to (bounds in document pixels). */
  moveSnapTargets: (documentId: number, ids: number[]) =>
    invoke<SnapTargets>("move_snap_targets", { documentId, ids }),
  addMaskFromTransparency: (documentId: number, layerId: number) =>
    serial(() => invoke<DocumentView>("add_mask_from_transparency", { documentId, layerId })),
  /**
   * Paste the clipboard: copied files open like dropped ones (layers of `documentId`, or new
   * tabs; outcomes arrive as `open-*` events), a copied image becomes a layer named `name` of
   * `documentId`, or a new tab.
   */
  paste: (documentId: number | null, name: string) => invoke<Pasted>("paste", { documentId, name }),
  /** Show a file (e.g. an exported one) selected in the system's file manager. */
  revealInFolder: (path: string) => invoke<void>("reveal_in_folder", { path }),
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
