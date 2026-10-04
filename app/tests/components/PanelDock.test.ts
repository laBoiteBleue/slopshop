import { fireEvent, render, screen } from "@testing-library/svelte";
import { createRawSnippet } from "svelte";
import { expect, test, vi } from "vitest";
import PanelDock from "../../src/lib/PanelDock.svelte";
import type { DockState } from "../../src/lib/panelDock";
import { reactive } from "./reactive.svelte";

const panels = [
  { id: "properties" as const, icon: "sliders" as const, label: "Properties" },
  { id: "selections" as const, icon: "marquee" as const, label: "Selections" },
];
const content = createRawSnippet((panel: () => string) => ({
  render: () => `<p>${panel()}</p>`,
}));

test("a tab dragged along the row is reordered, without unfolding it; a click selects", async () => {
  const props = reactive({ dock: { open: "properties", height: 280 } as DockState });
  const onreorder = vi.fn();
  const onselect = vi.fn();
  render(PanelDock, {
    get dock() {
      return props.dock;
    },
    set dock(dock) {
      props.dock = dock;
    },
    panels,
    content,
    onreorder,
    onselect,
  });
  const tab = screen.getByRole("tab", { name: /Properties/ });
  await fireEvent.pointerDown(tab, { pointerId: 1, button: 0, clientX: 10 });
  // Under the threshold: still a click.
  await fireEvent.pointerMove(tab, { pointerId: 1, clientX: 12 });
  expect(tab).not.toHaveClass("dragging");
  await fireEvent.pointerMove(tab, { pointerId: 1, clientX: 100 });
  expect(tab).toHaveClass("dragging");
  await fireEvent.pointerUp(tab, { pointerId: 1, clientX: 100 });
  await fireEvent.click(tab);
  // jsdom lays nothing out: every tab's middle is at 0, so 100 is past the last one.
  expect(onreorder).toHaveBeenCalledWith(0, 2);
  expect(onselect).not.toHaveBeenCalled();
  expect(screen.getByRole("tabpanel")).toHaveTextContent("properties");
  await fireEvent.click(screen.getByRole("tab", { name: /Selections/ }));
  expect(onselect).toHaveBeenCalledWith("selections");
  expect(props.dock.open).toBe("selections");
});
