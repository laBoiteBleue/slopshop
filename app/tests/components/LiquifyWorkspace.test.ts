// Filter > Liquify's workspace (ADR 0037) on a fake engine: what the tools, the keys, the brush's
// settings, the pointer and the buttons send, and what OK and Cancel do.
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { LiquifyState, LiquifyStrokePiece, LiquifyViewRequest } from "../../src/lib/engine";
import LiquifyWorkspace from "../../src/lib/LiquifyWorkspace.svelte";
import { fitView, toLayer } from "../../src/lib/liquify";

/** The layer: 1000 × 500 pixels, in a stage of 400 × 300. */
const LAYER = { width: 1000, height: 500 };
const STAGE = { width: 400, height: 300 };
const FIT = fitView(LAYER, STAGE);

let calls: { cmd: string; args: Record<string, unknown> }[] = [];
let opened: LiquifyState;
let openFails = false;

const state = (over: Partial<LiquifyState> = {}): LiquifyState => ({
  ...LAYER,
  canUndo: false,
  canRedo: false,
  changed: false,
  displaced: false,
  ...over,
});

beforeEach(() => {
  calls = [];
  opened = state();
  openFails = false;
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockReturnValue({
    ...STAGE,
    left: 0,
    top: 0,
    right: STAGE.width,
    bottom: STAGE.height,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  });
  mockIPC((cmd, payload) => {
    const args = (payload ?? {}) as Record<string, unknown>;
    calls.push({ cmd, args });
    switch (cmd) {
      case "liquify_open":
        if (openFails) throw new Error("no");
        return opened;
      case "liquify_stroke":
        return state({ changed: true, canUndo: true, displaced: true });
      case "liquify_undo":
        return state({
          canUndo: !!args.redo,
          canRedo: !args.redo,
          changed: !!args.redo,
          displaced: !!args.redo,
        });
      case "liquify_restore_all":
        return state({ canUndo: true, changed: true });
      case "liquify_frame": {
        const view = args.view as LiquifyViewRequest;
        return new ArrayBuffer(view.width * view.height * 4);
      }
    }
    return null;
  });
});

afterEach(() => {
  vi.restoreAllMocks();
  clearMocks();
});

const sent = (cmd: string) => calls.filter((c) => c.cmd === cmd).map((c) => c.args);
const pieces = () => sent("liquify_stroke").map((a) => a.request as LiquifyStrokePiece);
const frames = () => sent("liquify_frame").map((a) => a.view as LiquifyViewRequest);
const stage = () => screen.getByRole("application", { name: "Preview" });

async function workspace(index: number | null = null) {
  const onok = vi.fn();
  const oncancel = vi.fn();
  render(LiquifyWorkspace, { documentId: 7, layerId: 3, index, onok, oncancel });
  const user = userEvent.setup();
  // The session is open and the first frame drawn.
  await vi.waitFor(() => expect(frames().length).toBeGreaterThan(0));
  await vi.waitFor(() => expect(screen.getByRole("button", { name: "OK" })).toBeEnabled());
  return { user, onok, oncancel };
}

const ok = () => screen.getByRole("button", { name: "OK" });

/** The number field of the brush's setting `name`. */
const setting = (name: string) =>
  screen
    .getByText(name, { selector: ".label" })
    .closest(".field")!
    .querySelector("input[type=number]") as HTMLInputElement;

/** A stroke: press at `from`, move to each of `to`, release (CSS pixels of the stage). */
async function stroke(
  user: ReturnType<typeof userEvent.setup>,
  from: [number, number],
  ...to: [number, number][]
) {
  await user.pointer([
    {
      keys: "[MouseLeft>]",
      target: stage(),
      coords: { clientX: from[0], clientY: from[1] },
    },
    ...to.map(([x, y]) => ({ target: stage(), coords: { clientX: x, clientY: y } })),
    { keys: "[/MouseLeft]", target: stage() },
  ]);
  await vi.waitFor(() => expect(pieces().at(-1)?.end).toBe(true));
}

test("it opens its session on the layer (or the entry) and draws the layer fitted to the stage", async () => {
  await workspace();
  expect(sent("liquify_open")).toEqual([{ documentId: 7, layerId: 3, index: null }]);
  // The first frame: the stage's size, the whole layer in it, the frozen area tinted.
  expect(frames()[0]).toMatchObject({ width: STAGE.width, height: STAGE.height, overlay: true });
  await vi.waitFor(() =>
    expect(frames().at(-1)).toMatchObject({ zoom: FIT.zoom, x: FIT.x, y: FIT.y }),
  );
  expect(screen.getByRole("dialog", { name: "Liquify" })).toBeInTheDocument();
});

