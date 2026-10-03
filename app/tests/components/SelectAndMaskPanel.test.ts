import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import SelectAndMaskPanel, {
  DEFAULT_REFINE,
  type RefineSettings,
} from "../../src/lib/SelectAndMaskPanel.svelte";
import { reactive } from "./reactive.svelte";

function open(canOutputToLayer = true, busy = false) {
  const settings = reactive<RefineSettings>(structuredClone(DEFAULT_REFINE));
  const props = {
    settings,
    canOutputToLayer,
    busy,
    onview: vi.fn(),
    onedges: vi.fn(),
    ondetect: vi.fn(),
    onapply: vi.fn(),
    onclose: vi.fn(),
  };
  render(SelectAndMaskPanel, props);
  return { ...props, user: userEvent.setup() };
}

/** The number field of the setting labelled `label`. */
const field = (label: string) =>
  screen.getByText(label).parentElement!.querySelector("input[type=number]") as HTMLInputElement;

test("it shows the view and the settings at once, then each change", async () => {
  const { settings, onview, onedges, user } = open();
  expect(onview).toHaveBeenLastCalledWith("overlay");
  expect(onedges).toHaveBeenLastCalledWith({ smooth: 0, feather: 0, contrast: 0, shift: 0 });
  await user.selectOptions(screen.getByRole("combobox", { name: "View:" }), "onBlack");
  expect(onview).toHaveBeenLastCalledWith("onBlack");
  await user.clear(field("Feather:"));
  await user.type(field("Feather:"), "6");
  await user.clear(field("Shift Edge:"));
  await user.type(field("Shift Edge:"), "-3");
  expect(onedges).toHaveBeenLastCalledWith({ smooth: 0, feather: 6, contrast: 0, shift: -3 });
  expect(settings.edges.feather).toBe(6);
});

test("Detect runs edge detection with the radius", async () => {
  const { ondetect, user } = open();
  await user.clear(field("Radius:"));
  await user.type(field("Radius:"), "40");
  await user.click(screen.getByRole("button", { name: "Detect" }));
  expect(ondetect).toHaveBeenCalledWith(40);
});

test("the outputs to a layer wait for an active layer", () => {
  open(false);
  const output = screen.getByRole("combobox", { name: "Output To:" });
  expect(output.querySelector("option[value=layerMask]")).toBeDisabled();
  expect(output.querySelector("option[value=newLayer]")).toBeDisabled();
  expect(output.querySelector("option[value=selection]")).toBeEnabled();
});

test("OK and Enter apply, Cancel and Escape close; busy, nothing applies", async () => {
  const { onapply, onclose, user } = open();
  await user.click(screen.getByRole("button", { name: "OK" }));
  await user.keyboard("{Enter}");
  expect(onapply).toHaveBeenCalledTimes(2);
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  await user.keyboard("{Escape}");
  expect(onclose).toHaveBeenCalledTimes(2);
});

test("while detecting, OK and Detect wait", () => {
  open(true, true);
  expect(screen.getByRole("button", { name: "OK" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Detect" })).toBeDisabled();
});
