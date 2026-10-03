import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import StrokeDialog from "../../src/lib/StrokeDialog.svelte";

function open(color = "#336699") {
  const onpickcolor = vi.fn();
  const onchoose = vi.fn();
  const onclose = vi.fn();
  render(StrokeDialog, { color, onpickcolor, onchoose, onclose });
  return { onpickcolor, onchoose, onclose, user: userEvent.setup() };
}

/** The two number fields: the width, then the opacity. */
const width = () => screen.getAllByRole("spinbutton")[0];
const opacity = () => screen.getAllByRole("spinbutton")[1];
const ok = () => screen.getByRole("button", { name: "OK" });

// The dialog remembers its last settings in the module, for the whole session: the first test
// below is the only one that sees the defaults, the others set what they depend on.

test("by default it strokes 1 px centered on the outline at full opacity", async () => {
  const { onchoose, user } = open();
  expect(width()).toHaveValue(1);
  expect(screen.getByRole("radio", { name: "Center" })).toBeChecked();
  expect(opacity()).toHaveValue(100);
  await user.click(ok());
  expect(onchoose).toHaveBeenCalledWith({ width: 1, location: "center", opacity: 1 });
});

test("the width, location and opacity chosen are applied, and come back next time", async () => {
  const first = open();
  await first.user.clear(width());
  await first.user.type(width(), "12");
  await first.user.click(screen.getByRole("radio", { name: "Outside" }));
  await first.user.clear(opacity());
  await first.user.type(opacity(), "25");
  await first.user.click(ok());
  expect(first.onchoose).toHaveBeenCalledWith({ width: 12, location: "outside", opacity: 0.25 });
  document.body.innerHTML = "";
  open();
  expect(width()).toHaveValue(12);
  expect(screen.getByRole("radio", { name: "Outside" })).toBeChecked();
  expect(opacity()).toHaveValue(25);
});

test("the width is kept between 1 and 250 pixels", async () => {
  const { onchoose, user } = open();
  await user.clear(width());
  await user.type(width(), "900");
  expect(width()).toHaveValue(250);
  await user.clear(width());
  await user.type(width(), "0");
  expect(width()).toHaveValue(1);
  await user.click(ok());
  expect(onchoose).toHaveBeenCalledWith(expect.objectContaining({ width: 1 }));
});

test("the color swatch shows the color and opens the picker with the settings so far", async () => {
  const { onpickcolor, onchoose, user } = open("#336699");
  await user.click(screen.getByRole("radio", { name: "Inside" }));
  await user.clear(width());
  await user.type(width(), "5");
  const swatch = screen.getByRole("button", { name: "Choose the stroke color" });
  expect(swatch).toHaveStyle({ background: "rgb(51, 102, 153)" });
  await user.click(swatch);
  expect(onpickcolor).toHaveBeenCalledWith(
    expect.objectContaining({ width: 5, location: "inside" }),
  );
  expect(onchoose).not.toHaveBeenCalled();
});

test("Cancel and Escape close without applying, and the app's keys wait meanwhile", async () => {
  const { onchoose, onclose, user } = open();
  const appKeys = vi.fn();
  window.addEventListener("keydown", appKeys);
  await user.keyboard("v");
  window.removeEventListener("keydown", appKeys);
  expect(appKeys).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(onclose).toHaveBeenCalledTimes(2);
  expect(onchoose).not.toHaveBeenCalled();
});
