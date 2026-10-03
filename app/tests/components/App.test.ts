import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import App from "../../src/App.svelte";
import type { DocumentView, EditRequest, LayerView } from "../../src/lib/engine";

// The whole UI on a fake engine: a few documents in memory behind the IPC, answering the
// commands these tests go through (anything else answers nothing).

class FixedSizeObserver {
  constructor(private readonly callback: ResizeObserverCallback) {}
  observe() {
    const entry = { contentRect: { width: 200, height: 100 } } as ResizeObserverEntry;
    this.callback([entry], this as unknown as ResizeObserver);
  }
  unobserve() {}
  disconnect() {}
}

function layer(id: number, name: string): LayerView {
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

function documentView(id: number, name: string | null, layers: LayerView[]): DocumentView {
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

let documents: DocumentView[];
/** Every command the UI sent, with its arguments. */
let calls: { cmd: string; args: Record<string, unknown> }[];

const sent = (cmd: string) => calls.filter((c) => c.cmd === cmd).map((c) => c.args);

beforeEach(() => {
  vi.stubGlobal("ResizeObserver", FixedSizeObserver);
  documents = [];
  calls = [];
  mockIPC(
    (cmd, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args });
      const find = () => documents.find((d) => d.id === args.documentId) as DocumentView;
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

function open(...docs: DocumentView[]) {
  documents = docs;
  render(App);
  return userEvent.setup();
}

/** The names in the Layers panel, top to bottom. */
const layerNames = () =>
  [...document.querySelectorAll("li .name")].map((el) => el.textContent?.trim());

test("it starts on the welcome page, with the menus", async () => {
  open();
  expect(await screen.findByText("Open an image or create a document")).toBeInTheDocument();
  expect(screen.getByRole("menuitem", { name: "File" })).toBeInTheDocument();
});

test("documents open at startup are tabs; the last one shows its layers and is drawn", async () => {
  open(
    documentView(1, "cat.jpg", [layer(1, "Cat")]),
    documentView(2, "dog.png", [layer(1, "Background"), layer(2, "Dog")]),
  );
  expect(await screen.findByText("cat.jpg")).toBeInTheDocument();
  expect(screen.getByText("dog.png")).toBeInTheDocument();
  await vi.waitFor(() => expect(layerNames()).toEqual(["Dog", "Background"]));
  await vi.waitFor(() =>
    expect(sent("render_view")).toContainEqual(expect.objectContaining({ documentId: 2 })),
  );
});

test("File > New creates an untitled document in a new tab", async () => {
  const user = open();
  await user.click(await screen.findByRole("button", { name: "New document…" }));
  const dialog = screen.getByRole("dialog", { name: "New" });
  await user.selectOptions(within(dialog).getByRole("combobox", { name: "Preset" }), "hd");
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(sent("new_document")).toHaveLength(1));
  expect(sent("new_document")[0]).toMatchObject({ name: null, width: 1920, height: 1080 });
  await vi.waitFor(() => expect(layerNames()).toHaveLength(1));
});

test("Ctrl+Z undoes in the active document", async () => {
  const user = open(documentView(4, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("{Control>}z{/Control}");
  await vi.waitFor(() => expect(sent("undo")).toEqual([{ documentId: 4 }]));
});

test("a tool's letter picks it in the toolbar", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  await screen.findByText("cat.jpg");
  await user.keyboard("b");
  expect(screen.getByRole("button", { name: "Brush Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("Delete removes the selected layer, without a selection of pixels", async () => {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Background"), layer(2, "Cat")]));
  await vi.waitFor(() => expect(layerNames()).toEqual(["Cat", "Background"]));
  await user.keyboard("{Delete}");
  await vi.waitFor(() =>
    expect(sent("perform")).toContainEqual({
      documentId: 1,
      edit: { kind: "removeLayer", id: 2 },
    }),
  );
  await vi.waitFor(() => expect(layerNames()).toEqual(["Background"]));
});

test("the panels open at the width saved last, and their left edge resizes them", async () => {
  localStorage.setItem("slopshop.panelWidth", "420");
  open(documentView(1, "cat.jpg", [layer(1, "Cat")]));
  const edge = await screen.findByRole("separator", { name: "Resize the panels" });
  const main = document.querySelector("main") as HTMLElement;
  expect(main.style.getPropertyValue("--panel-width")).toBe("420px");
  await fireEvent.pointerDown(edge, { pointerId: 1, button: 0, clientX: 600 });
  await fireEvent.pointerMove(edge, { pointerId: 1, clientX: 550 });
  await fireEvent.pointerUp(edge, { pointerId: 1, clientX: 550 });
  expect(main.style.getPropertyValue("--panel-width")).toBe("470px");
  expect(localStorage.getItem("slopshop.panelWidth")).toBe("470");
  localStorage.clear();
});
