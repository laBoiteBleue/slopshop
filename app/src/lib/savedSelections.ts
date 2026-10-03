// Saved selections (Select > Save Selection): named objects of the document. Their names are
// what the user sees; saving under a name already used replaces that selection.

import type { SavedSelectionView } from "./engine";

/** The first of "Selection 1", "Selection 2"… (`format(n)`) that no saved selection uses. */
export function nextSelectionName(
  saved: SavedSelectionView[],
  format: (n: number) => string,
): string {
  const used = new Set(saved.map((s) => s.name));
  let n = 1;
  while (used.has(format(n))) n++;
  return format(n);
}

/** The saved selection that `name` (spaces around it aside) would replace, if any. */
export function savedNamed(saved: SavedSelectionView[], name: string): SavedSelectionView | null {
  const wanted = name.trim();
  return saved.find((s) => s.name === wanted) ?? null;
}
