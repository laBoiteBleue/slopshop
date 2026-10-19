import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import type { ContourPreset, LayerStyle, StyleContour } from "../../src/lib/engine";
import { CONTOUR_PRESET_IDS, forgetContourPresets } from "../../src/lib/styleContours";
import LayerStyleDialog, { type StylePage } from "../../src/lib/LayerStyleDialog.svelte";
import { withEffect, withPatternOverlay } from "../../src/lib/layerStyle";
import { forgetDialogPlaces } from "../../src/lib/dialogDrag";

function open(
  style: LayerStyle | null,
  page: StylePage = "blending",
  globalLight: [number, number] = [120, 30],
) {
  const props = {
    documentId: 1,
    style,
    globalLight,
    page,
    onchange: vi.fn(),
    onpage: vi.fn(),
    onpickcolor: vi.fn(),
    onpickpattern: vi.fn(),
    onok: vi.fn(),
    oncancel: vi.fn(),
  };
  render(LayerStyleDialog, props);
  return { ...props, user: userEvent.setup() };
}

/** The style of the last change sent. */
const last = (fn: ReturnType<typeof vi.fn>) => fn.mock.lastCall?.[0] as LayerStyle;
/** The global light of the last change sent. */
const lastLight = (fn: ReturnType<typeof vi.fn>) => fn.mock.lastCall?.[1] as [number, number];

test("a shadow added takes the global light; its angle moved moves it for every effect using it", async () => {
  const style = withEffect(null, "innerShadow", true);
  const { onchange, user } = open(
    { ...style, innerShadow: { ...style.innerShadow!, angle: 90 } },
    "dropShadow",
    [90, 30],
  );
  await user.click(screen.getByRole("checkbox", { name: "Drop Shadow" }));
  expect(last(onchange).dropShadow).toMatchObject({ angle: 90, useGlobalLight: true });
  expect(lastLight(onchange)).toEqual([90, 30]);
  const settings = screen.getByRole("region", { name: "Drop Shadow" });
  expect(within(settings).getByRole("checkbox", { name: "Use Global Light" })).toBeChecked();
  const angle = within(settings).getAllByRole("spinbutton")[1];
  await user.clear(angle);
  await user.type(angle, "45");
  await user.tab();
  // Sent once, both shadows on the new light.
  expect(last(onchange).dropShadow?.angle).toBe(45);
  expect(last(onchange).innerShadow?.angle).toBe(45);
  expect(lastLight(onchange)).toEqual([45, 30]);
});

test("a shadow on its own light leaves the global light; turned back on, it takes it", async () => {
  const { onchange, user } = open(withEffect(null, "dropShadow", true), "dropShadow", [120, 30]);
  const settings = screen.getByRole("region", { name: "Drop Shadow" });
  await user.click(within(settings).getByRole("checkbox", { name: "Use Global Light" }));
  const angle = within(settings).getAllByRole("spinbutton")[1];
  await user.clear(angle);
  await user.type(angle, "10");
  await user.tab();
  expect(last(onchange).dropShadow).toMatchObject({ angle: 10, useGlobalLight: false });
  expect(lastLight(onchange)).toEqual([120, 30]);
  await user.click(within(settings).getByRole("checkbox", { name: "Use Global Light" }));
  expect(last(onchange).dropShadow).toMatchObject({ angle: 120, useGlobalLight: true });
  expect(lastLight(onchange)).toEqual([120, 30]);
});

test("Bevel and Emboss's Use Global Light takes the light's angle and altitude", async () => {
  const bevel = withEffect(null, "bevel", true);
  const { onchange, user } = open(
    { ...bevel, bevel: { ...bevel.bevel!, useGlobalLight: false, angle: 0, altitude: 80 } },
    "bevel",
    [30, 50],
  );
  const settings = screen.getByRole("region", { name: "Bevel & Emboss" });
  await user.click(within(settings).getByRole("checkbox", { name: "Use Global Light" }));
  expect(last(onchange).bevel).toMatchObject({ angle: 30, altitude: 50, useGlobalLight: true });
  expect(lastLight(onchange)).toEqual([30, 50]);
});

