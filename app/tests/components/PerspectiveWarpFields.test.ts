import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import PerspectiveWarpFields from "../../src/lib/PerspectiveWarpFields.svelte";

test("Layout and Warp: the mode on is pressed, the other one sent", async () => {
  const onmode = vi.fn();
  render(PerspectiveWarpFields, { mode: "layout", warpable: true, onmode });
  const layout = screen.getByRole("button", { name: "Layout" });
  const warp = screen.getByRole("button", { name: "Warp" });
  expect(layout).toHaveAttribute("aria-pressed", "true");
  expect(warp).toHaveAttribute("aria-pressed", "false");
  const user = userEvent.setup();
  await user.click(layout);
  expect(onmode).not.toHaveBeenCalled();
  await user.click(warp);
  expect(onmode).toHaveBeenLastCalledWith("warp");
});

test("without a plane, there is nothing to warp", () => {
  render(PerspectiveWarpFields, { mode: "layout", warpable: false, onmode: vi.fn() });
  expect(screen.getByRole("button", { name: "Warp" })).toBeDisabled();
});
