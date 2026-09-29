// Typed access to the engine over Tauri IPC.
// Mirrors app/src-tauri/src/ipc.rs — keep both in sync.
//
// The UI never computes image content: it sends intents (edits) and displays what the engine
// returns. Frames arrive as raw binary (ArrayBuffer), never as JSON.

import { invoke } from "@tauri-apps/api/core";

export type LayerView = {
  id: number;
  name: string;
  visible: boolean;
  opacity: number;
  kind: "fill";
  /** sRGB-encoded RGBA in [0, 1], for display swatches only. */
  swatch: [number, number, number, number];
};

export type DocumentView = {
  width: number;
  height: number;
  /** Identifier, translated with the `colorSpace.<id>` i18n keys. */
  workingSpace: "linear-srgb" | "srgb";
  revision: number;
  canUndo: boolean;
  canRedo: boolean;
  /** Bottom to top. */
  layers: LayerView[];
};

export type EditRequest =
  | { kind: "addFillLayer"; name: string; color: [number, number, number, number] }
  | { kind: "removeLayer"; id: number }
  | { kind: "setLayerVisible"; id: number; visible: boolean }
  | { kind: "setLayerOpacity"; id: number; opacity: number }
  | { kind: "renameLayer"; id: number; name: string }
  /** `index` is the final position in the stack, 0 = bottom. */
  | { kind: "moveLayer"; id: number; index: number };

export type GpuInfo = {
  name: string;
  backend: string;
  deviceType: string;
  driver: string;
};

// Tauri runs async commands concurrently, so two quick edits could reach the engine in the
// wrong order. Every mutation goes through this queue to keep them in submission order.
let queue: Promise<unknown> = Promise.resolve();

function serial<T>(task: () => Promise<T>): Promise<T> {
  const run = queue.then(task, task);
  queue = run.catch(() => undefined);
  return run;
}

// Live edits (slider drags) are coalesced: only the latest pending value is sent.
let pendingLive: EditRequest | null = null;

export const engine = {
  document: () => invoke<DocumentView>("document"),
  perform: (edit: EditRequest) => serial(() => invoke<DocumentView>("perform", { edit })),
  /**
   * Apply an edit immediately as part of a gesture (one undo entry for the whole gesture).
   * Resolves to `null` when superseded by a newer live edit before being sent.
   */
  performLive: (edit: EditRequest): Promise<DocumentView | null> => {
    const alreadyQueued = pendingLive !== null;
    pendingLive = edit;
    if (alreadyQueued) return Promise.resolve(null);
    return serial(async () => {
      const latest = pendingLive;
      pendingLive = null;
      return latest ? invoke<DocumentView>("perform_live", { edit: latest }) : null;
    });
  },
  endGesture: () => serial(() => invoke<DocumentView>("end_gesture")),
  undo: () => serial(() => invoke<DocumentView>("undo")),
  redo: () => serial(() => invoke<DocumentView>("redo")),
  gpuInfo: () => invoke<GpuInfo>("gpu_info"),
  /** RGBA8 sRGB pixels of the document fitted into a width × height viewport. */
  renderView: (width: number, height: number) =>
    invoke<ArrayBuffer>("render_view", { width, height }),
};
