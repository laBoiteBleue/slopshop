import { expect, test } from "vitest";
import { latestWins } from "../src/lib/latest";

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
    await new Promise((r) => setTimeout(r, 0));
  };
  return { sent, send, finish };
}

test("one request at a time; only the latest value waits", async () => {
  const { sent, send, finish } = sender();
  const live = latestWins(send);
  live.push(1);
  live.push(2);
  live.push(3);
  expect(sent).toEqual([1]);
  await finish();
  expect(sent).toEqual([1, 3]);
  await finish();
  live.push(4);
  expect(sent).toEqual([1, 3, 4]);
});

test("a dropped value is not sent; a failed request lets the next one go", async () => {
  const sent: number[] = [];
  const live = latestWins(async (value: number) => {
    sent.push(value);
    throw new Error("engine");
  });
  live.push(1);
  live.push(2);
  live.drop();
  await new Promise((r) => setTimeout(r, 0));
  expect(sent).toEqual([1]);
  live.push(5);
  await new Promise((r) => setTimeout(r, 0));
  expect(sent).toEqual([1, 5]);
});
