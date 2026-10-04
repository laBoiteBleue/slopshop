import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import ColorRangeDialog, {
  colorRangeRequest,
  type ColorRangeState,
} from "../../src/lib/ColorRangeDialog.svelte";
import { reactive } from "./reactive.svelte";
import { eyedropperCursor } from "../../src/lib/eyedropper";

/** The previews the engine was asked for. */
let previews: Record<string, unknown>[];

beforeEach(() => {
  previews = [];
  mockIPC((cmd, args) => {
    if (cmd !== "color_range_preview") return;
    previews.push(args as Record<string, unknown>);
    // A 2 × 2 preview: its size, then one byte of coverage per pixel.
    const buffer = new ArrayBuffer(8 + 4);
    const view = new DataView(buffer);
    view.setUint32(0, 2, true);
    view.setUint32(4, 2, true);
    return buffer;
  });
});
afterEach(() => clearMocks());

/** Select > Color Range on a 1000 × 500 px document, as the app starts it. */
function open(layerId: number | null = 5) {
  const range = reactive<ColorRangeState>({
    document: 3,
    included: [],
    excluded: [],
    fuzziness: 40,
    invert: false,
    eyedropper: "pick",
    localized: false,
    radius: 250,
    sampleAll: false,
    layerId,
  });
  const onapply = vi.fn();
  const onclose = vi.fn();
  render(ColorRangeDialog, { range, width: 1000, height: 500, onapply, onclose });
  // The preview shows the whole document in a 220 × 110 box (nothing is laid out in jsdom).
  const preview = document.querySelector("canvas") as HTMLCanvasElement;
  preview.getBoundingClientRect = () => ({ left: 0, top: 0, width: 220, height: 110 }) as DOMRect;
  return { range, onapply, onclose, preview, user: userEvent.setup() };
}

const lastRequest = () => (previews.at(-1)?.request ?? null) as Record<string, unknown> | null;
const ok = () => screen.getByRole("button", { name: "OK" });

/** A click on the preview at the document's point (500, 250): its middle. */
const clickPreview = (user: ReturnType<typeof userEvent.setup>, preview: HTMLElement) =>
  user.pointer({ keys: "[MouseLeft]", target: preview, coords: { clientX: 110, clientY: 55 } });

test("the engine draws the preview of what the settings would select", async () => {
  open();
  await vi.waitFor(() => expect(previews).toHaveLength(1));
  expect(previews[0]).toMatchObject({
    documentId: 3,
    maxSide: 220,
    request: {
      included: [],
      excluded: [],
      fuzziness: 40,
      invert: false,
      localized: null,
      layerId: 5,
    },
  });
});

test("a click on the preview samples the color there, and the preview follows", async () => {
  const { range, preview, user } = open();
  await clickPreview(user, preview);
  expect(range.included).toEqual([[500, 250]]);
  await vi.waitFor(() => expect(lastRequest()).toMatchObject({ included: [[500, 250]] }));
  // The next plain click replaces the sample.
  await user.pointer({
    keys: "[MouseLeft]",
    target: preview,
    coords: { clientX: 22, clientY: 11 },
  });
  expect(range.included).toEqual([[100, 50]]);
});

test("Shift adds a color to the sample, Alt takes one away", async () => {
  const { range, preview, user } = open();
  await clickPreview(user, preview);
  await user.keyboard("{Shift>}");
  await user.pointer({
    keys: "[MouseLeft]",
    target: preview,
    coords: { clientX: 22, clientY: 11 },
  });
  await user.keyboard("{/Shift}{Alt>}");
  await user.pointer({
    keys: "[MouseLeft]",
    target: preview,
    coords: { clientX: 219, clientY: 55 },
  });
  await user.keyboard("{/Alt}");
  expect(range.included).toEqual([
    [500, 250],
    [100, 50],
  ]);
  expect(range.excluded).toEqual([[995, 250]]);
  await vi.waitFor(() => expect(lastRequest()).toMatchObject({ excluded: [[995, 250]] }));
});

test("the eyedroppers choose what a click does", async () => {
  const { range, preview, user } = open();
  expect(screen.getByRole("radio", { name: "Sample a color" })).toBeChecked();
  await clickPreview(user, preview);
  await user.click(screen.getByRole("radio", { name: "Subtract from sample (Alt)" }));
  await user.pointer({
    keys: "[MouseLeft]",
    target: preview,
    coords: { clientX: 22, clientY: 11 },
  });
  expect(range.excluded).toEqual([[100, 50]]);
  await user.click(screen.getByRole("radio", { name: "Add to sample (Shift)" }));
  await user.pointer({
    keys: "[MouseLeft]",
    target: preview,
    coords: { clientX: 219, clientY: 55 },
  });
  expect(range.included).toEqual([
    [500, 250],
    [995, 250],
  ]);
});

