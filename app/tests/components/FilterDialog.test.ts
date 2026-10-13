import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import FilterDialog from "../../src/lib/FilterDialog.svelte";
import { forgetDialogPlaces } from "../../src/lib/dialogDrag";

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

test("its title bar moves it, and it opens again where it was left", async () => {
  forgetDialogPlaces();
  window.innerWidth = 1200;
  window.innerHeight = 900;
  vi.spyOn(HTMLDialogElement.prototype, "getBoundingClientRect").mockImplementation(function (
    this: HTMLDialogElement,
  ) {
    const [x = 0, y = 0] = this.style.translate.split(" ").map((v) => parseFloat(v) || 0);
    return new DOMRect(430 + x, 350 + y, 340, 200);
  });
  open([2]);
  const bar = screen.getByText("Gaussian Blur");
  await fireEvent.pointerDown(bar, { pointerId: 1, button: 0, clientX: 500, clientY: 360 });
  await fireEvent.pointerMove(bar, { pointerId: 1, clientX: 380, clientY: 400 });
  await fireEvent.pointerUp(bar, { pointerId: 1 });
  expect(screen.getByRole("dialog").style.translate).toBe("-120px 40px");
  document.body.innerHTML = "";
  open([2], { filter: "motionBlur", values: [0, 10] });
  expect(screen.getByRole("dialog").style.translate).toBe("-120px 40px");
  vi.restoreAllMocks();
});

test("Shadows/Highlights: its Amounts under Shadows and Highlights, the rest with Show More Options", async () => {
  const values = [35, 50, 30, 0, 50, 30, 20, 0];
  const { onlive, onok, user } = open(values, { filter: "shadowsHighlights" });
  expect(screen.getByRole("dialog", { name: "Shadows/Highlights" })).toBeInTheDocument();
  const amounts = screen.getAllByRole("spinbutton", { name: "Amount" });
  expect(amounts.map((a) => (a as HTMLInputElement).valueAsNumber)).toEqual([35, 0]);
  expect(screen.getByText("Shadows")).toBeInTheDocument();
  expect(screen.getByText("Highlights")).toBeInTheDocument();
  expect(screen.queryByText("Adjustments")).not.toBeInTheDocument();
  expect(screen.queryByRole("spinbutton", { name: "Tone" })).not.toBeInTheDocument();
  await user.click(screen.getByRole("checkbox", { name: "Show More Options" }));
  expect(screen.getAllByRole("spinbutton", { name: "Tone" })).toHaveLength(2);
  expect(screen.getAllByRole("spinbutton", { name: "Radius" })).toHaveLength(2);
  expect(screen.getByRole("spinbutton", { name: "Color" })).toHaveValue(20);
  expect(screen.getByText("Adjustments")).toBeInTheDocument();
  // Hidden settings keep their values: all eight are sent.
  await user.clear(amounts[1]);
  await user.type(amounts[1], "25");
  expect(onlive).toHaveBeenLastCalledWith([35, 50, 30, 25, 50, 30, 20, 0]);
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onok).toHaveBeenCalledWith([35, 50, 30, 25, 50, 30, 20, 0]);
});

test("filters without More Options have no such box", () => {
  open([1]);
  expect(screen.queryByRole("checkbox", { name: "Show More Options" })).not.toBeInTheDocument();
});

test("Wave: its settings under Photoshop's headings, Randomize draws other waves, shown live", async () => {
  const values = [5, 10, 120, 5, 35, 100, 100, 0, 1, 77];
  const { onlive, onok, user } = open(values, { filter: "wave" });
  expect(screen.getByRole("dialog", { name: "Wave" })).toBeInTheDocument();
  for (const heading of ["Wavelength", "Amplitude", "Scale"]) {
    expect(screen.getByText(heading)).toBeInTheDocument();
  }
  expect(screen.getAllByRole("spinbutton", { name: "Min." })).toHaveLength(2);
  expect(screen.getByRole("radio", { name: "Sine" })).toBeChecked();
  expect(screen.getByRole("radio", { name: "Repeat Edge Pixels" })).toBeChecked();
  expect(onlive).toHaveBeenLastCalledWith(values);
  const random = vi.spyOn(Math, "random").mockReturnValue(0.5);
  await user.click(screen.getByRole("button", { name: "Randomize" }));
  random.mockRestore();
  const randomized = [...values.slice(0, 9), 2 ** 23];
  expect(onlive).toHaveBeenLastCalledWith(randomized);
  await user.click(screen.getByRole("radio", { name: "Square" }));
  await user.click(screen.getByRole("radio", { name: "Wrap Around" }));
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onok).toHaveBeenCalledWith([5, 10, 120, 5, 35, 100, 100, 2, 0, 2 ** 23]);
});