test("an effect's checkbox turns it on at its defaults and shows its settings", async () => {
  const { onchange, onpage, user } = open(null);
  await user.click(screen.getByRole("checkbox", { name: "Drop Shadow" }));
  expect(last(onchange).dropShadow).toMatchObject({ enabled: true, mode: "multiply", size: 5 });
  expect(onpage).toHaveBeenCalledWith("dropShadow");
});

test("a setting sent as it changes; OK and Cancel are the app's", async () => {
  const { onchange, onok, oncancel, user } = open(withEffect(null, "stroke", true), "stroke");
  const settings = screen.getByRole("region", { name: "Stroke" });
  const size = within(settings).getAllByRole("spinbutton")[0];
  await user.clear(size);
  await user.type(size, "12");
  await user.tab();
  expect(last(onchange).stroke).toMatchObject({ size: 12 });
  await user.selectOptions(within(settings).getByRole("combobox", { name: "Position" }), "inside");
  expect(last(onchange).stroke?.position).toBe("inside");
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onok).toHaveBeenCalledOnce();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(oncancel).toHaveBeenCalledOnce();
});

test("Blending Options sets Fill Opacity; an effect's swatch asks for its color", async () => {
  const { onchange, onpickcolor, user } = open(withEffect(null, "colorOverlay", true));
  const fill = within(screen.getByRole("region", { name: "Blending Options" })).getByRole(
    "spinbutton",
  );
  await user.clear(fill);
  await user.type(fill, "40{Enter}");
  expect(last(onchange).fillOpacity).toBeCloseTo(0.4);
  document.body.innerHTML = "";
  const second = open(withEffect(null, "colorOverlay", true), "colorOverlay");
  await second.user.click(screen.getByRole("button", { name: "Color" }));
  expect(second.onpickcolor).toHaveBeenCalledWith("colorOverlay", "color");
  expect(onpickcolor).not.toHaveBeenCalled();
});

test("an effect not added says how to turn it on", () => {
  open(null, "dropShadow");
  expect(screen.getByText("Check this effect in the list to set it.")).toBeInTheDocument();
});

test("an inner glow's settings say Choke, a shadow's has an angle", async () => {
  open(withEffect(null, "innerGlow", true), "innerGlow");
  const settings = screen.getByRole("region", { name: "Inner Glow" });
  expect(within(settings).getByText("Choke")).toBeInTheDocument();
  expect(within(settings).queryByText("Angle")).not.toBeInTheDocument();
  document.body.innerHTML = "";
  const { onchange, user } = open(withEffect(null, "innerShadow", true), "innerShadow");
  const shadow = screen.getByRole("region", { name: "Inner Shadow" });
  expect(within(shadow).getByText("Angle")).toBeInTheDocument();
  await user.selectOptions(within(shadow).getByRole("combobox", { name: "Blend Mode" }), "overlay");
  expect(last(onchange).innerShadow?.mode).toBe("overlay");
});

/** The dialog's box: 560 x 400 centered in a 1200 x 900 window, moved by its `translate`. */
function layOut() {
  window.innerWidth = 1200;
  window.innerHeight = 900;
  vi.spyOn(HTMLDialogElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLDialogElement,
  ) {
    const [x = 0, y = 0] = this.style.translate.split(" ").map((v) => parseFloat(v) || 0);
    return new DOMRect(320 + x, 250 + y, 560, 400);
  });
}

test("dragging the title bar moves the dialog, kept in the window", async () => {
  forgetDialogPlaces();
  layOut();
  open(null);
  const dialog = screen.getByRole("dialog", { hidden: true });
  const bar = screen.getByText("Layer Style");
  await fireEvent.pointerDown(bar, { pointerId: 1, button: 0, clientX: 400, clientY: 260 });
  await fireEvent.pointerMove(bar, { pointerId: 1, clientX: 300, clientY: 300 });
  expect(dialog.style.translate).toBe("-100px 40px");
  // Past the top: its title bar stays reachable.
  await fireEvent.pointerMove(bar, { pointerId: 1, clientX: 300, clientY: -500 });
  expect(dialog.style.translate).toBe("-100px -250px");
  await fireEvent.pointerUp(bar, { pointerId: 1 });
  // A move after the release, or with another button, moves nothing.
  await fireEvent.pointerMove(bar, { pointerId: 1, clientX: 0, clientY: 0 });
  await fireEvent.pointerDown(bar, { pointerId: 2, button: 2, clientX: 400, clientY: 260 });
  await fireEvent.pointerMove(bar, { pointerId: 2, clientX: 500, clientY: 400 });
  expect(dialog.style.translate).toBe("-100px -250px");
  vi.restoreAllMocks();
});

