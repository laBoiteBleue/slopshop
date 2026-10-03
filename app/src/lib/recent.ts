/** The last part of a path (Windows or POSIX separators). */
export function baseName(path: string): string {
  const parts = path.split(/[\/]/).filter((part) => part !== "");
  return parts.at(-1) ?? path;
}

/**
 * How File > Open Recent and the welcome page name each entry: its file name, with its folder
 * when two entries share a name.
 */
export function recentLabels(paths: string[]): string[] {
  const names = paths.map(baseName);
  return paths.map((path, i) => {
    if (names.filter((name) => name === names[i]).length < 2) return names[i];
    const parts = path.split(/[\/]/).filter((part) => part !== "");
    const folder = parts.at(-2);
    return folder ? `${names[i]} — ${folder}` : names[i];
  });
}
