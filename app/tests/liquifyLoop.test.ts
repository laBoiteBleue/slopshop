import { expect, test, vi } from "vitest";
import type { LiquifyBrush, LiquifyStrokePiece } from "../src/lib/engine";
import { FrameLoop, StrokeQueue } from "../src/lib/liquifyLoop";

const brush: LiquifyBrush = { size: 100, density: 50, pressure: 100, rate: 80 };

/** A sender whose answers the test releases one at a time. */
function sender() {
  const pieces: LiquifyStrokePiece[] = [];
  const releases: (() => void)[] = [];
  const send = (piece: LiquifyStrokePiece) => {
    pieces.push(structuredClone(piece));
    return new Promise<void>((resolve) => releases.push(resolve));
  };
  return {
    send,
    pieces,
    /** Answers the piece in flight. */
    answer: async () => {
      releases.shift()?.();
      await Promise.resolve();
      await Promise.resolve();
    },
  };
}

test("a stroke starts at once and its samples that came meanwhile go in the next piece", async () => {
  const s = sender();
  const queue = new StrokeQueue(s.send);
  queue.begin("forwardWarp", brush, [10, 10]);
  expect(queue.active).toBe(true);
  expect(s.pieces).toEqual([
    { tool: "forwardWarp", brush, begin: true, points: [[10, 10]], hold: 0, end: false },
  ]);
  // In flight: nothing else leaves, the samples wait together.
  queue.move([20, 10]);
  queue.move([30, 10]);
  queue.move([40, 10]);
  expect(s.pieces).toHaveLength(1);
  await s.answer();
  expect(s.pieces[1]).toEqual({
    tool: "forwardWarp",
    brush,
    begin: false,
    points: [
      [20, 10],
      [30, 10],
      [40, 10],
    ],
    hold: 0,
    end: false,
  });
  queue.end();
  await s.answer();
  // The end goes alone when nothing waited: the last piece had left already.
  expect(s.pieces[2]).toMatchObject({ begin: false, points: [], end: true });
  expect(queue.active).toBe(false);
});

test("the end joins the samples that wait, and the queue is idle once all are answered", async () => {
  const s = sender();
  const queue = new StrokeQueue(s.send);
  queue.begin("pucker", brush, [1, 1]);
  queue.move([2, 2]);
  queue.end();
  expect(s.pieces).toHaveLength(1);
  let idle = false;
  void queue.idle().then(() => (idle = true));
  await s.answer();
  expect(s.pieces[1]).toMatchObject({ points: [[2, 2]], end: true });
  await Promise.resolve();
  expect(idle).toBe(false);
  await s.answer();
  await Promise.resolve();
  expect(idle).toBe(true);
  // Nothing to wait for: settled at once.
  await queue.idle();
});

test("time held still adds up, after the samples it follows and before the ones that come next", async () => {
  const s = sender();
  const queue = new StrokeQueue(s.send);
  queue.begin("bloat", brush, [5, 5]);
  queue.move([6, 5]);
  queue.hold(0.25);
  queue.hold(0.5);
  queue.move([7, 5]);
  queue.move([8, 5]);
  await s.answer();
  expect(s.pieces[1]).toMatchObject({ points: [[6, 5]], hold: 0.75 });
  await s.answer();
  // The sample after the hold is in a piece of its own, in order.
  expect(s.pieces[2]).toMatchObject({
    points: [
      [7, 5],
      [8, 5],
    ],
    hold: 0,
  });
  // Holding with nothing waiting: a piece of its own, with no samples.
  await s.answer();
  queue.hold(0.03);
  expect(s.pieces[3]).toMatchObject({ points: [], hold: 0.03, begin: false });
});