test("the dialog opens again where it was left (after the color picker)", async () => {
  forgetDialogPlaces();
  layOut();
  open(null);
  const bar = screen.getByText("Layer Style");
  await fireEvent.pointerDown(bar, { pointerId: 1, button: 0, clientX: 400, clientY: 260 });
  await fireEvent.pointerMove(bar, { pointerId: 1, clientX: 450, clientY: 290 });
  await fireEvent.pointerUp(bar, { pointerId: 1 });
  document.body.innerHTML = "";
  open(null);
  expect(screen.getByRole("dialog", { hidden: true }).style.translate).toBe("50px 30px");
  vi.restoreAllMocks();
});

test("a slider's arrow drops its slider down without applying the dialog", async () => {
  const { onok, user } = open(null);
  const blending = screen.getByRole("region", { name: "Blending Options" });
  await user.click(within(blending).getByText("▾"));
  expect(onok).not.toHaveBeenCalled();
  expect(within(blending).getAllByRole("slider").length).toBeGreaterThan(1);
});

test("Gradient Overlay: reverse, its style, align, angle and scale sent as they change", async () => {
  const { onchange, onpickcolor, user } = open(
    withEffect(null, "gradientOverlay", true),
    "gradientOverlay",
  );
  const settings = screen.getByRole("region", { name: "Gradient Overlay" });
  await user.click(within(settings).getByRole("checkbox", { name: "Reverse" }));
  expect(last(onchange).gradientOverlay?.reverse).toBe(true);
  const style = within(settings).getByRole("combobox", { name: "Style" });
  // Photoshop's five styles.
  expect(
    within(style)
      .getAllByRole("option")
      .map((o) => o.textContent),
  ).toEqual(["Linear", "Radial", "Angle", "Reflected", "Diamond"]);
  await user.selectOptions(style, "radial");
  expect(last(onchange).gradientOverlay?.shape).toBe("radial");
  await user.selectOptions(style, "Diamond");
  expect(last(onchange).gradientOverlay?.shape).toBe("diamond");
  await user.click(within(settings).getByRole("checkbox", { name: "Align with Layer" }));
  expect(last(onchange).gradientOverlay?.align).toBe(false);
  // Angle and Scale, the page's last two fields.
  const [angle, scale] = within(settings).getAllByRole("spinbutton").slice(-2);
  await user.clear(angle);
  await user.type(angle, "-45");
  await user.tab();
  expect(last(onchange).gradientOverlay?.angle).toBe(-45);
  // Typed digit by digit, 6 would be below the range: the value at once.
  await fireEvent.input(scale, { target: { value: "60" } });
  expect(last(onchange).gradientOverlay).toMatchObject({ scale: 60, mode: "normal" });
  // Its colors are its gradient's: no single color to pick.
  expect(within(settings).queryByRole("button", { name: "Color" })).not.toBeInTheDocument();
  expect(onpickcolor).not.toHaveBeenCalled();
});

test("Satin: its color asked for, Invert and its distance sent as they change", async () => {
  const { onchange, onpickcolor, user } = open(withEffect(null, "satin", true), "satin");
  const settings = screen.getByRole("region", { name: "Satin" });
  await user.click(within(settings).getByRole("button", { name: "Color" }));
  expect(onpickcolor).toHaveBeenCalledWith("satin", "color");
  await user.click(within(settings).getByRole("checkbox", { name: "Invert" }));
  expect(last(onchange).satin?.invert).toBe(false);
  // Opacity, Angle, Distance, Size.
  const [, , distance] = within(settings).getAllByRole("spinbutton");
  await fireEvent.input(distance, { target: { value: "30" } });
  expect(last(onchange).satin).toMatchObject({ distance: 30, mode: "multiply" });
});

