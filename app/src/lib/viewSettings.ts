// The View menu's settings remembered on this machine: Rulers and Snap. Hide Extras and Hide
// Panels are not: they hide things for a moment, and must not greet the next session hidden.

export type ViewSettings = { rulers: boolean; snap: boolean };

/** Rulers off and Snap on at first, as in Photoshop. */
export const DEFAULT_VIEW_SETTINGS: ViewSettings = { rulers: false, snap: true };

const STORAGE_KEY = "slopshop.view";
type Store = Pick<Storage, "getItem" | "setItem">;

/** The settings saved last, or the defaults (nothing saved, storage unavailable or corrupt). */
export function loadViewSettings(store?: Store): ViewSettings {
  try {
    const saved = JSON.parse((store ?? localStorage).getItem(STORAGE_KEY) ?? "null");
    const flag = (name: keyof ViewSettings) =>
      typeof saved?.[name] === "boolean" ? (saved[name] as boolean) : DEFAULT_VIEW_SETTINGS[name];
    return { rulers: flag("rulers"), snap: flag("snap") };
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
