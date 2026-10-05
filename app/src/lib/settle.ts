// Requests for what changes many times a second while the user drags (a layer's thumbnail
// while a slider of its stack moves: each new state is a whole layer for the engine to
// evaluate): sent once the value rests, one at a time.

import { latestWins } from "./latest";

/** A sender of `send(value)`, one at a time and the latest value waiting meanwhile (as
 * `latestWins`), each value sent once it has stayed `delay` ms without another one, or at once
 * when pushed with `now`. `drop()` forgets what waits. */
export function onceSettled<T>(delay: number, send: (value: T) => Promise<unknown>) {
  const latest = latestWins(send);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const stop = () => {
    clearTimeout(timer);
    timer = undefined;
  };
  return {
    push(value: T, now = false) {
      stop();
      if (now) {
        latest.push(value);
        return;
      }
      timer = setTimeout(() => {
        timer = undefined;
        latest.push(value);
      }, delay);
    },
    drop() {
      stop();
      latest.drop();
    },
  };
}
