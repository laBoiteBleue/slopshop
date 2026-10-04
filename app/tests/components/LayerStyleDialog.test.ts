import { render, screen, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import type { LayerStyle } from "../../src/lib/engine";
import LayerStyleDialog, { type StylePage } from "../../src/lib/LayerStyleDialog.svelte";
import { withEffect } from "../../src/lib/layerStyle";

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
