import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import StrokePathDialog from "../../src/lib/StrokePathDialog.svelte";
import WorkPathDialog from "../../src/lib/WorkPathDialog.svelte";

// The Paths panel's dialogs (ADR 0041): Stroke Path's tool and Simulate Pressure, Make Work
// Path's tolerance.

test("Stroke Path: the tool and Simulate Pressure chosen, starting from the last ones", async () => {
  const onapply = vi.fn();
  const onclose = vi.fn();
  render(StrokePathDialog, {
    settings: { tool: "brush", simulatePressure: false },
    onapply,
    onclose,
  });
  const user = userEvent.setup();
  const tool = screen.getByRole("combobox", { name: "Tool" });
  expect(tool).toHaveValue("brush");
  await user.selectOptions(tool, "eraser");
  await user.click(screen.getByRole("checkbox", { name: "Simulate Pressure" }));
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(onapply).toHaveBeenCalledWith({ tool: "eraser", simulatePressure: true });
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onclose).toHaveBeenCalledOnce();
});

test("Make Work Path: a tolerance from 0.5 to 10 pixels", async () => {
  const onapply = vi.fn();
  render(WorkPathDialog, { tolerance: 2, onapply, onclose: vi.fn() });
  const user = userEvent.setup();
  const field = screen.getByRole("spinbutton", { name: "Tolerance" });
  expect(field).toHaveValue(2);
  await user.clear(field);
  await user.type(field, "12");
  expect(screen.getByRole("button", { name: "OK" })).toBeDisabled();
  await user.clear(field);
  await user.type(field, "0.5{Enter}");
  expect(onapply).toHaveBeenCalledWith(0.5);
});
