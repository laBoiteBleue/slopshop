import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import AdjustDialog from "../../src/lib/AdjustDialog.svelte";
import { ADJUSTMENT_PARAMS, type AdjustmentId } from "../../src/lib/engine";

// The fields themselves are tested with the Properties panel, which shares them: this tests what
// the dialog adds (the buttons, the preview, the keys, the way the fields are wired).

const padded = (values: number[]) => [
  ...values,
  ...Array<number>(ADJUSTMENT_PARAMS - values.length).fill(0),
];

function open(id: AdjustmentId = "hueSaturation", values: number[] = [], preview = true) {
  const callbacks = {
    onlive: vi.fn(),
    oncurves: vi.fn(),
    onpreview: vi.fn(),
    onok: vi.fn(),
    oncancel: vi.fn(),
  };
  render(AdjustDialog, {
    adjustment: { id, values: padded(values), curves: null, curveSamples: null, gradient: null },
    preview,
    ...callbacks,
  });
  return { ...callbacks, user: userEvent.setup() };
}

const number = (name: string) => screen.getByRole("spinbutton", { name });

test("the title is the adjustment's, and a setting changed is sent live for the canvas", async () => {
  const { onlive, onok, user } = open("hueSaturation", [0, 0, 0]);
  expect(screen.getByRole("dialog", { name: "Hue/Saturation" })).toBeInTheDocument();
  await user.clear(number("Hue"));
  await user.type(number("Hue"), "30");
  await user.tab();
  expect(onlive).toHaveBeenLastCalledWith(padded([30, 0, 0]));
  // Changing a setting does not apply it: only OK does.
  expect(onok).not.toHaveBeenCalled();
});

test("OK applies, and so does Enter", async () => {
  const { onok, user } = open("hueSaturation", [10, 0, 0]);
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onok).toHaveBeenCalledOnce();
  await user.type(number("Hue"), "{Enter}");
  expect(onok).toHaveBeenCalledTimes(2);
});

test("Enter applies from a list too, which a form does not do by itself", async () => {
  const { onok, user } = open("colorBalance", [0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
  await user.selectOptions(screen.getByRole("combobox", { name: "Tone" }), "Highlights");
  screen.getByRole("combobox", { name: "Tone" }).focus();
  await user.keyboard("{Enter}");
  expect(onok).toHaveBeenCalledOnce();
});

test("Cancel and Escape leave everything as it was, and the app's keys wait meanwhile", async () => {
  const { onok, oncancel, user } = open();
  const appKeys = vi.fn();
  window.addEventListener("keydown", appKeys);
  await user.keyboard("v");
  window.removeEventListener("keydown", appKeys);
  expect(appKeys).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(oncancel).toHaveBeenCalledTimes(2);
  expect(onok).not.toHaveBeenCalled();
});

test("Preview shows whether the canvas follows the settings, and turns it off or on", async () => {
  const { onpreview, user } = open("hueSaturation", [], true);
  const preview = screen.getByRole("checkbox", { name: "Preview" });
  expect(preview).toBeChecked();
  await user.click(preview);
  expect(onpreview).toHaveBeenLastCalledWith(false);
});

test("Preview is off when the app says so", () => {
  open("hueSaturation", [], false);
  expect(screen.getByRole("checkbox", { name: "Preview" })).not.toBeChecked();
});

test("an entry applied several times in a row chooses which application it edits", async () => {
  const onstep = vi.fn();
  const user = userEvent.setup();
  render(AdjustDialog, {
    adjustment: {
      id: "hueSaturation",
      values: padded([10, 0, 0]),
      curves: null,
      curveSamples: null,
      gradient: null,
    },
    preview: true,
    steps: 2,
    step: 1,
    onstep,
    onlive: vi.fn(),
    oncurves: vi.fn(),
    onpreview: vi.fn(),
    onok: vi.fn(),
    oncancel: vi.fn(),
  });
  const which = screen.getByRole("combobox", { name: "Application" });
  expect(which).toHaveDisplayValue("2 of 2");
  await user.selectOptions(which, "1 of 2");
  expect(onstep).toHaveBeenCalledWith(0);
});

test("an adjustment applied once has no choice of application", () => {
  open("hueSaturation", [0, 0, 0]);
  expect(screen.queryByRole("combobox", { name: "Application" })).toBeNull();
});