test("Bevel and Emboss: its style, direction and each of its two colors", async () => {
  const { onchange, onpickcolor, user } = open(withEffect(null, "bevel", true), "bevel");
  const settings = screen.getByRole("region", { name: "Bevel & Emboss" });
  await user.selectOptions(within(settings).getByRole("combobox", { name: "Style" }), "emboss");
  expect(last(onchange).bevel?.style).toBe("emboss");
  await user.selectOptions(within(settings).getByRole("combobox", { name: "Direction" }), "down");
  expect(last(onchange).bevel?.up).toBe(false);
  await user.click(within(settings).getByRole("button", { name: "Shadow Color" }));
  expect(onpickcolor).toHaveBeenLastCalledWith("bevel", "shadowColor");
  await user.click(within(settings).getByRole("button", { name: "Highlight Color" }));
  expect(onpickcolor).toHaveBeenLastCalledWith("bevel", "highlightColor");
});

test("a Pattern Overlay starts with its pattern chosen, then its settings change", async () => {
  const empty = open(null);
  await empty.user.click(screen.getByRole("checkbox", { name: "Pattern Overlay" }));
  // Nothing to draw yet: the app shows the picker.
  expect(empty.onpickpattern).toHaveBeenCalled();
  expect(empty.onchange).not.toHaveBeenCalled();
});

test("a Pattern Overlay's scale, angle and link", async () => {
  const { onchange, onpickpattern, user } = open(withPatternOverlay(null, 9), "patternOverlay");
  const settings = screen.getByRole("region", { name: "Pattern Overlay" });
  await user.click(within(settings).getByRole("checkbox", { name: "Link with Layer" }));
  expect(last(onchange).patternOverlay).toMatchObject({ source: 9, link: false, scale: 1 });
  await user.click(within(settings).getByRole("button", { name: "Choose another pattern" }));
  expect(onpickpattern).toHaveBeenCalled();
});

// Contours (Photoshop's Quality > Contour): the engine lists its presets, each drawn.

const CONE: StyleContour["points"] = [
  [0, 0, false],
  [128, 255, true],
  [255, 0, false],
];
const GAUSSIAN: StyleContour["points"] = [
  [0, 0, false],
  [64, 18, false],
  [128, 128, false],
  [191, 237, false],
  [255, 255, false],
];

/** The engine's presets (the points of those the tests pick; the others' made up). */
function withPresets() {
  const presets: ContourPreset[] = CONTOUR_PRESET_IDS.map((id, i) => ({
    id,
    contour: {
      points:
        id === "cone"
          ? CONE
          : id === "gaussian"
            ? GAUSSIAN
            : [
                [0, i, false],
                [255, 255 - i, false],
              ],
      samples: [0, 0.5, 1],
    },
  }));
  presets[0].contour.points = [
    [0, 0, false],
    [255, 255, false],
  ];
  mockIPC((cmd) => (cmd === "contour_presets" ? presets : null));
}

afterEach(() => {
  clearMocks();
  forgetContourPresets();
});

test("a shadow's Contour drops down Photoshop's contours; a pick and Anti-aliased are sent", async () => {
  withPresets();
  const { onchange, user } = open(withEffect(null, "dropShadow", true), "dropShadow");
  const settings = screen.getByRole("region", { name: "Drop Shadow" });
  const picker = within(settings).getByRole("button", { name: "Contour" });
  expect(picker).toHaveAttribute("title", "Linear");
  await user.click(picker);
  const list = await within(settings).findByRole("listbox", { name: "Contour" });
  await vi.waitFor(() => expect(within(list).getAllByRole("option")).toHaveLength(12));
  expect(within(list).getByRole("option", { name: "Linear" })).toHaveAttribute(
    "aria-selected",
    "true",
  );
  await user.click(within(list).getByRole("option", { name: "Cone" }));
  expect(last(onchange).dropShadow?.contour?.points).toEqual(CONE);
  expect(within(settings).queryByRole("listbox")).not.toBeInTheDocument();
  expect(picker).toHaveAttribute("title", "Cone");
  await user.click(within(settings).getByRole("checkbox", { name: "Anti-aliased" }));
  expect(last(onchange).dropShadow?.antiAliased).toBe(true);
});

