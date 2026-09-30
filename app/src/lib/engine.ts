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
  kind: "fill" | "raster";
  /** sRGB-encoded RGBA in [0, 1], for display swatches only. */
  swatch: [number, number, number, number];
  /** Translated with the `blendMode.<id>` keys. */
  blendMode: BlendModeId;
};

/** Blend modes (BlendMode in crates/slopshop-core/src/blend.rs, ADR 0012). */
export type BlendModeId =
  | "normal"
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
  ["normal"],
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
  /** `index` is the final position in the stack, 0 = bottom. */
  | { kind: "moveLayer"; id: number; index: number }
  | { kind: "setLayerBlendMode"; id: number; mode: BlendModeId }
  | { kind: "setBlendSpace"; space: BlendSpaceId };

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
  | "colorInfoUnsupported";

export type OpenErrorCode =
  | "io"
  | "decode"
  | "notYetSupported"
  | "heic"
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
export type ExportFormat = "png" | "tiff" | "exr" | "jpeg" | "webp";
/** Sample types of exported files: 8/16-bit integers, 16/32-bit floats. */
export type ExportSample = "u8" | "u16" | "f16" | "f32";
export type ExportCompression =
  "fast" | "small" | "none" | "deflate" | "lzw" | "lossy" | "lossless";
export type ExportSubsampling = "444" | "422" | "420";

/** Export settings (see ExportSpecDto in app/src-tauri/src/ipc.rs). */
export type ExportSpec = {
  format: ExportFormat;
  sample: ExportSample;
  /** `null` for EXR, whose compression is fixed (lossless), and for JPEG. */
  compression: ExportCompression | null;
  /** JPEG (1 to 100) and lossy WebP (0 to 100); `null` otherwise. */
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
 * lists them, whether the format can keep alpha and whether it can write gray samples
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
  | "alphaFlattened"
  | "colorDiscarded";

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
let waitingLive: { documentId: number; edit: EditRequest } | null = null;

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
  copyLayers: (sourceId: number, targetId: number) =>
    serial(() => invoke<DocumentView>("copy_layers", { sourceId, targetId })),
  /** Opens in progress, and recent failures (for when their events were missed). */
  openings: () => invoke<Opening[]>("openings"),
  openFailures: () => invoke<OpenFailed[]>("open_failures"),
  /**
   * Decode images in parallel (seconds for large ones), each into a new tab or, with a
   * `documentId`, each as a new top layer of that document; tabs and layers come in the order
   * of `paths`. Outcomes arrive as `open-*` events; resolves once every image is done.
   */
  openImages: (paths: string[], documentId: number | null) =>
    invoke<void>("open_images", { paths, documentId }),
  /**
   * Save a document to its `.slop` file (incremental), or to `path` (Save As: a new compact
   * file the document continues with). Rejects with a `SaveFailed`. Queued after the edits
   * already sent, so the save includes them.
   */
  saveDocument: (documentId: number, path: string | null) =>
    serial(() => invoke<DocumentView>("save_document", { documentId, path })),
  /** Close the window even with unsaved changes (after asking the user). */
  quit: () => invoke<void>("quit"),
  /** Show a file (e.g. an exported one) selected in the system's file manager. */
  revealInFolder: (path: string) => invoke<void>("reveal_in_folder", { path }),
  perform: (documentId: number, edit: EditRequest) =>
    serial(() => invoke<DocumentView>("perform", { documentId, edit })),
  /**
   * Apply an edit immediately as part of a gesture (one undo entry for the whole gesture).
   * Resolves to `null` when merged into a live edit that was already waiting.
   */
  performLive: (documentId: number, edit: EditRequest): Promise<DocumentView | null> => {
    if (waitingLive && waitingLive.documentId === documentId) {
      waitingLive.edit = edit;
      return Promise.resolve(null);
    }
    const slot = { documentId, edit };
    waitingLive = slot;
    return serial(() => {
      if (waitingLive === slot) waitingLive = null;
      return invoke<DocumentView>("perform_live", { documentId, edit: slot.edit });
    });
  },
  endGesture: (documentId: number) =>
    serial(() => invoke<DocumentView>("end_gesture", { documentId })),
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
