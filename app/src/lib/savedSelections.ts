// Saved selections (Select > Save Selection): named objects of the document. Their names are
// what the user sees; saving under a name already used replaces that selection.

import type { SavedSelectionView, SelectionMode } from "./engine";

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

/** A saved selection loaded into the image, and how (Shift adds, Alt subtracts, both intersect). */
export type CombinedRow = { id: number; mode: SelectionMode };

/**
 * What the image's selection was made of from the Selections panel: its rows, as long as the
 * selection is still the one they made (`key`); any other change makes it something else.
 */
export type Combination = { document: number; key: number | null; rows: CombinedRow[] };

/** The rows of `combination` still describing the selection `key` of `document`. */
export function combinedRows(
  combination: Combination | null,
  document: number,
  key: number | null,
): CombinedRow[] {
  const valid = combination?.document === document && combination.key === key && key !== null;
  return valid ? combination.rows : [];
}

/**
 * The rows once saved selection `id` is loaded by `mode` into the selection `key`: a new list
 * when it replaces, one more row when it combines (the same saved selection again replaces its
 * row, the last way wins).
 */
export function loadedRows(
  combination: Combination | null,
  document: number,
  key: number | null,
  id: number,
  mode: SelectionMode,
): CombinedRow[] {
  if (mode === "replace") return [{ id, mode }];
  const rows = combinedRows(combination, document, key).filter((row) => row.id !== id);
  return [...rows, { id, mode }];
}