test("a glow has a Range; a contour from Photoshop that is no preset shows as Custom", async () => {
  withPresets();
  const glow = withEffect(null, "outerGlow", true);
  const custom: StyleContour = {
    points: [
      [0, 40, false],
      [90, 200, true],
      [255, 10, false],
    ],
    samples: [0.2, 0.8, 0.1],
  };
  const { onchange, user } = open(
    { ...glow, outerGlow: { ...glow.outerGlow!, contour: custom } },
    "outerGlow",
  );
  const settings = screen.getByRole("region", { name: "Outer Glow" });
  const picker = within(settings).getByRole("button", { name: "Contour" });
  await vi.waitFor(() => expect(picker).toHaveAttribute("title", "Custom"));
  const range = within(settings).getAllByRole("spinbutton").at(-1)!;
  expect(range).toHaveValue(50);
  await user.clear(range);
  await user.type(range, "30");
  await user.tab();
  expect(last(onchange).outerGlow).toMatchObject({ range: 30, contour: custom });
  // A shadow has none.
  document.body.innerHTML = "";
  open(withEffect(null, "innerShadow", true), "innerShadow");
  const shadow = screen.getByRole("region", { name: "Inner Shadow" });
  expect(within(shadow).queryByText("Range")).not.toBeInTheDocument();
});

test("Satin's contour is Photoshop's Gaussian, anti-aliased", async () => {
  withPresets();
  const { onchange, user } = open(withEffect(null, "satin", true), "satin");
  const settings = screen.getByRole("region", { name: "Satin" });
  expect(within(settings).getByRole("button", { name: "Contour" })).toHaveAttribute(
    "title",
    "Gaussian",
  );
  const anti = within(settings).getByRole("checkbox", { name: "Anti-aliased" });
  expect(anti).toBeChecked();
  await user.click(anti);
  expect(last(onchange).satin?.antiAliased).toBe(false);
});

test("Bevel & Emboss: its Gloss Contour, and its Contour element turned on from the list", async () => {
  withPresets();
  const { onchange, onpage, user } = open(null, "bevel");
  // The Contour element on: the bevel is turned on with it, its page shown.
  await user.click(screen.getByRole("checkbox", { name: "Contour" }));
  expect(last(onchange).bevel).toMatchObject({ enabled: true, contourEnabled: true });
  expect(onpage).toHaveBeenLastCalledWith("bevelContour");
  expect(screen.getByRole("checkbox", { name: "Bevel & Emboss" })).toBeChecked();
  // Its page, as the app shows it next.
  document.body.innerHTML = "";
  const next = open(last(onchange), "bevelContour");
  const page = screen.getByRole("region", { name: "Contour" });
  await next.user.click(within(page).getByRole("button", { name: "Contour" }));
  await next.user.click(await within(page).findByRole("option", { name: "Cone" }));
  expect(last(next.onchange).bevel?.contour?.points).toEqual(CONE);
  const range = within(page).getByRole("spinbutton");
  await next.user.clear(range);
  await next.user.type(range, "70");
  await next.user.tab();
  expect(last(next.onchange).bevel?.contourRange).toBe(70);
  // Off: the bevel stays, its Contour page says how to turn it on.
  await next.user.click(screen.getByRole("checkbox", { name: "Contour" }));
  expect(last(next.onchange).bevel).toMatchObject({ enabled: true, contourEnabled: false });
  expect(within(page).getByText("Check this effect in the list to set it.")).toBeInTheDocument();
  // The Gloss Contour, on the bevel's page.
  document.body.innerHTML = "";
  const bevel = open(last(next.onchange), "bevel");
  const shading = screen.getByRole("region", { name: "Bevel & Emboss" });
  await bevel.user.click(within(shading).getByRole("button", { name: "Gloss Contour" }));
  await bevel.user.click(await within(shading).findByRole("option", { name: "Gaussian" }));
  expect(last(bevel.onchange).bevel?.glossContour?.points).toEqual(GAUSSIAN);
  await bevel.user.click(within(shading).getByRole("checkbox", { name: "Anti-aliased" }));
  expect(last(bevel.onchange).bevel?.glossAntiAliased).toBe(true);
});
