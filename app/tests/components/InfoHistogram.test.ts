import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import type { DocumentView } from "../../src/lib/engine";
import Histogram from "../../src/lib/panels/Histogram.svelte";
import Info from "../../src/lib/panels/Info.svelte";
import { PANEL_CONTEXT, type PanelContext } from "../../src/lib/panels/context";
import { reactive } from "./reactive.svelte";

// The Histogram and Info panels, given a context of their own (the app's is tested with it).

afterEach(() => clearMocks());

function doc(changes: Partial<DocumentView> = {}): DocumentView {
  return {
    id: 3,
    width: 400,
    height: 300,
    revision: 1,
    selectionKey: null,
    ...changes,
  } as DocumentView;
}

/** A context whose document and pointer the test changes. */
function context(state: { doc: DocumentView; pointer: [number, number] | null }) {
  return new Map([
    [
      PANEL_CONTEXT,
      {
        get doc() {
          return state.doc;
        },
        get pointer() {
          return state.pointer;
        },
      } as unknown as PanelContext,
    ],
  ]);
}

const counts = (entries: Record<number, number>) =>
  Array.from({ length: 256 }, (_, v) => entries[v] ?? 0);

test("Histogram asks for the counts, again as the document changes, and shows a channel", async () => {
  const calls: unknown[] = [];
  mockIPC((cmd, args) => {
    if (cmd !== "histogram") return null;
    calls.push(args);
    return {
      red: counts({ 0: 10, 200: 10 }),
      green: counts({ 0: 10, 200: 10 }),
      blue: counts({ 0: 10, 200: 10 }),
      luminosity: counts({ 100: 20 }),
      step: 1,
    };
  });
  const state = reactive({ doc: doc(), pointer: null as [number, number] | null });
  render(Histogram, { context: context(state) });
  await vi.waitFor(() =>
    expect(screen.getByText("Mean").nextElementSibling).toHaveTextContent("100"),
  );
  expect(calls).toEqual([{ documentId: 3 }]);
  expect(document.querySelectorAll("path.curve")).toHaveLength(3);
  await userEvent
    .setup()
    .selectOptions(screen.getByRole("combobox", { name: "Channel" }), "luminosity");
  expect(document.querySelectorAll("path.curve.luminosity")).toHaveLength(1);
  expect(screen.getByText("Std Dev").nextElementSibling).toHaveTextContent("0");
  state.doc = doc({ revision: 2 });
  await vi.waitFor(() => expect(calls).toHaveLength(2));
});

test("Histogram asks again once the document rests, not at each step of a drag", async () => {
  const calls: unknown[] = [];
  mockIPC((cmd, args) => {
    if (cmd !== "histogram") return null;
    calls.push(args);
    return {
      red: counts({}),
      green: counts({}),
      blue: counts({}),
      luminosity: counts({}),
      step: 1,
    };
  });
  const state = reactive({ doc: doc(), pointer: null as [number, number] | null });
  render(Histogram, { context: context(state) });
  await vi.waitFor(() => expect(calls).toHaveLength(1));
  // A slider dragged: a new revision every few milliseconds.
  for (let revision = 2; revision <= 8; revision++) {
    state.doc = doc({ revision });
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  expect(calls).toHaveLength(1);
  await vi.waitFor(() => expect(calls).toHaveLength(2));
  await new Promise((resolve) => setTimeout(resolve, 300));
  expect(calls).toHaveLength(2);
  // Another document: at once (sooner than a rest).
  state.doc = doc({ id: 4, revision: 1 });
  await vi.waitFor(
    () => expect(calls).toEqual([{ documentId: 3 }, { documentId: 3 }, { documentId: 4 }]),
    { timeout: 100 },
  );
});

test("Info shows the color and place under the pointer, the selection and the document", async () => {
  const sampled: unknown[] = [];
  mockIPC((cmd, args) => {
    if (cmd === "sample_color") {
      sampled.push(args);
      return [255, 128, 0];
    }
    if (cmd === "selection_bounds") return { left: 10, top: 20, right: 110, bottom: 70 };
    return null;
  });
  const state = reactive({ doc: doc(), pointer: null as [number, number] | null });
  render(Info, { context: context(state) });
  const value = (label: string) => screen.getByText(label, { selector: "dt" }).nextElementSibling;
  expect(value("R")).toHaveTextContent("—");
  expect(value("Document")).toHaveTextContent("400 × 300 px");
  state.pointer = [12.7, 40.2];
  await vi.waitFor(() => expect(value("R")).toHaveTextContent("255"));
  expect(sampled).toEqual([{ documentId: 3, x: 12.5, y: 40.5 }]);
  expect(value("X")).toHaveTextContent("12");
  expect(value("Y")).toHaveTextContent("40");
  expect(value("#")).toHaveTextContent("#ff8000");
  // Off the canvas: nothing.
  state.pointer = [-5, 10];
  await vi.waitFor(() => expect(value("X")).toHaveTextContent("—"));
  expect(value("G")).toHaveTextContent("—");
  expect(value("Selection")).toHaveTextContent("—");
  state.doc = doc({ selectionKey: 4 });
  await vi.waitFor(() => expect(value("Selection")).toHaveTextContent("100 × 50 px"));
  expect(value("Selection")).toHaveTextContent("at 10, 20");
});
