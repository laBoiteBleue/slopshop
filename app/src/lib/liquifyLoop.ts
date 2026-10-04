// The Liquify workspace's traffic with the engine (ADR 0037): strokes are sent in pieces, one in
// flight at a time, the pointer's samples that came meanwhile merged into the next piece; frames
// are asked for after the strokes they show, the latest view only. Plain classes over functions
// the component gives, so that their order and merging are tested without the engine.

import type { LiquifyBrush, LiquifyStrokePiece, LiquifyToolId } from "./engine";

/** Sends strokes' pieces to the engine, in order, merging what accumulates while one is away. */
export class StrokeQueue {
  private queue: LiquifyStrokePiece[] = [];
  private sending = false;
  private waiting: (() => void)[] = [];
  /** The tool and brush of the stroke last begun. */
  private stroke: { tool: LiquifyToolId; brush: LiquifyBrush } | null = null;
  /** The stroke is under way: the pointer is down. */
  private down = false;

  constructor(
    private readonly send: (piece: LiquifyStrokePiece) => Promise<unknown>,
    /** Told after each piece is answered. */
    private readonly onsent: () => void = () => {},
  ) {}

  /** A stroke is under way (begun, not ended). */
  get active(): boolean {
    return this.down;
  }

  /** The pointer went down at layer point `point`. */
  begin(tool: LiquifyToolId, brush: LiquifyBrush, point: [number, number]) {
    this.stroke = { tool, brush };
    this.down = true;
    this.queue.push({ tool, brush, begin: true, points: [point], hold: 0, end: false });
    this.pump();
  }

  /** The pointer moved to layer point `point`. */
  move(point: [number, number]) {
    if (!this.down) return;
    const last = this.queue.at(-1);
    // Merged into what has not left yet, unless the pointer stayed put since its last sample.
    if (last && !last.end && last.hold === 0) last.points.push(point);
    else this.queue.push(this.piece({ points: [point] }));
    this.pump();
  }

  /** The pointer has stayed still for `seconds`. */
  hold(seconds: number) {
    if (!this.down || !(seconds > 0)) return;
    const last = this.queue.at(-1);
    if (last && !last.end) last.hold += seconds;
    else this.queue.push(this.piece({ hold: seconds }));
    this.pump();
  }

  /** The pointer went up. */
  end() {
    if (!this.down) return;
    this.down = false;
    const last = this.queue.at(-1);
    if (last && !last.end) last.end = true;
    else this.queue.push(this.piece({ end: true }));
    this.pump();
  }

  /** Settles once everything queued has been sent and answered. */
  idle(): Promise<void> {
    if (!this.sending && this.queue.length === 0) return Promise.resolve();
    return new Promise((resolve) => this.waiting.push(resolve));
  }

  private piece(part: Partial<LiquifyStrokePiece>): LiquifyStrokePiece {
    const stroke = this.stroke;
    if (!stroke) throw new Error("no stroke");
    return {
      tool: stroke.tool,
      brush: stroke.brush,
      begin: false,
      points: [],
      hold: 0,
      end: false,
      ...part,
    };
  }

  private pump() {
    if (this.sending) return;
    const piece = this.queue.shift();
    if (!piece) return;
    this.sending = true;
    void this.send(piece)
      .catch(() => undefined)
      .finally(() => {
        this.sending = false;
        this.onsent();
        if (this.queue.length > 0) {
          this.pump();
        } else {
          const done = this.waiting.splice(0);
          done.forEach((resolve) => resolve());
        }
      });
  }
}

/**
 * Draws frames as the view or the field change: one request at a time, the strokes sent first
 * (`settle`), and a change that came meanwhile asks for one more after it (never a backlog).
 */
export class FrameLoop<Frame> {
  private running = false;
  private dirty = false;
  private stopped = false;

  constructor(
    private readonly io: {
      /** Settles once the strokes so far are in the engine. */
      settle: () => Promise<unknown>;
      /** Asks for a frame of the view as it is now. */
      frame: () => Promise<Frame>;
      draw: (frame: Frame) => void;
      /** Waits for the next moment to draw (an animation frame). */
      next?: () => Promise<unknown>;
    },
  ) {}

  /** The view or the field changed: a frame is wanted. */
  invalidate() {
    this.dirty = true;
    if (!this.running && !this.stopped) void this.run();
  }

  /** No more frames (the workspace closed). */
  stop() {
    this.stopped = true;
  }

  private async run() {
    this.running = true;
    try {
      while (this.dirty && !this.stopped) {
        this.dirty = false;
        if (this.io.next) await this.io.next();
        await this.io.settle();
        if (this.stopped) break;
        const frame = await this.io.frame();
        if (!this.stopped) this.io.draw(frame);
      }
    } catch {
      // A frame that could not be drawn: the next change asks again.
    } finally {
      this.running = false;
    }
  }
}
