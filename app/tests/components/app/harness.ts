// The whole UI on a fake engine, for the app's tests (the `*.test.ts` files of this folder): a
// few documents in memory behind the IPC, answering the commands the tests go through
// (anything else answers nothing). A test file adds the answers it needs with `respond`, at its
// top level; importing this module sets the mocks up before each test and clears them after.
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { cleanup, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, vi } from "vitest";
import App from "../../../src/App.svelte";
import type { DocumentView, EditRequest, LayerView } from "../../../src/lib/engine";

class FixedSizeObserver {
  constructor(private readonly callback: ResizeObserverCallback) {}
  observe(target: Element) {
    // With its target: Svelte's size bindings (the selection outline's) key entries by it.
    const entry = {
      target,
      contentRect: { width: 200, height: 100 },
    } as unknown as ResizeObserverEntry;
    this.callback([entry], this as unknown as ResizeObserver);
  }
  unobserve() {}
  disconnect() {}
}

export function layer(id: number, name: string): LayerView {
  return {
    id,
    name,
    visible: true,
    opacity: 1,
    kind: "raster",
    swatch: [0, 0, 0, 0],
    blendMode: "normal",
    contentKey: id,
    hasAlpha: false,
    mask: null,
    children: [],
    passThrough: false,
    clipped: false,
    transform: [1, 0, 0, 1, 0, 0],
    painted: false,
    entries: [],
    adjustment: null,
  };
}

export function documentView(id: number, name: string | null, layers: LayerView[]): DocumentView {
  return {
    id,
    name,
    width: 400,
    height: 300,
    workingSpace: "srgb",
    blendSpace: "perceptual",
    resolution: 72,
    revision: 0,
    canUndo: true,
    canRedo: false,
    layers,
    warnings: [],
    path: null,
    dirty: false,
    selectionKey: null,
    canReselect: false,
    quickMask: false,
    quickMaskOpacity: 50,
    savedSelections: [],
  } as DocumentView;
}

/** A 1 × 1 frame of `documentId` at 100% (see `parseFrame` in engine.ts). */
function frame(documentId: number): ArrayBuffer {
  const buffer = new ArrayBuffer(56 + 4);
  const view = new DataView(buffer);
  view.setUint32(0, 2, true);
  view.setUint32(4, 1, true);
  view.setUint32(8, 1, true);
  view.setFloat64(24, 1, true);
  view.setUint32(36, documentId, true);
  return buffer;
}

/** The documents behind the IPC. */
export let documents: DocumentView[] = [];
/** Every command the UI sent, with its arguments. */
export let calls: { cmd: string; args: Record<string, unknown> }[] = [];

export const sent = (cmd: string) => calls.filter((c) => c.cmd === cmd).map((c) => c.args);

/** An answer to a command: its arguments, and the document they name (if any). */
type Answer = (args: Record<string, unknown>, doc: DocumentView) => unknown;

const answers = new Map<string, Answer>();

/** Answer `cmd` with `answer`, for the tests of the file that calls it (at its top level). */
export function respond(cmd: string, answer: Answer) {
  answers.set(cmd, answer);
}

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", FixedSizeObserver);
  documents = [];
  calls = [];
  mockIPC(
    (cmd, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args });
      const find = () => documents.find((d) => d.id === args.documentId) as DocumentView;
      const answer = answers.get(cmd);
      if (answer) return answer(args, find());
      switch (cmd) {
        case "presenter_mode":
          return "frames";
        case "documents":
          return documents;
        case "openings":
        case "open_failures":
        case "recent_files":
          return [];
        case "clipboard_contents":
          return "nothing";
        case "render_view":
          return frame(args.documentId as number);
        case "view":
          return { zoom: 1, origin: [0, 0], fit: true };
        case "new_document": {
          const doc = documentView(documents.length + 1, args.name as string | null, [
            layer(1, args.layerName as string),
          ]);
          documents.push(doc);
          return doc;
        }
        case "undo":
          return { ...find(), revision: find().revision + 1, canUndo: false };
        case "perform": {
          const edit = args.edit as EditRequest;
          const doc = find();
          if (edit.kind === "removeLayer") {
            doc.layers = doc.layers.filter((l) => l.id !== edit.id);
          }
          return { ...doc, revision: doc.revision + 1 };
        }
      }
      return null;
    },
    { shouldMockEvents: true },
  );
  mockWindows("main");
});

afterEach(async () => {
  // Unmounted first: the app stops listening to the engine's events (some once a promise
  // settles), which must still be mocked then.
  cleanup();
  await new Promise((resolve) => setTimeout(resolve, 0));
  clearMocks();
  vi.unstubAllGlobals();
});

/** The app on `docs`, opened at startup. */
export function open(...docs: DocumentView[]) {
  documents = docs;
  render(App);
  return userEvent.setup();
}

/** The names in the Layers panel, top to bottom. */
export const layerNames = () =>
  [...document.querySelectorAll("li .name")].map((el) => el.textContent?.trim());

/** The Layers panel's row of the layer named `name`. */
export const row = (name: string) =>
  screen.getByText(name, { selector: "li .name" }).closest("li")!;

/** The labels of the open menu's first level, separators as "—". */
export const menuLabels = () =>
  [...document.querySelectorAll(".dropdown:not(.nested) > *")].map((el) =>
    el.classList.contains("separator") ? "—" : el.querySelector(".label")?.textContent,
  );
