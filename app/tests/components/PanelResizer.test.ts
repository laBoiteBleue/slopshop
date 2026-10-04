import { fireEvent, render, screen } from "@testing-library/svelte";
import { beforeEach, expect, test } from "vitest";
import { tick } from "svelte";
import { reactive } from "./reactive.svelte";
import PanelResizer from "../../src/lib/PanelResizer.svelte";
import { DEFAULT_PANEL_WIDTH, MIN_PANEL_WIDTH } from "../../src/lib/panelWidth";

beforeEach(() => {
  localStorage.clear();
  window.innerWidth = 1920;
});

function open(width = 260) {
  const props = reactive({ width });
  render(PanelResizer, props);
  return { props, edge: screen.getByRole("separator", { name: "Resize the panels" }) };
}

test("dragging the left edge left widens the panels", async () => {
  const { props, edge } = open();
  await fireEvent.pointerDown(edge, { pointerId: 1, button: 0, clientX: 1600 });
  await fireEvent.pointerMove(edge, { pointerId: 1, clientX: 1500 });
  expect(props.width).toBe(360);
  await fireEvent.pointerMove(edge, { pointerId: 1, clientX: 1700 });
  expect(props.width).toBe(MIN_PANEL_WIDTH);
  await fireEvent.pointerMove(edge, { pointerId: 1, clientX: 1550 });
  await fireEvent.pointerUp(edge, { pointerId: 1, clientX: 1550 });
  expect(props.width).toBe(310);
  await tick();
  expect(edge).toHaveAttribute("aria-valuenow", "310");
});

test("the panels never take the image's room", async () => {
  window.innerWidth = 1000;
  const { props, edge } = open();
  await fireEvent.pointerDown(edge, { pointerId: 1, button: 0, clientX: 740 });
  await fireEvent.pointerMove(edge, { pointerId: 1, clientX: 0 });
  expect(props.width).toBe(720);
});

test("moves without a press, or another button, resize nothing", async () => {
  const { props, edge } = open();
  await fireEvent.pointerMove(edge, { pointerId: 1, clientX: 1500 });
  await fireEvent.pointerDown(edge, { pointerId: 1, button: 2, clientX: 1600 });
  await fireEvent.pointerMove(edge, { pointerId: 1, clientX: 1500 });
  expect(props.width).toBe(260);
});

test("a double-click puts the default width back", async () => {
  const { props, edge } = open(500);
  await fireEvent.dblClick(edge);
  expect(props.width).toBe(DEFAULT_PANEL_WIDTH);
});
