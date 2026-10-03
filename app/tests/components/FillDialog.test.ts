import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import FillDialog from "../../src/lib/FillDialog.svelte";

function open(color = "#336699") {
  const onpickcolor = vi.fn();
  const onchoose = vi.fn();
  const onclose = vi.fn();
  render(FillDialog, { color, onpickcolor, onchoose, onclose });
  return { onpickcolor, onchoose, onclose, user: userEvent.setup() };
}

const contents = () => screen.getByRole("combobox", { name: "Contents:" });
const opacity = () => screen.getByRole("spinbutton");
const ok = () => screen.getByRole("button", { name: "OK" });

// The dialog remembers its last settings in the module, for the whole session: the first test
// below is the only one that sees the defaults, the others set what they depend on.

test("by default it fills with the foreground color at full opacity", async () => {
  const { onchoose, user } = open();
  expect(contents()).toHaveValue("foreground");
  expect(opacity()).toHaveValue(100);
  // The color swatch only shows for Color….
  expect(screen.queryByRole("button", { name: "Fill color" })).not.toBeInTheDocument();
  await user.click(ok());
  expect(onchoose).toHaveBeenCalledWith({ contents: "foreground", opacity: 1 });
});

test("the contents and the opacity chosen are applied, and come back next time", async () => {
  const first = open();
  await first.user.selectOptions(contents(), "gray");
  await first.user.clear(opacity());
  await first.user.type(opacity(), "40");
  await first.user.click(ok());
  expect(first.onchoose).toHaveBeenCalledWith({ contents: "gray", opacity: 0.4 });
  document.body.innerHTML = "";
  open();
  expect(contents()).toHaveValue("gray");
  expect(opacity()).toHaveValue(40);
});

test("Color… opens the color picker at once, and its swatch opens it again", async () => {
  const { onpickcolor, onchoose, user } = open("#336699");
  await user.clear(opacity());
  await user.type(opacity(), "70");
  await user.selectOptions(contents(), "color");
  expect(onpickcolor).toHaveBeenCalledWith({ contents: "color", opacity: 0.7 });
  const swatch = screen.getByRole("button", { name: "Fill color" });
  expect(swatch).toHaveStyle({ background: "rgb(51, 102, 153)" });
  await user.click(swatch);
  expect(onpickcolor).toHaveBeenCalledTimes(2);
  expect(onchoose).not.toHaveBeenCalled();
});

test("Enter on the closed list applies, as in Photoshop", async () => {
  const { onchoose, user } = open();
  await user.selectOptions(contents(), "white");
  contents().focus();
  await user.keyboard("{Enter}");
  expect(onchoose).toHaveBeenCalledOnce();
  expect(onchoose).toHaveBeenCalledWith(expect.objectContaining({ contents: "white" }));
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
