<script lang="ts">
  // The Type tool (T, ADR 0041): a click on the image places a text there, or edits the text
  // layer clicked; what is typed goes to a hidden text field (so that dead keys and input
  // methods work), and the engine draws the text as it is typed, a caret blinking where the
  // field's caret is. Escape, Ctrl+Enter or the keypad's Enter commit (Photoshop's keys); Enter
  // starts a new line. A click on the image while a text is typed commits it.
  import { t } from "./i18n/index.svelte";
  import { caretAt, commitsText, type Caret } from "./typeText";
  import type { ViewMapping } from "./Viewport.svelte";

  let {
    mapping,
    editing,
    mac,
    onplace,
    oninput,
    oncommit,
  }: {
    mapping: ViewMapping;
    /** The text being typed: its string, and the engine's carets for it (document pixels). */
    editing: { key: number; content: string; carets: Caret[] } | null;
    mac: boolean;
    /** A click on the image (document pixels) while nothing is typed. */
    onplace: (point: [number, number]) => void;
    /** The text typed changed. */
    oninput: (content: string) => void;
    oncommit: () => void;
  } = $props();

  let field = $state<HTMLTextAreaElement | null>(null);
  /** The field's caret, in UTF-16 units. */
  let caret = $state(0);

  // A new text being typed: the field takes the keys, its caret at the end (Photoshop's, for
  // a text clicked; the place clicked comes with selecting characters).
  let started: number | null = null;
  $effect(() => {
    const key = editing?.key ?? null;
    if (key === started) return;
    started = key;
    if (key === null || !field) return;
    field.value = editing?.content ?? "";
    caret = field.value.length;
    field.setSelectionRange(caret, caret);
    field.focus({ preventScroll: true });
  });

  function press(e: PointerEvent) {
    if (e.button !== 0 || mapping.hand) return;
    e.preventDefault();
    if (editing) {
      oncommit();
      return;
    }
    onplace(mapping.toDocument(e.clientX, e.clientY));
  }

  function keydown(e: KeyboardEvent) {
    if (commitsText(e, mac)) {
      e.preventDefault();
      e.stopPropagation();
      oncommit();
      return;
    }
    // The text's keys stay the text's (the app's shortcuts wait for the commit).
    e.stopPropagation();
  }

  function track() {
    if (field) caret = field.selectionEnd;
  }

  function input() {
    if (!field) return;
    track();
    oninput(field.value);
  }

  const shown = $derived.by(() => {
    const at = editing ? caretAt(editing.carets, caret) : null;
    if (!at) return null;
    const [x0, y0] = mapping.toViewport(at[0], at[1]);
    const [x1, y1] = mapping.toViewport(at[2], at[3]);
    return { x0, y0, x1, y1 };
  });
</script>

<!-- Full-size: it takes the presses on the image while the tool is active. -->
<svg class="type-tool" class:hand={mapping.hand} role="presentation" onpointerdown={press}>
  {#if shown}
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