test("it can open on a Liquify entry to edit it again", async () => {
  await workspace(2);
  expect(sent("liquify_open")).toEqual([{ documentId: 7, layerId: 3, index: 2 }]);
});

test("a session that could not open says so and cannot be accepted", async () => {
  openFails = true;
  render(LiquifyWorkspace, {
    documentId: 7,
    layerId: 3,
    index: null,
    onok: vi.fn(),
    oncancel: vi.fn(),
  });
  expect(await screen.findByText("Liquify could not open on this layer.")).toBeInTheDocument();
  expect(ok()).toBeDisabled();
  expect(sent("liquify_frame")).toHaveLength(0);
});

test("the toolbar has the nine tools, and no Hand nor Zoom tool", async () => {
  await workspace();
  const toolbar = screen.getByRole("toolbar", { name: "Liquify tools" });
  const names = [...toolbar.querySelectorAll("button")].map((b) => b.getAttribute("aria-label"));
  expect(names).toEqual([
    "Forward Warp",
    "Reconstruct",
    "Smooth",
    "Twirl Clockwise",
    "Pucker",
    "Bloat",
    "Push Left",
    "Freeze Mask",
    "Thaw Mask",
  ]);
  expect(screen.getByRole("button", { name: "Forward Warp" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(screen.getByRole("button", { name: "Forward Warp" })).toHaveAttribute(
    "title",
    "Forward Warp (W)",
  );
});

test("the keys choose the tools, whatever the case", async () => {
  const { user } = await workspace();
  const keys: [string, string][] = [
    ["r", "Reconstruct"],
    ["e", "Smooth"],
    ["c", "Twirl Clockwise"],
    ["s", "Pucker"],
    ["b", "Bloat"],
    ["o", "Push Left"],
    ["f", "Freeze Mask"],
    ["d", "Thaw Mask"],
    ["W", "Forward Warp"],
  ];
  for (const [key, name] of keys) {
    await user.keyboard(key);
    expect(screen.getByRole("button", { name })).toHaveAttribute("aria-pressed", "true");
    expect(screen.getAllByRole("button", { pressed: true })).toHaveLength(1);
  }
});

test("a stroke sends the tool, the brush and the layer points the pointer went through", async () => {
  const { user } = await workspace();
  await user.keyboard("r");
  await stroke(user, [100, 80], [150, 90], [200, 100]);
  const sentPieces = pieces();
  expect(sentPieces[0]).toMatchObject({
    tool: "reconstruct",
    brush: { size: 100, density: 50, pressure: 100, rate: 80 },
    begin: true,
    end: false,
  });
  const [x0, y0] = toLayer(FIT, 100, 80);
  expect(sentPieces[0].points[0][0]).toBeCloseTo(x0);
  expect(sentPieces[0].points[0][1]).toBeCloseTo(y0);
  // Every sample reached the engine, in order, and the last piece ends the stroke.
  const points = sentPieces.flatMap((p) => p.points);
  expect(points).toHaveLength(3);
  const [x2, y2] = toLayer(FIT, 200, 100);
  expect(points[2][0]).toBeCloseTo(x2);
  expect(points[2][1]).toBeCloseTo(y2);
  expect(sentPieces.filter((p) => p.begin)).toHaveLength(1);
  expect(sentPieces.at(-1)?.end).toBe(true);
  // The frame follows the stroke.
  const before = frames().length;
  await vi.waitFor(() => expect(frames().length).toBeGreaterThan(before - 1));
});

test("Alt turns the twirl the other way and a pucker into a bloat, as long as it is held", async () => {
  const { user } = await workspace();
  await user.keyboard("c");
  await user.keyboard("{Alt>}");
  expect(screen.getByRole("button", { name: "Twirl Clockwise" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await stroke(user, [100, 100], [120, 100]);
  expect(pieces().find((p) => p.begin)?.tool).toBe("twirlCounterclockwise");
  await user.keyboard("{/Alt}");
  await stroke(user, [100, 100], [120, 100]);
  expect(
    pieces()
      .filter((p) => p.begin)
      .at(-1)?.tool,
  ).toBe("twirlClockwise");
  await user.keyboard("s{Alt>}");
  // The toolbar shows the tool at work: the bloat while Alt is held on the pucker.
  expect(screen.getByRole("button", { name: "Bloat" })).toHaveAttribute("aria-pressed", "true");
  await stroke(user, [100, 100], [120, 100]);
  expect(
    pieces()
      .filter((p) => p.begin)
      .at(-1)?.tool,
  ).toBe("bloat");
  await user.keyboard("{/Alt}f{Alt>}");
  await stroke(user, [100, 100], [100, 100]);
  expect(
    pieces()
      .filter((p) => p.begin)
      .at(-1)?.tool,
  ).toBe("thaw");
});

test("[ and ] change the brush's size as in Photoshop, and the field agrees", async () => {
  const { user } = await workspace();
  const size = () => setting("Size");
  expect(size()).toHaveValue(100);
  await user.keyboard("]");
  expect(size()).toHaveValue(110);
  // "[[" types one "[" in user-event's syntax.
  await user.keyboard("[[[[");
  expect(size()).toHaveValue(89);
  await stroke(user, [100, 100], [110, 100]);
  expect(pieces()[0].brush.size).toBe(89);
});

test("the brush's settings go with the stroke", async () => {
  const { user } = await workspace();
  for (const [name, value] of [
    ["Size", "250"],
    ["Density", "20"],
    ["Pressure", "60"],
    ["Rate", "35"],
  ]) {
    const field = setting(name);
    await user.clear(field);
    await user.type(field, value);
    await user.tab();
  }
  await stroke(user, [100, 100], [130, 100]);
  expect(pieces()[0].brush).toEqual({ size: 250, density: 20, pressure: 60, rate: 35 });
});

test("a tool that acts while held does, on a timer, with the time the pointer stayed still", async () => {
  const { user } = await workspace();
  await user.keyboard("s");
  await user.pointer({
    keys: "[MouseLeft>]",
    target: stage(),
    coords: { clientX: 200, clientY: 150 },
  });
  await vi.waitFor(() => expect(pieces().some((p) => p.hold > 0)).toBe(true), { timeout: 2000 });
  const held = pieces().reduce((sum, p) => sum + p.hold, 0);
  expect(held).toBeGreaterThan(0.03);
  expect(held).toBeLessThan(2);
  await user.pointer({ keys: "[/MouseLeft]", target: stage() });
  await vi.waitFor(() => expect(pieces().at(-1)?.end).toBe(true));
  // Once released, no more time passes.
  const count = pieces().length;
  await new Promise((resolve) => setTimeout(resolve, 120));
  expect(pieces()).toHaveLength(count);
});

test("Restore All is for when something is displaced, and sends the engine its command", async () => {
  const { user } = await workspace();
  const restore = () => screen.getByRole("button", { name: "Restore All" });
  expect(restore()).toBeDisabled();
  await stroke(user, [100, 100], [160, 110]);
  await vi.waitFor(() => expect(restore()).toBeEnabled());
  await user.click(restore());
  expect(sent("liquify_restore_all")).toEqual([{ documentId: 7 }]);
  await vi.waitFor(() => expect(restore()).toBeDisabled());
});

test("Ctrl+Z undoes a stroke, Shift+Ctrl+Z and Ctrl+Y bring it back; so do the buttons", async () => {
  const { user } = await workspace();
  const undo = () => screen.getByRole("button", { name: "Undo Stroke" });
  const redo = () => screen.getByRole("button", { name: "Redo Stroke" });
  expect(undo()).toBeDisabled();
  expect(redo()).toBeDisabled();
  await stroke(user, [100, 100], [160, 110]);
  await vi.waitFor(() => expect(undo()).toBeEnabled());
  await user.keyboard("{Control>}z{/Control}");
  expect(sent("liquify_undo")).toEqual([{ documentId: 7, redo: false }]);
  await vi.waitFor(() => expect(redo()).toBeEnabled());
  await user.keyboard("{Control>}{Shift>}z{/Shift}{/Control}");
  expect(sent("liquify_undo").at(-1)).toEqual({ documentId: 7, redo: true });
  await user.keyboard("{Control>}y{/Control}");
  expect(sent("liquify_undo").at(-1)).toEqual({ documentId: 7, redo: true });
  await vi.waitFor(() => expect(undo()).toBeEnabled());
  await user.click(undo());
  expect(sent("liquify_undo").at(-1)).toEqual({ documentId: 7, redo: false });
  await vi.waitFor(() => expect(redo()).toBeEnabled());
  await user.click(redo());
  expect(sent("liquify_undo").at(-1)).toEqual({ documentId: 7, redo: true });
  // The frame follows each change.
  const before = frames().length;
  await user.keyboard("{Control>}z{/Control}");
  await vi.waitFor(() => expect(frames().length).toBeGreaterThan(before));
});

test("OK accepts once the strokes are in; Enter does the same", async () => {
  const { user, onok, oncancel } = await workspace();
  await stroke(user, [100, 100], [160, 110]);
  await user.click(ok());
  await vi.waitFor(() => expect(onok).toHaveBeenCalledTimes(1));
  expect(oncancel).not.toHaveBeenCalled();
  // The workspace itself sends nothing to commit: the app does, from OK.
  expect(sent("liquify_commit")).toHaveLength(0);
  await user.keyboard("{Enter}");
  await vi.waitFor(() => expect(onok).toHaveBeenCalledTimes(2));
});

test("Cancel and Escape give up: nothing is accepted", async () => {
  const { user, onok, oncancel } = await workspace();
  await stroke(user, [100, 100], [160, 110]);
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(oncancel).toHaveBeenCalledTimes(1);
  await fireEvent(screen.getByRole("dialog"), new Event("cancel", { cancelable: true }));
  expect(oncancel).toHaveBeenCalledTimes(2);
  expect(onok).not.toHaveBeenCalled();
});

test("the wheel zooms about the pointer and the middle button pans, as in the rest of the app", async () => {
  const { user } = await workspace();
  const start = frames().at(-1)!;
  await fireEvent.wheel(stage(), { deltaY: -100, clientX: 200, clientY: 150 });
  await vi.waitFor(() => expect(frames().at(-1)!.zoom).toBeGreaterThan(start.zoom));
  const zoomed = frames().at(-1)!;
  // The layer point under the pointer stays under it.
  const under = (v: LiquifyViewRequest) => [v.x + 200 / v.zoom, v.y + 150 / v.zoom];
  expect(under(zoomed)[0]).toBeCloseTo(under(start)[0], 3);
  expect(under(zoomed)[1]).toBeCloseTo(under(start)[1], 3);
  await fireEvent.wheel(stage(), { deltaY: 100, clientX: 200, clientY: 150 });
  await vi.waitFor(() => expect(frames().at(-1)!.zoom).toBeLessThan(zoomed.zoom));

  // A middle-button drag moves the view, and draws nothing.
  const before = frames().at(-1)!;
  await user.pointer([
    { keys: "[MouseMiddle>]", target: stage(), coords: { clientX: 100, clientY: 100 } },
    { target: stage(), coords: { clientX: 140, clientY: 120 } },
    { keys: "[/MouseMiddle]", target: stage() },
  ]);
  await vi.waitFor(() => expect(frames().at(-1)!.x).toBeLessThan(before.x));
  expect(frames().at(-1)!.y).toBeLessThan(before.y);
  expect(pieces()).toHaveLength(0);
  expect(screen.getByLabelText("Zoom")).toHaveTextContent(/%$/);
});

test("Space and the main button pan too, without a stroke", async () => {
  const { user } = await workspace();
  const before = frames().at(-1)!;
  await user.keyboard("{ >}");
  await user.pointer([
    { keys: "[MouseLeft>]", target: stage(), coords: { clientX: 100, clientY: 100 } },
    { target: stage(), coords: { clientX: 130, clientY: 100 } },
    { keys: "[/MouseLeft]", target: stage() },
  ]);
  await vi.waitFor(() => expect(frames().at(-1)!.x).toBeLessThan(before.x));
  expect(pieces()).toHaveLength(0);
});

test("the frozen area's tint can be turned off", async () => {
  const { user } = await workspace();
  expect(frames().at(-1)!.overlay).toBe(true);
  await user.click(screen.getByRole("checkbox", { name: "Show Freeze Mask" }));
  await vi.waitFor(() => expect(frames().at(-1)!.overlay).toBe(false));
});

test("the keys stay in the workspace: the app behind does not hear them", async () => {
  const { user } = await workspace();
  const heard = vi.fn();
  document.addEventListener("keydown", heard);
  await user.keyboard("wfd{Control>}z{/Control}");
  document.removeEventListener("keydown", heard);
  expect(heard).not.toHaveBeenCalled();
});

test("typing in a field is not a shortcut", async () => {
  const { user } = await workspace();
  const density = setting("Density");
  await user.click(density);
  await user.keyboard("r");
  // The tool stays Forward Warp: the letter went to the field.
  expect(screen.getByRole("button", { name: "Forward Warp" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});
