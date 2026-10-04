<script lang="ts">
  // The dock's History panel (ADR 0036): the document's steps, oldest first, under its initial
  // state. A click goes back (or forward) to the state after that step; the steps undone stay
  // listed, dimmed, until the next change. Asked from the engine only while the panel shows.
  import { engine, type HistoryEntryView, type HistoryView } from "../engine";
  import { historyName } from "../history";
  import { t } from "../i18n/index.svelte";
  import { latestWins } from "../latest";
  import { keepFocus } from "../platform";
  import { panelContext } from "./context";

  const app = panelContext();
  let history = $state<HistoryView | null>(null);
  let list = $state<HTMLElement>();

  // One request at a time: a slider being dragged changes the document many times a second.
  const fetcher = latestWins(async (documentId: number) => {
    const view = await engine.history(documentId);
    if (documentId === app.doc.id) history = view;
  });
  $effect(() => {
    void app.doc.revision;
    fetcher.push(app.doc.id);
  });
  $effect(() => () => fetcher.drop());

  // The current state stays in sight.
  $effect(() => {
    const done = history?.done;
    if (done === undefined || !list) return;
    list.querySelector(`[data-done="${done}"]`)?.scrollIntoView?.({ block: "nearest" });
  });

  function goTo(done: number) {
    if (history && done !== history.done) void app.sync(engine.goToHistory(app.doc.id, done));
  }

  function label(entry: HistoryEntryView): string {
    const name = historyName(entry);
    return t(name.key, name.name ? { name: t(name.name) } : undefined);
  }
</script>

<section class="history" aria-label={t("history.title")}>
  <ul bind:this={list}>
    <li>
      <button
        type="button"
        class="row"
        class:current={history?.done === 0}
        aria-current={history?.done === 0 ? "step" : undefined}
        data-done={0}
        onmousedown={keepFocus}
        onclick={() => goTo(0)}
      >
        {t("history.initial")}
      </button>
    </li>
    {#each history?.entries ?? [] as entry, index (index)}
      {@const done = index + 1}
      <li>
        <button
          type="button"
          class="row"
          class:current={history?.done === done}
          class:undone={done > (history?.done ?? 0)}
          aria-current={history?.done === done ? "step" : undefined}
          data-done={done}
          onmousedown={keepFocus}
          onclick={() => goTo(done)}
        >
          {label(entry)}
        </button>
      </li>
    {/each}
  </ul>
  <p class="hint">{t("history.hint")}</p>
</section>

<style>
  .history {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  ul {
    flex: 1 1 0;
    min-height: 0;
    margin: 0;
    padding: 2px 0;
    list-style: none;
    overflow-y: auto;
  }

  .row {
    display: block;
    width: 100%;
    padding: 3px 10px;
    border: none;
    border-radius: 0;
    background: transparent;
    color: var(--text);
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .row:hover {
    background: var(--hover);
  }

  .row.current {
    background: var(--selected);
    box-shadow: inset 2px 0 var(--accent);
  }

  .row.undone {
    color: var(--text-muted);
    font-style: italic;
  }

  .hint {
    flex: none;
    margin: 0;
    padding: 4px 10px 6px;
    border-top: 1px solid var(--border-dark);
    color: var(--text-muted);
    font-size: 11px;
  }
</style>
