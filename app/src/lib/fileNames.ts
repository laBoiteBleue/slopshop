// File names of Open and Save As: which format a path names, and the name an export proposes.

/** Files that open through the vector import dialog (ADR 0021): PDF and SVG. */
export function isVectorPath(path: string): boolean {
  return /\.(pdf|svgz?)$/i.test(path);
}

/** The format among `formats` whose extensions include `path`'s, ignoring case. */
export function formatOfPath<F extends string>(
  path: string,
  formats: Record<F, { extensions: readonly string[] }>,
): F | null {
  const extension = path.toLowerCase().split(".").pop() ?? "";
  const entries = Object.entries(formats) as [F, { extensions: readonly string[] }][];
  return entries.find(([, f]) => f.extensions.includes(extension))?.[0] ?? null;
}

/** `formats` in the order of the Save As file types: `last` (the last one used) first. */
export function formatOrder<F extends string>(formats: readonly F[], last: F): F[] {
  return [last, ...formats.filter((f) => f !== last)];
}

/** The document name with `extension`, without characters files cannot have. */
export function exportFileName(name: string, extension: string): string {
  const safe = name.replace(/[\\/:*?"<>|]/g, "_");
  const stem = safe.replace(/\.[^.]+$/, "") || safe;
  return `${stem}.${extension}`;
}
