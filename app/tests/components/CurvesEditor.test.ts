import { fireEvent, render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import CurvesEditor from "../../src/lib/CurvesEditor.svelte";

type Point = [number, number];

const LINE: Point[] = [
  [0, 0],
  [255, 255],
];
/** Composite, red, green, blue. */
const CURVES = () => [LINE, LINE, LINE, LINE].map((c) => c.map((p) => [...p]));
const SAMPLES = [
  [0, 0.5, 1],
  [0, 0.25, 1],
  [0, 0.5, 1],
  [0, 0.75, 1],
];

/**
 * The editor as the Properties panel hosts it: every step sent (live or applied) reaches the
 * engine, which gives back the curves of the layer.
 */
function open(curves: number[][][] = CURVES()) {
  const engine = (sent: unknown) => {
    const next = JSON.parse(JSON.stringify(sent));
    void view.rerender({ curves: next, samples: SAMPLES, onlive, onend, onapply });
  };
  const onlive = vi.fn(engine);
  const onend = vi.fn();
  const onapply = vi.fn(engine);
  const props = { curves, samples: SAMPLES, onlive, onend, onapply };
  const view = render(CurvesEditor, props);
  const svg = screen.getByRole("application", { name: "Curves" }) as unknown as SVGSVGElement;
  // jsdom lays nothing out: the graph is 255 px wide and high from the window's corner, so
  // that a pointer position is the 0-255 position (the vertical axis pointing down).
  vi.spyOn(svg, "getBoundingClientRect").mockReturnValue({
    left: 0,
    top: 0,
    width: 255,
    height: 255,
    right: 255,
    bottom: 255,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  });
  return { ...view, props, onlive, onend, onapply, svg, user: userEvent.setup() };
}

/** The pointer position of the curve position (x, y). */
const at = (x: number, y: number) => ({ clientX: x, clientY: 255 - y });

/** A drag on the graph through `path`, from the first position to the last. */
async function drag(
  user: ReturnType<typeof userEvent.setup>,
  svg: Element,
  ...path: [number, number][]
) {
  const [first, ...rest] = path;
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: at(...first) },
    ...rest.map((p) => ({ target: svg, coords: at(...p) })),
    { keys: "[/MouseLeft]", target: svg, coords: at(...path[path.length - 1]) },
  ]);
}

/** The curves of the last step sent, as plain arrays. */
const live = (fn: ReturnType<typeof vi.fn>) => JSON.parse(JSON.stringify(fn.mock.lastCall?.[0]));

const pointCount = (container: HTMLElement) => container.querySelectorAll(".point").length;
const input = () => screen.getByLabelText("Input") as HTMLInputElement;
const output = () => screen.getByLabelText("Output") as HTMLInputElement;

test("the curve of the engine is drawn with a handle per point", () => {
  const { container } = open();
  expect(pointCount(container)).toBe(2);
  // From the bottom left (0, 0) to the top right, through the middle at the sampled output.
  expect(container.querySelector(".curve")).toHaveAttribute("d", "M0,255 L127.5,127.5 L255,0");
});

test("a click on the graph adds a point there, as a step of the gesture", async () => {
  const { user, svg, onlive, onend, container } = open();
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: at(100, 180) });
  expect(live(onlive)[0]).toEqual([
    [0, 0],
    [100, 180],
    [255, 255],
  ]);
  // Only the composite curve changed.
  expect(live(onlive).slice(1)).toEqual(CURVES().slice(1));
  expect(pointCount(container)).toBe(3);
  expect(onend).toHaveBeenCalledOnce();
  // The new point is the selected one: its position is in Input and Output.
  expect([input().valueAsNumber, output().valueAsNumber]).toEqual([100, 180]);
});

test("dragging a point moves it, between its neighbours and inside the grid", async () => {
  const { user, svg, onlive, onend } = open();
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: at(100, 100) });
  onlive.mockClear();
  onend.mockClear();
  await drag(user, svg, [100, 100], [120, 90], [270, 90]);
  // Not past the last point (255), a unit short of it.
  expect(live(onlive)[0][1]).toEqual([254, 90]);
  expect(onlive.mock.calls.length).toBeGreaterThanOrEqual(2);
  expect(onend).toHaveBeenCalledOnce();
});

test("a point grabbed near where it is moves instead of adding another", async () => {
  const { user, svg, onlive, container } = open();
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: at(100, 100) });
  onlive.mockClear();
  // A few units away: the same point.
  await drag(user, svg, [105, 103], [105, 150]);
  expect(pointCount(container)).toBe(3);
  expect(live(onlive)[0][1]).toEqual([105, 150]);
});

