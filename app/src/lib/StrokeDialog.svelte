<script lang="ts" module>
  import type { MessageKey } from "./i18n/en";

  /** Where the band lies relative to the selection's outline (Photoshop's Location). */
  export type StrokeLocation = "inside" | "center" | "outside";

  /** Width in pixels, location and opacity in `[0, 1]`; the color is the app's. */
  export type StrokeSettings = { width: number; location: StrokeLocation; opacity: number };

  const LOCATIONS: { value: StrokeLocation; label: MessageKey }[] = [
    { value: "inside", label: "stroke.inside" },
    { value: "center", label: "stroke.center" },
    { value: "outside", label: "stroke.outside" },
  ];

  /** As Photoshop's Stroke dialog, the last settings come back (for the session). */
  let last: StrokeSettings = { width: 1, location: "center", opacity: 1 };
</script>

<script lang="ts">
  // Edit > Stroke, laid out as Photoshop's Stroke dialog: Width and Color, Location, Opacity;
  // OK and Cancel on the right. A band along the selection's outline painted on the active
  // layer (paint, ADR 0029: not editable afterwards; the editable stroke will be a layer of its
  // own). The color swatch opens the color picker. Enter applies, Esc cancels.
  import { onMount } from "svelte";
  import { t } from "./i18n/index.svelte";
  import { movable } from "./dialogDrag";
  import SliderField from "./SliderField.svelte";

  let {
    color,
    onpickcolor,
    onchoose,
    onclose,
  }: {
    /** `#rrggbb` sRGB. */
    color: string;
    /** The swatch was clicked: the app shows the color picker, then this dialog again. */
    onpickcolor: (settings: StrokeSettings) => void;
    onchoose: (settings: StrokeSettings) => void;
    onclose: () => void;
  } = $props();

  let width = $state(last.width);
  let location = $state(last.location);
  let opacity = $state(last.opacity);
  let dialog: HTMLDialogElement;

  function settings(): StrokeSettings {
    last = { width, location, opacity };
    return last;
  }

  onMount(() => {
    dialog.showModal();
    // Modal: the app's shortcuts must not act behind the dialog.
    const isolate = (e: KeyboardEvent) => e.stopPropagation();
    window.addEventListener("keydown", isolate, true);
    return () => window.removeEventListener("keydown", isolate, true);
  });

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (width >= 1) onchoose(settings());
  }
</script>

<dialog
  bind:this={dialog}
  aria-labelledby="stroke-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
>
  <header id="stroke-title" {@attach movable("stroke")}>{t("stroke.title")}</header>
  <form onsubmit={submit}>
    <div class="fields">
      <fieldset>
        <legend>{t("stroke.stroke")}</legend>
        <SliderField
          label={t("stroke.width")}
          bind:value={width}
          min={1}
          max={250}
          unit="px"
          log
          width={52}
        />
        <div class="color">
          <span class="label">{t("stroke.color")}</span>
          <button
            type="button"
            class="swatch"
            style:background={color}
            title={t("stroke.pickColor")}
            aria-label={t("stroke.pickColor")}
            onclick={() => onpickcolor(settings())}
          ></button>
        </div>
      </fieldset>
      <fieldset>
        <legend>{t("stroke.location")}</legend>
        <div class="locations">
          {#each LOCATIONS as entry (entry.value)}
            <label>
              <input type="radio" name="location" value={entry.value} bind:group={location} />
              {t(entry.label)}
            </label>
          {/each}
        </div>
      </fieldset>
      <fieldset>
        <legend>{t("stroke.blending")}</legend>
        <SliderField
          label={t("fillChoice.opacity")}
          bind:value={opacity}
          min={0}
          max={100}
          unit="%"
          factor={100}
          width={52}
        />
      </fieldset>
    </div>
    <div class="buttons">
      <!-- svelte-ignore a11y_autofocus -->
      <button type="submit" class="btn primary" autofocus>{t("sizeDialog.ok")}</button>
      <button type="button" class="btn" onclick={onclose}>{t("sizeDialog.cancel")}</button>
    </div>
  </form>
</dialog>

<style>
  dialog {
    width: 380px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
    background: var(--panel);
    color: var(--text);
    box-shadow: 0 10px 32px #0009;
  }

  dialog::backdrop {
    background: #00000055;
  }

  header {
    padding: 5px 10px;
    background: var(--panel-header);
    border-bottom: 1px solid var(--border-dark);
    font-weight: 600;
  }

  /* Photoshop's layout: the settings on the left, OK and Cancel stacked on the right. */
  form {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 14px;
    padding: 14px;
  }

  .fields {
    display: grid;
    gap: 10px;
    align-self: start;
  }

  fieldset {
    display: grid;
    gap: 8px;
    margin: 0;
    padding: 6px 10px 10px;
    border: 1px solid var(--border-dark);
    border-radius: 4px;
  }

  legend {
    padding: 0 4px;
    color: var(--text-muted);
  }

  .color {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .label {
    color: var(--text-muted);
  }

  .swatch {
    width: 40px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--border-dark);
    border-radius: 2px;
    cursor: pointer;
  }

  .locations {
    display: flex;
    gap: 12px;
  }

  .locations label {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .buttons {
    display: grid;
    align-content: start;
    gap: 6px;
  }

  .buttons .btn {
    min-width: 80px;
  }
</style>
