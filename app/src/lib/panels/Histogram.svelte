<script lang="ts" module>
  import type { HistogramChannel } from "../histogram";

  /** The channel shown, kept while the app runs (the panel is made again when unfolded). */
  let channel = $state<HistogramChannel>("colors");
</script>

<script lang="ts">
  // The dock's Histogram panel (ADR 0036): how many pixels of the visible image (within the
  // selection) have each value, as displayed, for the three colors over each other or one
  // channel, with Photoshop's statistics. Asked from the engine only while the panel shows,
  // again as the document changes (one request at a time while a slider moves).
  import { engine, type HistogramView } from "../engine";
  import { HISTOGRAM_CHANNELS, channelCounts, histogramPath, histogramStats } from "../histogram";
  import { t } from "../i18n/index.svelte";
  import { latestWins } from "../latest";
  import { panelContext } from "./context";

  const app = panelContext();
  let view = $state<HistogramView | null>(null);

  const fetcher = latestWins(async (documentId: number) => {
    const counts = await engine.histogram(documentId);
    if (documentId === app.doc.id) view = counts;
  });
  $effect(() => {
    void app.doc.revision;
    fetcher.push(app.doc.id);
  });
  $effect(() => () => fetcher.drop());

  const WIDTH = 256;
  const HEIGHT = 100;
  const COLORS = ["red", "green", "blue"] as const;
  const stats = $derived(view ? histogramStats(channelCounts(view, channel)) : null);
  /** Colors: the three drawn to the tallest count of any. */
  const colorScale = $derived(
    view ? Math.max(...COLORS.flatMap((c) => (view as HistogramView)[c])) : 0,
  );
  const format = (value: number, digits = 2) =>
    value.toLocaleString(undefined, { maximumFractionDigits: digits });
</script>

<section class="histogram" aria-label={t("histogram.title")}>
  <label class="channel">
    <span>{t("histogram.channel")}</span>
    <select bind:value={channel}>
      {#each HISTOGRAM_CHANNELS as id (id)}
        <option value={id}>{t(`histogram.${id}`)}</option>
      {/each}
    </select>
  </label>
  <svg
    class="graph"
    viewBox="0 0 {WIDTH} {HEIGHT}"
    preserveAspectRatio="none"
    role="img"
    aria-label={t("histogram.graph")}
  >
    {#if view}
      {#if channel === "colors"}
        {#each COLORS as color (color)}
          <path class="curve {color}" d={histogramPath(view[color], WIDTH, HEIGHT, colorScale)} />
        {/each}
      {:else}
        <path class="curve {channel}" d={histogramPath(view[channel], WIDTH, HEIGHT)} />
      {/if}
    {/if}
  </svg>
  {#if stats}
    <dl class="stats">
      <dt>{t("histogram.mean")}</dt>
      <dd>{format(stats.mean)}</dd>
      <dt>{t("histogram.stdDev")}</dt>
      <dd>{format(stats.stdDev)}</dd>
      <dt>{t("histogram.median")}</dt>
      <dd>{stats.median}</dd>
      <dt>{t("histogram.pixels")}</dt>
      <dd>
        {format((stats.pixels / (channel === "colors" ? 3 : 1)) * (view?.step ?? 1) ** 2, 0)}
      </dd>
    </dl>
    {#if (view?.step ?? 1) > 1}
      <p class="note">{t("histogram.sampled")}</p>
    {/if}
  {:else if view}
    <p class="note">{t("histogram.empty")}</p>
  {/if}
</section>

<style>
  .histogram {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 10px;
  }

  .channel {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .graph {
    width: 100%;
    height: 100px;
    background: var(--field);
    border: 1px solid var(--border-dark);
  }

  .curve {
    stroke: none;
    mix-blend-mode: screen;
  }

  .curve.red {
    fill: #ff4d4d;
  }

  .curve.green {
    fill: #4dff6a;
  }

  .curve.blue {
    fill: #4d7aff;
  }

  .curve.luminosity {
    fill: var(--text);
    mix-blend-mode: normal;
  }

  .stats {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 12px;
    margin: 0;
  }

  dt {
    color: var(--text-muted);
  }

  dd {
    margin: 0;
    font-variant-numeric: tabular-nums;
  }

  .note {
    margin: 0;
    color: var(--text-muted);
    font-size: 11px;
  }
</style>