test("the point of an end can move up and down but not out of the curve", async () => {
  const { user, svg, onlive } = open();
  await drag(user, svg, [0, 0], [-10, 60]);
  expect(live(onlive)[0][0]).toEqual([0, 60]);
});

test("a point dragged out of the grid is removed when released, an end of the curve never", async () => {
  const { user, svg, onlive, container } = open();
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: at(100, 100) });
  expect(pointCount(container)).toBe(3);
  await drag(user, svg, [100, 100], [100, 300]);
  expect(live(onlive)[0]).toEqual(LINE);
  expect(pointCount(container)).toBe(2);
  // Only two points left: dragged out of the grid, they stay.
  onlive.mockClear();
  await drag(user, svg, [0, 0], [-100, -100]);
  expect(pointCount(container)).toBe(2);
  expect(live(onlive)[0][0][0]).toBe(0);
});

test("a point dragged out of the grid and back in is kept", async () => {
  const { user, svg, container } = open();
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: at(100, 100) });
  await drag(user, svg, [100, 100], [100, 300], [110, 120]);
  expect(pointCount(container)).toBe(3);
});

test("a point is not added on a column already taken, nor beyond sixteen", async () => {
  const { user, svg, onlive, container } = open();
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: at(100, 100) });
  onlive.mockClear();
  // Same column as the point, too far above it to grab it.
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: at(100, 200) });
  expect(onlive).not.toHaveBeenCalled();
  for (let x = 10; x < 255; x += 16) {
    await user.pointer({ keys: "[MouseLeft]", target: svg, coords: at(x, 240) });
  }
  expect(pointCount(container)).toBe(16);
});

test("Input and Output wait for a point to be selected, then move it as one undoable change", async () => {
  const { user, svg, onlive, onapply } = open();
  expect(input()).toBeDisabled();
  expect(output()).toBeDisabled();
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: at(100, 100) });
  expect(input()).toBeEnabled();
  onlive.mockClear();
  await fireEvent.change(output(), { target: { value: "200" } });
  expect(live(onapply)[0]).toEqual([
    [0, 0],
    [100, 200],
    [255, 255],
  ]);
  await fireEvent.change(input(), { target: { value: "300" } });
  // Between its neighbours: a unit short of the end.
  expect(live(onapply)[0][1]).toEqual([254, 200]);
  expect(onlive).not.toHaveBeenCalled();
});

test("an emptied Input or Output field leaves the point where it was", async () => {
  // Regression: an empty field was read as 0 and sent the point to the bottom.
  const { user, svg, onapply } = open();
  await user.pointer({ keys: "[MouseLeft]", target: svg, coords: at(100, 100) });
  const shown = output().value;
  await fireEvent.change(output(), { target: { value: "" } });
  expect(onapply).not.toHaveBeenCalled();
  expect(output().value).toBe(shown);
});

test("the channel menu switches to the red, green and blue curves, which are edited on their own", async () => {
  const curves = CURVES();
  curves[1] = [
    [0, 0],
    [50, 70],
    [255, 255],
  ];
  const { user, svg, onlive, container } = open(curves);
  expect(pointCount(container)).toBe(2);
  await user.selectOptions(screen.getByLabelText("Channel"), "Red");
  expect(pointCount(container)).toBe(3);
  // The selection does not follow to another channel.
  expect(input()).toBeDisabled();
  expect(container.querySelector(".curve")).toHaveAttribute("d", "M0,255 L127.5,191.25 L255,0");
  await drag(user, svg, [50, 70], [60, 80]);
  const sent = live(onlive);
  expect(sent[1][1]).toEqual([60, 80]);
  expect(sent[0]).toEqual(LINE);
  expect(sent[2]).toEqual(LINE);
  expect(sent[3]).toEqual(LINE);
});

test("the points follow the curves when they change elsewhere, as after an undo", async () => {
  const { rerender, container } = open();
  expect(pointCount(container)).toBe(2);
  await rerender({
    curves: [
      [
        [0, 0],
        [30, 40],
        [90, 90],
        [255, 255],
      ],
      LINE,
      LINE,
      LINE,
    ],
    samples: SAMPLES,
    onlive: vi.fn(),
    onend: vi.fn(),
    onapply: vi.fn(),
  });
  expect(pointCount(container)).toBe(4);
});
