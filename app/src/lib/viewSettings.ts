// The View menu's settings remembered on this machine: View > Snap. Hide Extras and Hide Panels
// are not: they hide things for a moment, and must not greet the next session hidden.

export type ViewSettings = { snap: boolean };

export const DEFAULT_VIEW_SETTINGS: ViewSettings = { snap: true };

const STORAGE_KEY = "slopshop.view";
type Store = Pick<Storage, "getItem" | "setItem">;

/** The settings saved last, or the defaults (nothing saved, storage unavailable or corrupt). */
export function loadViewSettings(store?: Store): ViewSettings {
  try {
    const saved = JSON.parse((store ?? localStorage).getItem(STORAGE_KEY) ?? "null");
    return {
      snap: typeof saved?.snap === "boolean" ? saved.snap : DEFAULT_VIEW_SETTINGS.snap,
    };
  } catch {
    return { ...DEFAULT_VIEW_SETTINGS };
  }
}

export function saveViewSettings(settings: ViewSettings, store?: Store) {
  try {
    (store ?? localStorage).setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // Not remembered: it lasts for the session.
  }
}
