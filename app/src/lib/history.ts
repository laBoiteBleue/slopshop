// The History panel's names for the engine's history entries (ADR 0036): an entry says what it
// did by an identifier (`kind`) and, for an adjustment or a filter, which one (`detail`).

import type { HistoryEntryView } from "./engine";
import en, { type MessageKey } from "./i18n/en";

const known = (key: string): key is MessageKey => key in en;

/** What `entry` is called: a catalog key, and the name it puts in `{name}` when it has one. */
export function historyName(entry: HistoryEntryView): { key: MessageKey; name?: MessageKey } {
  const detail =
    entry.detail === null
      ? null
      : entry.kind === "filter"
        ? `filter.${entry.detail}`
        : `adjustment.${entry.detail}`;
  if (detail !== null && known(detail)) {
    if (entry.kind === "filter" || entry.kind === "adjustment") {
      return { key: "history.named", name: detail };
    }
    if (entry.kind === "newAdjustmentLayer") {
      return { key: "history.newAdjustmentLayerOf", name: detail };
    }
    if (entry.kind === "adjustmentSettings") {
      return { key: "history.adjustmentSettingsOf", name: detail };
    }
  }
  const key = `history.${entry.kind}`;
  // An entry newer than the catalogs: named generically rather than by its identifier.
  return { key: known(key) ? key : "history.edit" };
}
