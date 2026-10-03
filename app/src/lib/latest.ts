// Live previews that cost the engine some time (Select > Modify, Select and Mask): one request
// at a time, and while it runs only the latest value waits; the ones in between are skipped.

/** A sender of `send(value)`, one at a time, the latest value waiting meanwhile. `drop()`
 * forgets the waiting value (the panel closed). */
export function latestWins<T>(send: (value: T) => Promise<unknown>) {
  let busy = false;
  let waiting: { value: T } | null = null;
  const push = (value: T) => {
    if (busy) {
      waiting = { value };
      return;
    }
    busy = true;
    void send(value)
      .catch(() => undefined)
      .finally(() => {
        busy = false;
        const next = waiting;
        waiting = null;
        if (next) push(next.value);
      });
  };
  return {
    push,
    drop: () => {
      waiting = null;
    },
  };
}
