import { fireEvent, render, screen, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import type { LayerStyle } from "../../src/lib/engine";
import LayerStyleDialog, { type StylePage } from "../../src/lib/LayerStyleDialog.svelte";
import { withEffect } from "../../src/lib/layerStyle";
import { forgetDialogPlaces } from "../../src/lib/dialogDrag";

function open(style: LayerStyle | null, page: StylePage = "blending") {
  const props = {
    style,
    page,
    onchange: vi.fn(),
    onpage: vi.fn(),
    onpickcolor: vi.fn(),
    onok: vi.fn(),
    oncancel: vi.fn(),
  };
  render(LayerStyleDialog, props);
  return { ...props, user: userEvent.setup() };
}

/** The style of the last change sent. */
const last = (fn: ReturnType<typeof vi.fn>) => fn.mock.lastCall?.[0] as LayerStyle;

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
  expect(second.onpickcolor).toHaveBeenCalledWith("colorOverlay");
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
  await user.selectOptions(within(settings).getByRole("combobox", { name: "Style" }), "radial");
  expect(last(onchange).gradientOverlay?.shape).toBe("radial");
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
  expect(onpickcolor).toHaveBeenCalledWith("satin");
  await user.click(within(settings).getByRole("checkbox", { name: "Invert" }));
  expect(last(onchange).satin?.invert).toBe(false);
  // Opacity, Angle, Distance, Size.
  const [, , distance] = within(settings).getAllByRole("spinbutton");
  await fireEvent.input(distance, { target: { value: "30" } });
  expect(last(onchange).satin).toMatchObject({ distance: 30, mode: "multiply" });
});
