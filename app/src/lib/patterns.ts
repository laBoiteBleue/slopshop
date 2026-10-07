// Patterns (ADR 0042): how the library's patterns are named and what a new one is called.
import type { PatternEntry } from "./engine";
import type { MessageKey } from "./i18n/en";

/** The generated patterns' identifiers (`slopshop_io::patterns::BUILT_IN`). */
export const BUILT_IN = ["checkers", "stripes", "dots", "noise"] as const;

/** What the user knows a pattern by: theirs, or the generated one's name (translated). */
export function patternName(entry: PatternEntry, t: (key: MessageKey) => string): string {
  const id = entry.id.startsWith("builtin:") ? entry.id.slice("builtin:".length) : null;
  if (id && (BUILT_IN as readonly string[]).includes(id)) {
    return t(`patterns.builtin.${id}` as MessageKey);
  }
  return entry.name || entry.id;
}

/** "Pattern n", the first `n` no pattern of the library is named. */
export function nextPatternName(entries: PatternEntry[], named: (n: number) => string): string {
  const taken = new Set(entries.map((e) => e.name));
  let n = 1;
  while (taken.has(named(n))) n++;
  return named(n);
}
