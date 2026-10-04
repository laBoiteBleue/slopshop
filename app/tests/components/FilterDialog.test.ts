import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import FilterDialog from "../../src/lib/FilterDialog.svelte";

function open(values = [1], extra: Record<string, unknown> = {}) {
  const callbacks = {
    onlive: vi.fn(),
    onpreview: vi.fn(),
    onok: vi.fn(),
    oncancel: vi.fn(),
  };
  render(FilterDialog, { filter: "gaussianBlur", values, preview: true, ...callbacks, ...extra });
  return { ...callbacks, user: userEvent.setup() };
}

const radius = () => screen.getByRole("spinbutton", { name: "Radius" });

test("Gaussian Blur's radius in pixels, shown live from the start; OK applies it", async () => {
  const { onlive, onok, user } = open([2.5]);
  expect(screen.getByRole("dialog", { name: "Gaussian Blur" })).toBeInTheDocument();
  expect(radius()).toHaveValue(2.5);
  expect(onlive).toHaveBeenLastCalledWith([2.5]);
  await user.clear(radius());
  await user.type(radius(), "12");
  expect(onlive).toHaveBeenLastCalledWith([12]);
  expect(onok).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onok).toHaveBeenCalledWith([12]);
});

test("a radius out of range is not shown and cannot be applied", async () => {
  const { onlive, user } = open([3]);
  await user.clear(radius());
  await user.type(radius(), "2000");
  expect(onlive).not.toHaveBeenCalledWith([2000]);
  expect(screen.getByRole("button", { name: "OK" })).toBeDisabled();
});

test("the slider is logarithmic: its middle is the geometric middle of 0.1 and 1000", async () => {
  const { onlive } = open([1]);
  const slider = screen.getByRole("slider", { name: "Radius" }) as HTMLInputElement;
  // 1 pixel is a quarter of the way.
  expect(Number(slider.value)).toBeCloseTo(250, 0);
  slider.value = "500";
  slider.dispatchEvent(new Event("input", { bubbles: true }));
  await vi.waitFor(() => expect(onlive).toHaveBeenLastCalledWith([10]));
});

test("Unsharp Mask: its amount in percent, its radius in pixels, its threshold in levels", async () => {
  const { onlive, onok, user } = open([100, 1, 0], { filter: "unsharpMask" });
  expect(screen.getByRole("dialog", { name: "Unsharp Mask" })).toBeInTheDocument();
  const amount = screen.getByRole("spinbutton", { name: "Amount" });
  const threshold = screen.getByRole("spinbutton", { name: "Threshold" });
  expect(amount).toHaveValue(100);
  expect(radius()).toHaveValue(1);
  expect(threshold).toHaveValue(0);
  for (const unit of ["%", "pixels", "levels"]) expect(screen.getByText(unit)).toBeInTheDocument();
  await user.clear(amount);
  await user.type(amount, "250");
  const slider = screen.getByRole("slider", { name: "Threshold" }) as HTMLInputElement;
  slider.value = "200";
  slider.dispatchEvent(new Event("input", { bubbles: true }));
  await vi.waitFor(() => expect(onlive).toHaveBeenLastCalledWith([250, 1, 51]));
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onok).toHaveBeenCalledWith([250, 1, 51]);
});

test("Clarity and Texture: two strengths, no unit after them", async () => {
  const { onok, user } = open([0, 0], { filter: "clarityTexture" });
  expect(screen.getByRole("dialog", { name: "Clarity and Texture" })).toBeInTheDocument();
  expect(screen.queryByText("pixels")).toBeNull();
  const clarity = screen.getByRole("spinbutton", { name: "Clarity" });
  await user.clear(clarity);
  await user.type(clarity, "-40");
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onok).toHaveBeenCalledWith([0, -40]);
});

test("Preview, Cancel and Escape; the app's keys wait meanwhile", async () => {
  const { onpreview, oncancel, onok, user } = open();
  const appKeys = vi.fn();
  window.addEventListener("keydown", appKeys);
  await user.keyboard("v");
  window.removeEventListener("keydown", appKeys);
  expect(appKeys).not.toHaveBeenCalled();
  await user.click(screen.getByRole("checkbox", { name: "Preview" }));
  expect(onpreview).toHaveBeenCalledWith(false);
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(oncancel).toHaveBeenCalledOnce();
  screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(oncancel).toHaveBeenCalledTimes(2);
  expect(onok).not.toHaveBeenCalled();
});
