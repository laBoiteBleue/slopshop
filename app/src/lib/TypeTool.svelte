<script lang="ts">
  // The Type tool (T, ADR 0041): a click on the image places a text there, or edits the text
  // layer clicked; what is typed goes to a hidden text field (so that dead keys and input
  // methods work), and the engine draws the text as it is typed, a caret blinking where the
  // field's caret is. Escape, Ctrl+Enter or the keypad's Enter commit (Photoshop's keys); Enter
  // starts a new line. In the text being typed, a click moves the caret, a drag selects, a
  // double click selects a word, a triple click a line (a fourth the paragraph, a fifth the
  // whole text), Shift+click extends the selection; the arrows, Home and End follow the lines
  // as they are drawn; Ctrl+A selects everything, Ctrl+C, Ctrl+X and Ctrl+V copy, cut and
  // paste. A click outside the text commits it. The selection is drawn from the engine's
  // layout (parley's carets and lines).
  import { t } from "./i18n/index.svelte";
  import { hasShortcutModifier, shortcutLetter } from "./platform";
  import {
    clickSelection,
    countClick,
    lineBoundary,
    moveByLine,
    selectionQuads,
    type Clicks,
    type TextCommand,
    type TextLayout,
  } from "./textEditing";
  import { caretAt, caretIndexAt, commitsText } from "./typeText";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    editing,
    mac,
    onplace,
    oninput,
    oncommit,
    onselect,
    oncopy,
    onpaste,
  }: {
    mapping: ViewMapping;
    /**
     * The text being typed: its string, the engine's layout for it (document pixels), and
     * where the caret starts (UTF-16 units; absent: at the end).
     */
    editing: { key: number; content: string; layout: TextLayout; start?: number } | null;
    mac: boolean;
    /** A click on the image (document pixels) while nothing is typed. */
    onplace: (point: [number, number]) => void;
    /** The text typed changed. */
    oninput: (content: string) => void;
    oncommit: () => void;
    /** The selection changed (UTF-16 units, `start <= end`). */
    onselect?: (start: number, end: number) => void;
    /** Characters copied or cut: for the system clipboard. */
    oncopy: (text: string) => void;
    /** A paste: the system clipboard's text, if any. */
    onpaste: () => Promise<string | null>;
  } = $props();

  let field = $state<HTMLTextAreaElement | null>(null);
  /** The field's selection: where it started (`anchor`) and where it ends (`focus`). */
  let anchor = $state(0);
  let focus = $state(0);
  /** How far along the line Up and Down moves started (kept through successive moves). */
  let goal: number | null = null;
  /** The presses counted for double and triple clicks. */
  let clicks: Clicks | null = null;
  /** A drag selecting from `anchor` (its pointer). */
  let dragging: number | null = null;

  // A new text being typed: the field takes the keys, its caret where the text was clicked.
  let started: number | null = null;
  $effect(() => {
    const key = editing?.key ?? null;
    if (key === started) return;
    started = key;
    goal = null;
    clicks = null;
    if (key === null || !field) return;
    field.value = editing?.content ?? "";
    const at = editing?.start ?? field.value.length;
    select(at, at);
    field.focus({ preventScroll: true });
  });

  // Selections made by the system (keys the field handles itself, as Shift+Left).
  $effect(() => {
    if (!editing) return;
    const changed = () => {
      if (field && document.activeElement === field) track();
    };
    document.addEventListener("selectionchange", changed);
    return () => document.removeEventListener("selectionchange", changed);
  });

  /** Select from `from` to `to` (the caret at `to`), in the field and on the image. */
  function select(from: number, to: number) {
    if (!field) return;
    const length = field.value.length;
    anchor = Math.min(Math.max(from, 0), length);
    focus = Math.min(Math.max(to, 0), length);
    field.setSelectionRange(
      Math.min(anchor, focus),
      Math.max(anchor, focus),
      focus < anchor ? "backward" : "forward",
    );
    onselect?.(Math.min(anchor, focus), Math.max(anchor, focus));
  }

  /** The field's selection, after it changed it itself. */
  function track() {
    if (!field) return;
    const { selectionStart: start, selectionEnd: end, selectionDirection } = field;
    const backward = selectionDirection === "backward";
    const [a, f] = backward ? [end, start] : [start, end];
    if (a === anchor && f === focus) return;
    anchor = a;
    focus = f;
    onselect?.(start, end);
  }

  function pointAt(e: PointerEvent): [number, number] {
    return mapping.toDocument(e.clientX, e.clientY);
  }

  function press(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    const point = pointAt(e);
    if (!editing) {
      onplace(point);
      return;
    }
    const { carets } = editing.layout;
    // Within a line's height of the text: in it.
    const slack = Math.max(0, ...carets.map(([, y0, , y1]) => Math.abs(y1 - y0)));
    const index = caretIndexAt(carets, point, slack);
    if (index === null) {
      clicks = null;
      oncommit();
      return;
    }
    goal = null;
    field?.focus({ preventScroll: true });
    if (e.shiftKey) {
      select(anchor, index);
    } else {
      clicks = countClick(clicks, e.timeStamp, [e.clientX, e.clientY]);
      const content = field?.value ?? editing.content;
      const [from, to] = clickSelection(editing.layout, content, index, clicks.count);
      select(from, to);
      if (clicks.count > 1) return;
    }
    dragging = e.pointerId;
    (e.currentTarget as Element).setPointerCapture?.(e.pointerId);
  }

  function move(e: PointerEvent) {
    if (dragging !== e.pointerId || !editing) return;
    const index = caretIndexAt(editing.layout.carets, pointAt(e), Infinity);
    if (index !== null && index !== focus) select(anchor, index);
  }

  function release(e: PointerEvent) {
    if (dragging === e.pointerId) dragging = null;
  }

  /** The selected characters. */
  function selected(): string {
    if (!field) return "";
    return field.value.slice(field.selectionStart, field.selectionEnd);
  }

  /** `text` in place of the selection, the caret after it. */
  function replaceSelection(text: string) {
    if (!field) return;
    field.setRangeText(text, field.selectionStart, field.selectionEnd, "end");
    input();
  }

  /** Cut, Copy, Paste and Select All, from the keys or the Edit menu. */
  export function edit(command: TextCommand) {
    if (!field || !editing) return;
    switch (command) {
      case "selectAll":
        select(0, field.value.length);
        break;
      case "copy":
      case "cut": {
        const text = selected();
        if (text === "") return;
        oncopy(text);
        if (command === "cut") replaceSelection("");
        break;
      }
      case "paste": {
        const key = editing.key;
        void onpaste().then((text) => {
          // Line breaks of other systems as the text's.
          if (text && editing?.key === key) replaceSelection(text.replace(/\r\n?/g, "\n"));
        });
        break;
      }
    }
    field.focus({ preventScroll: true });
  }

  /** What Ctrl+letter does in the text, as in every text field. */
  const LETTER_COMMANDS: Record<string, TextCommand> = {
    a: "selectAll",
    c: "copy",
    x: "cut",
    v: "paste",
  };

  function keydown(e: KeyboardEvent) {
    // The text's keys stay the text's (the app's shortcuts wait for the commit).
    e.stopPropagation();
    if (commitsText(e, mac)) {
      e.preventDefault();
      oncommit();
      return;
    }
    const letter = shortcutLetter(e);
    if (hasShortcutModifier(e) && !e.altKey && !e.shiftKey && letter && LETTER_COMMANDS[letter]) {
      e.preventDefault();
      edit(LETTER_COMMANDS[letter]);
      return;
    }
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    const layout = editing?.layout;
    if (plain && layout && field && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
      // Up and Down by the lines drawn (wrapped or not), not the field's.
      e.preventDefault();
      const moved = moveByLine(layout, field.value, focus, e.key === "ArrowUp" ? -1 : 1, goal);
      goal = moved.goal;
      select(e.shiftKey ? anchor : moved.index, moved.index);
      return;
    }
    goal = null;
    if (plain && layout && field && (e.key === "Home" || e.key === "End")) {
      e.preventDefault();
      const to = lineBoundary(layout, field.value, focus, e.key === "End");
      select(e.shiftKey ? anchor : to, to);
    }
  }

  function input() {
    if (!field) return;
    track();
    oninput(field.value);
  }

  const toView = (x: number, y: number) => mapping.toViewport(x, y);

  const collapsed = $derived(anchor === focus);

  const shown = $derived.by(() => {
    const at = editing ? caretAt(editing.layout.carets, focus) : null;
    if (!at) return null;
    const [x0, y0] = toView(at[0], at[1]);
    const [x1, y1] = toView(at[2], at[3]);
    return { x0, y0, x1, y1 };
  });

  /** The selection's highlight, one polygon per line, in viewport pixels. */
  const highlight = $derived.by(() => {
    if (!editing || collapsed) return [];
    return selectionQuads(editing.layout, anchor, focus).map((quad) =>
      quad.map(([x, y]) => toView(x, y).join(",")).join(" "),
    );
  });