test("nothing is sent outside a stroke, and a stroke that failed does not stop the next", async () => {
  const sent: LiquifyStrokePiece[] = [];
  let fail = true;
  const queue = new StrokeQueue((piece) => {
    sent.push(piece);
    return fail ? Promise.reject(new Error("no")) : Promise.resolve();
  });
  queue.move([1, 1]);
  queue.hold(1);
  queue.end();
  expect(sent).toHaveLength(0);
  expect(queue.active).toBe(false);
  queue.begin("smooth", brush, [0, 0]);
  queue.move([5, 5]);
  await queue.idle();
  fail = false;
  queue.end();
  await queue.idle();
  expect(sent.map((p) => [p.begin, p.end])).toEqual([
    [true, false],
    [false, false],
    [false, true],
  ]);
  // Zero or negative time is nothing.
  queue.begin("smooth", brush, [0, 0]);
  await queue.idle();
  const count = sent.length;
  queue.hold(0);
  queue.hold(-1);
  queue.hold(NaN);
  await queue.idle();
  expect(sent).toHaveLength(count);
});

test("each stroke carries the tool and the brush it began with", async () => {
  const sent: LiquifyStrokePiece[] = [];
  const queue = new StrokeQueue((piece) => {
    sent.push(structuredClone(piece));
    return Promise.resolve();
  });
  queue.begin("pushLeft", { ...brush, size: 30 }, [0, 0]);
  queue.move([1, 1]);
  queue.end();
  await queue.idle();
  queue.begin("freeze", { ...brush, size: 7 }, [0, 0]);
  queue.end();
  await queue.idle();
  expect(sent.map((p) => [p.tool, p.brush.size, p.begin])).toEqual([
    ["pushLeft", 30, true],
    ["pushLeft", 30, false],
    ["freeze", 7, true],
    ["freeze", 7, false],
  ]);
});

test("a frame is asked after the strokes are in, and a change meanwhile asks one more, once", async () => {
  const order: string[] = [];
  let release!: () => void;
  const settle = vi.fn(() => {
    order.push("settle");
    return Promise.resolve();
  });
  let n = 0;
  const frame = vi.fn(() => {
    const mine = ++n;
    order.push(`frame ${mine}`);
    return new Promise<number>((resolve) => {
      release = () => resolve(mine);
    });
  });
  const drawn: number[] = [];
  const loop = new FrameLoop<number>({ settle, frame, draw: (f) => drawn.push(f) });
  loop.invalidate();
  await vi.waitFor(() => expect(frame).toHaveBeenCalledTimes(1));
  // Changes while the frame is being made: one more frame, not one each.
  loop.invalidate();
  loop.invalidate();
  loop.invalidate();
  release();
  await vi.waitFor(() => expect(frame).toHaveBeenCalledTimes(2));
  release();
  await vi.waitFor(() => expect(drawn).toEqual([1, 2]));
  expect(order).toEqual(["settle", "frame 1", "settle", "frame 2"]);
  // Idle again: a new change wakes it.
  loop.invalidate();
  await vi.waitFor(() => expect(frame).toHaveBeenCalledTimes(3));
  release();
  await vi.waitFor(() => expect(drawn).toEqual([1, 2, 3]));
});

test("a stopped loop draws nothing more, and a failed frame does not stop it", async () => {
  let fail = true;
  const drawn: number[] = [];
  const loop = new FrameLoop<number>({
    settle: () => Promise.resolve(),
    frame: () => (fail ? Promise.reject(new Error("no")) : Promise.resolve(7)),
    draw: (f) => drawn.push(f),
  });
  loop.invalidate();
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(drawn).toEqual([]);
  fail = false;
  loop.invalidate();
  await vi.waitFor(() => expect(drawn).toEqual([7]));
  loop.stop();
  loop.invalidate();
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(drawn).toEqual([7]);
});

test("the loop waits for the next moment to draw when told how", async () => {
  const waits: string[] = [];
  const loop = new FrameLoop<number>({
    next: () => {
      waits.push("next");
      return Promise.resolve();
    },
    settle: () => {
      waits.push("settle");
      return Promise.resolve();
    },
    frame: () => Promise.resolve(1),
    draw: () => waits.push("draw"),
  });
  loop.invalidate();
  await vi.waitFor(() => expect(waits).toEqual(["next", "settle", "draw"]));
});
