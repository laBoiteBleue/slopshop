import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { onceSettled } from "../src/lib/settle";

beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
});

/** A sender whose requests finish when told, recording what was sent. */
function sender() {
  const sent: number[] = [];
  const pending: (() => void)[] = [];
  const send = (value: number) =>
    new Promise<void>((resolve) => {
      sent.push(value);
      pending.push(resolve);
    });
  const finish = async () => {
    pending.shift()?.();
    await vi.advanceTimersByTimeAsync(0);
  };
  return { sent, send, finish };
}

test("a value is sent once it has rested; the ones replaced before are never sent", async () => {
  const { sent, send } = sender();
  const thumbnails = onceSettled(200, send);
  thumbnails.push(1);
  await vi.advanceTimersByTimeAsync(150);
  thumbnails.push(2);
  await vi.advanceTimersByTimeAsync(150);
  thumbnails.push(3);
  await vi.advanceTimersByTimeAsync(199);
  expect(sent).toEqual([]);
  await vi.advanceTimersByTimeAsync(1);
  expect(sent).toEqual([3]);
});

test("a value pushed now is sent at once, and replaces one resting", async () => {
  const { sent, send } = sender();
  const thumbnails = onceSettled(200, send);
  thumbnails.push(1);
  thumbnails.push(2, true);
  expect(sent).toEqual([2]);
  await vi.advanceTimersByTimeAsync(500);
  expect(sent).toEqual([2]);
});

test("one request at a time: a value rested meanwhile waits for the one being sent", async () => {
  const { sent, send, finish } = sender();
  const thumbnails = onceSettled(200, send);
  thumbnails.push(1, true);
  thumbnails.push(2);
  await vi.advanceTimersByTimeAsync(200);
  thumbnails.push(3);
  await vi.advanceTimersByTimeAsync(200);
  expect(sent).toEqual([1]);
  await finish();
  expect(sent).toEqual([1, 3]);
});

test("a dropped value is not sent, resting or waiting", async () => {
  const { sent, send, finish } = sender();
  const thumbnails = onceSettled(200, send);
  thumbnails.push(1);
  thumbnails.drop();
  await vi.advanceTimersByTimeAsync(500);
  expect(sent).toEqual([]);
  thumbnails.push(2, true);
  thumbnails.push(3, true);
  thumbnails.drop();
  await finish();
  expect(sent).toEqual([2]);
});