</script>

<!-- Full-size: it takes the presses on the image while the tool is active. -->
<svg
  class="type-tool"
  class:hand={mapping.hand}
  role="presentation"
  onpointerdown={press}
  onpointermove={move}
  onpointerup={release}
  onpointercancel={release}
>
  {#each highlight as points, i (i)}
    <polygon class="selected" {points} />
  {/each}
  {#if shown && collapsed}
    <line class="caret" x1={shown.x0} y1={shown.y0} x2={shown.x1} y2={shown.y1} />
  {/if}
</svg>
{#if editing}
  <textarea
    bind:this={field}
    class="type-field"
    aria-label={t("tools.type")}
    wrap="off"
    spellcheck="false"
    autocomplete="off"
    style:left={`${shown?.x0 ?? 0}px`}
    style:top={`${shown?.y0 ?? 0}px`}
    oninput={input}
    onkeydown={keydown}
    onkeyup={track}
    onselect={track}
    onpointerup={track}></textarea>
{/if}

<style>
  .type-tool {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    cursor: text;
  }

  /* Space held: the viewport pans. */
  .type-tool.hand {
    pointer-events: none;
  }

  .caret {
    stroke: var(--accent, #3b82f6);
    stroke-width: 1.5;
    animation: blink 1s steps(1) infinite;
    pointer-events: none;
  }

  /* Photoshop inverts the selected characters; a translucent highlight shows them as well
     without reading the pixels back. */
  .selected {
    fill: var(--accent, #3b82f6);
    fill-opacity: 0.35;
    pointer-events: none;
  }

  @keyframes blink {
    50% {
      opacity: 0;
    }
  }

  /* Out of sight but focusable, at the caret so that input methods show their window there. */
  .type-field {
    position: absolute;
    width: 1px;
    height: 1em;
    padding: 0;
    border: 0;
    opacity: 0;
    resize: none;
    overflow: hidden;
    pointer-events: none;
  }
</style>