test("Fuzziness and Invert change the preview", async () => {
  const { range, user } = open();
  await vi.waitFor(() => expect(previews).toHaveLength(1));
  const fuzziness = screen.getByRole("spinbutton");
  await user.clear(fuzziness);
  await user.type(fuzziness, "120");
  await user.click(screen.getByRole("checkbox", { name: "Invert" }));
  expect(range.fuzziness).toBe(120);
  await vi.waitFor(() => expect(lastRequest()).toMatchObject({ fuzziness: 120, invert: true }));
});

test("OK needs a sample, or Invert (everything but nothing), and Enter applies as OK does", async () => {
  const { onapply, preview, user } = open();
  expect(ok()).toBeDisabled();
  await user.click(screen.getByRole("checkbox", { name: "Invert" }));
  expect(ok()).toBeEnabled();
  await user.click(screen.getByRole("checkbox", { name: "Invert" }));
  expect(ok()).toBeDisabled();
  await clickPreview(user, preview);
  await user.click(ok());
  expect(onapply).toHaveBeenCalledOnce();
  await user.keyboard("{Enter}");
  expect(onapply).toHaveBeenCalledTimes(2);
});

test("Escape and Cancel close it, but Enter in a number field is left to the field", async () => {
  const { onapply, onclose, user } = open();
  await user.type(screen.getByRole("spinbutton"), "{Enter}");
  expect(onapply).not.toHaveBeenCalled();
  expect(onclose).not.toHaveBeenCalled();
  // Regression: Escape did nothing while the field had the focus.
  await user.keyboard("{Escape}");
  expect(onclose).toHaveBeenCalledOnce();
  await user.tab();
  await user.keyboard("{Escape}");
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onclose).toHaveBeenCalledTimes(3);
});

test("Localized keeps the colors near their samples, within the radius shown then", async () => {
  const { range, user } = open();
  expect(screen.queryByText("Range:")).not.toBeInTheDocument();
  await user.click(screen.getByRole("checkbox", { name: "Localized" }));
  expect(screen.getByText("Range:")).toBeInTheDocument();
  await vi.waitFor(() => expect(lastRequest()).toMatchObject({ localized: 250 }));
  const radius = screen.getAllByRole("spinbutton").at(-1)!;
  await user.clear(radius);
  await user.type(radius, "80{Tab}");
  expect(range.radius).toBe(80);
  await vi.waitFor(() => expect(lastRequest()).toMatchObject({ localized: 80 }));
  await user.click(screen.getByRole("checkbox", { name: "Localized" }));
  await vi.waitFor(() => expect(lastRequest()).toMatchObject({ localized: null }));
});

test("the active layer is sampled, or the image as displayed with Sample All Layers", async () => {
  const { user } = open();
  await vi.waitFor(() => expect(lastRequest()).toMatchObject({ layerId: 5 }));
  await user.click(screen.getByRole("checkbox", { name: "Sample All Layers" }));
  await vi.waitFor(() => expect(lastRequest()).toMatchObject({ layerId: null }));
});

test("without an active layer, the image as displayed is sampled", async () => {
  open(null);
  expect(screen.getByRole("checkbox", { name: "Sample All Layers" })).toBeDisabled();
  await vi.waitFor(() => expect(lastRequest()).toMatchObject({ layerId: null }));
});

test("colorRangeRequest sends what the dialog shows", () => {
  const range: ColorRangeState = {
    document: 1,
    included: [[1, 2]],
    excluded: [[3, 4]],
    fuzziness: 12,
    invert: true,
    eyedropper: "add",
    localized: true,
    radius: 30,
    sampleAll: true,
    layerId: 9,
  };
  expect(colorRangeRequest(range)).toEqual({
    included: [[1, 2]],
    excluded: [[3, 4]],
    fuzziness: 12,
    invert: true,
    localized: 30,
    layerId: null,
  });
});

test("on the preview the pointer is the eyedropper a click would use", async () => {
  const { range, preview, user } = open();
  expect(preview.style.cursor).toBe(eyedropperCursor("pick"));
  await user.click(screen.getByRole("radio", { name: "Subtract from sample (Alt)" }));
  expect(range.eyedropper).toBe("subtract");
  expect(preview.style.cursor).toBe(eyedropperCursor("subtract"));
  await user.keyboard("{Shift>}");
  expect(preview.style.cursor).toBe(eyedropperCursor("add"));
  await user.keyboard("{/Shift}");
  expect(preview.style.cursor).toBe(eyedropperCursor("subtract"));
});
