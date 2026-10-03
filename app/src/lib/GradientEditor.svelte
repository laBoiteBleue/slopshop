<script lang="ts">
  // Gradient Map's gradient editor (a simple version of Photoshop's): the gradient, its color
  // stops below it. Click under the gradient to add a stop, drag one to move it (between its
  // neighbours), drag it down away from the bar to remove it; Color, Location and Delete edit
  // the selected one. The logic is in `gradient.ts`.
  import { hexToSrgb, srgbToHex } from "./color";
  import {
    LOCATIONS,
    MIN_STOPS,
    addStop,
    cssGradient,
    moveStop,
    recolorStop,
    removeStop,
    type Stop,
  } from "./gradient";
  import { t } from "./i18n/index.svelte";

  let {
    stops,
    onlive,
    onend,
    onapply,
  }: {
    stops: number[][];
    /** A step of a drag (part of one undoable gesture). */
    onlive: (stops: Stop[]) => void;
    onend: () => void;
    /** One undoable change. */
    onapply: (stops: Stop[]) => void;
  } = $props();

  /** How far below the stops (in pixels) a dragged stop is removed. */
  const REMOVE_DISTANCE = 24;

  /** The stops shown: the adjustment's, or the ones being dragged. */
  let local = $state<Stop[]>([]);
  let dragging = $state(false);
  $effect(() => {
    const current = stops;
    if (!dragging) local = current.map((s) => [s[0], s[1], s[2], s[3]] as Stop);
  });
  let selected = $state<number | null>(null);
  /** The dragged stop is away from the bar: released there, it goes. */
  let removing = $state(false);
  let track: HTMLDivElement;

  const current = $derived(selected !== null ? (local[selected] ?? null) : null);

  /** The location under the pointer (not clamped). */
  function locationAt(e: PointerEvent): number {
    const r = track.getBoundingClientRect();
    return ((e.clientX - r.left) / Math.max(r.width, 1)) * LOCATIONS;
  }

  function onTrackDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const added = addStop(local, locationAt(e));
    if (!added) return;
    local = added.stops;
    selected = added.index;
    onapply(local);
  }

  function onStopDown(e: PointerEvent, index: number) {
    if (e.button !== 0) return;
    e.stopPropagation();
    selected = index;
    dragging = true;
    removing = false;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onStopMove(e: PointerEvent) {
    if (!dragging || selected === null) return;
    const r = track.getBoundingClientRect();
    removing = e.clientY - r.bottom > REMOVE_DISTANCE && local.length > MIN_STOPS;
    if (removing) return;
    local = moveStop(local, selected, locationAt(e));
    onlive(local);
  }

  function onStopUp() {
    if (!dragging) return;
    if (removing && selected !== null) {
      const next = removeStop(local, selected);
      if (next) {
        local = next;
        onlive(local);
        selected = null;
      }
    }
    dragging = false;
    removing = false;
    onend();
  }

  function onColor(hex: string) {
    if (selected === null) return;
    local = recolorStop(
      local,
      selected,
      hexToSrgb(hex).map((c) => Math.round(c * 255)),
    );
    onapply(local);
  }

  function onLocation(input: HTMLInputElement) {
    const percent = input.value.trim() === "" ? NaN : Number(input.value);
    if (selected === null || !Number.isFinite(percent)) {
      if (current) input.value = String(Math.round((current[0] / LOCATIONS) * 100));
      return;
    }
    local = moveStop(local, selected, (percent / 100) * LOCATIONS);
    onapply(local);
  }

  function deleteSelected() {
    if (selected === null) return;
    const next = removeStop(local, selected);
    if (!next) return;
    local = next;
    selected = null;
    onapply(local);
  }
</script>

<div class="gradient">
  <div class="bar" style:background={cssGradient(local)} aria-hidden="true"></div>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    bind:this={track}
    class="track"
    class:removing
    title={t("adjustment.gradientMap.addStop")}
    onpointerdown={onTrackDown}
  >
    {#each local as stop, i (i)}
      <button
        type="button"
        class="stop"
        class:selected={i === selected}
        style:left="{(stop[0] / LOCATIONS) * 100}%"
        style:--color="rgb({stop[1]}, {stop[2]}, {stop[3]})"
        aria-label={t("adjustment.gradientMap.stop", {
          n: i + 1,
          location: Math.round((stop[0] / LOCATIONS) * 100),
        })}
        aria-pressed={i === selected}
        onpointerdown={(e) => onStopDown(e, i)}
        onpointermove={onStopMove}
        onpointerup={onStopUp}
        onpointercancel={onStopUp}
        onclick={() => (selected = i)}
        onkeydown={(e) => {
          if (e.key === "Delete" || e.key === "Backspace") {
            e.stopPropagation();
            deleteSelected();
          }
        }}
      ></button>
    {/each}
  </div>
  <div class="edit">
    <label class="label" for="gradient-color">{t("adjustment.gradientMap.color")}</label>
    <input
      id="gradient-color"
      type="color"
      disabled={current === null}
      value={current
        ? srgbToHex([current[1], current[2], current[3]].map((c) => c / 255))
        : "#000000"}
      onchange={(e) => onColor(e.currentTarget.value)}
    />
    <label class="label" for="gradient-location">{t("adjustment.gradientMap.location")}</label>
    <input
      id="gradient-location"
      type="number"
      min="0"
      max="100"
      disabled={current === null}
      value={current ? Math.round((current[0] / LOCATIONS) * 100) : ""}
      onchange={(e) => onLocation(e.currentTarget)}
    />
    <button
      type="button"
      class="btn small"
      disabled={current === null || local.length <= MIN_STOPS}
      onclick={deleteSelected}
    >
      {t("adjustment.gradientMap.delete")}
    </button>
  </div>
</div>

<style>
  .gradient {
    grid-column: 1 / -1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .bar {
    height: 22px;
    border: 1px solid var(--border-dark);
    border-radius: 2px;
  }

  .track {
    position: relative;
    height: 16px;
    margin: 0 5px;
    cursor: copy;
    touch-action: none;
  }

  .track.removing {
    cursor: not-allowed;
  }

  .stop {
    position: absolute;
    top: 0;
    width: 10px;
    height: 14px;
    margin-left: -5px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 0 0 2px 2px;
    background: var(--color);
    box-shadow: inset 0 0 0 1px #fff6;
    cursor: ew-resize;
  }

  .stop.selected {
    outline: 2px solid var(--accent);
    outline-offset: 0;
  }

  .edit {
    display: grid;
    grid-template-columns: auto 1fr auto 1fr auto;
    align-items: center;
    gap: 4px 6px;
    margin-top: 4px;
  }

  .edit input[type="number"] {
    min-width: 0;
  }

  .edit input[type="color"] {
    width: 100%;
    min-width: 0;
    height: 20px;
    padding: 0;
  }

  .label {
    color: var(--text-muted);
  }
</style>
