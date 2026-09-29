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
};

export type DocumentView = {
  /** One document per tab; ids are never reused. */
  id: number;
  /** File name, or null for an untitled document. */
  name: string | null;
  width: number;
  height: number;
  /** Identifier, translated with the `colorSpace.<id>` i18n keys. */
  workingSpace: ColorSpaceId;
  revision: number;
  canUndo: boolean;
  canRedo: boolean;
  /** Bottom to top. */
  layers: LayerView[];
  /** How the source file was interpreted; translated with the `open.warning.<id>` keys. */
  warnings: ImportWarning[];
};

export type EditRequest =
  | { kind: "addFillLayer"; name: string; color: [number, number, number, number] }
  | { kind: "removeLayer"; id: number }
  | { kind: "setLayerVisible"; id: number; visible: boolean }
  | { kind: "setLayerOpacity"; id: number; opacity: number }
  | { kind: "renameLayer"; id: number; name: string }
  /** `index` is the final position in the stack, 0 = bottom. */
  | { kind: "moveLayer"; id: number; index: number };

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
  | "internal";

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
  /** Opens in progress, and recent failures (for when their events were missed). */
  openings: () => invoke<Opening[]>("openings"),
  openFailures: () => invoke<OpenFailed[]>("open_failures"),
  /** Decode an image into a new tab (seconds for large images). */
  openImage: (path: string) => invoke<DocumentView>("open_image", { path }),
  /** Decode an image into a new top layer of a document (undoable). */
  addImageLayer: (documentId: number, path: string) =>
    invoke<DocumentView>("add_image_layer", { documentId, path }),
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
