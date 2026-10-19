// Color Lookup (Image > Adjustments and Layer > New Adjustment Layer): a `.cube` or `.3dl`
// table loaded through the file dialog, shown live, applied with OK.
import { screen, within } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import {
  ADJUSTMENT_PARAMS,
  type EditRequest,
  type LayerView,
  type LutView,
} from "../../../src/lib/engine";
import { documentView, layer, menuLabels, open, respond, sent } from "./harness";

const night: LutView = { id: 42, name: "night.cube", dimension: 3, size: 33 };
/** The table the preview layer shows (what the last live edit set). */
let shown: LutView | null = null;
/** Whether the engine refuses the file. */
let refuse = false;

/** Image > Adjustments' preview: an adjustment layer clipped above the photo. */
const preview = (): LayerView => ({
  ...layer(9, "colorLookup"),
  kind: "adjustment",
  clipped: true,
  adjustment: {
    id: "colorLookup",
    values: Array<number>(ADJUSTMENT_PARAMS).fill(0),
    curves: null,
    curveSamples: null,
    gradient: null,
    lut: shown,
  },
});

respond("perform_live", (args, doc) => {
  const edit = args.edit as EditRequest;
  if (edit.kind === "batch") {
    const set = edit.edits.find((e) => e.kind === "setAdjustment");
    shown = set && "lut" in set && set.lut === night.id ? night : null;
  }
  doc.layers = [...doc.layers.filter((l) => l.id !== 9), preview()];
  return { ...doc, revision: doc.revision + 1 };
});
respond("replace_gesture", (_, doc) => ({ ...doc, revision: doc.revision + 1 }));
respond("plugin:dialog|open", () => "C:/luts/night.cube");
respond("load_color_lookup", () => {
  if (refuse) throw "invalid lookup table, line 3: 2 numbers expected";
  return night;
});

async function openDialog() {
  shown = null;
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Image" }));
  await user.hover(screen.getByText("Adjustments", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Color Lookup…" }));
  const dialog = within(await screen.findByRole("dialog", { name: "Color Lookup" }));
  return { user, dialog };
}

test("Image > Adjustments > Color Lookup: Load 3D LUT reads the file chosen, shown live, OK applies it", async () => {
  refuse = false;
  const { user, dialog } = await openDialog();
  expect(dialog.getByText("None")).toBeInTheDocument();
  await user.click(dialog.getByRole("button", { name: "Load 3D LUT…" }));
  // The file dialog offers lookup tables only; the engine reads the one chosen.
  const options = sent("plugin:dialog|open").at(-1)?.options as {
    filters: { extensions: string[] }[];
  };
  expect(options.filters[0].extensions).toEqual(["cube", "3dl"]);
  await vi.waitFor(() =>
    expect(sent("load_color_lookup").at(-1)).toEqual({ path: "C:/luts/night.cube" }),
  );
  await vi.waitFor(() => expect(dialog.getByText("night.cube")).toBeInTheDocument());
  const live = sent("perform_live").at(-1)?.edit as { edits: EditRequest[] };
  expect(live.edits).toContainEqual(
    expect.objectContaining({ kind: "setAdjustment", id: 9, adjustment: "colorLookup", lut: 42 }),
  );
  await user.click(dialog.getByRole("button", { name: "OK" }));
  await vi.waitFor(() =>
    expect(sent("replace_gesture").at(-1)?.edit).toEqual(
      expect.objectContaining({
        kind: "applyEffect",
        ids: [1],
        adjustment: "colorLookup",
        lut: 42,
      }),
    ),
  );
});

test("a lookup table the engine refuses is said, and nothing changes", async () => {
  refuse = true;
  const { user, dialog } = await openDialog();
  const before = sent("perform_live").length;
  await user.click(dialog.getByRole("button", { name: "Load 3D LUT…" }));
  expect(await screen.findByText(/2 numbers expected/)).toBeInTheDocument();
  expect(sent("perform_live")).toHaveLength(before);
  expect(dialog.getByText("None")).toBeInTheDocument();
});

test("Color Lookup is in Photoshop's place in both menus, after Channel Mixer", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Layer" }));
  await user.hover(screen.getByText("New Adjustment Layer", { selector: ".label" }));
  const items = [...document.querySelectorAll(".dropdown.nested .label")].map((el) =>
    el.textContent?.trim(),
  );
  const at = items.indexOf("Color Lookup…");
  expect(at).toBeGreaterThan(0);
  expect(items[at - 1]).toBe("Channel Mixer…");
  expect(menuLabels()).toContain("New Adjustment Layer");
});
