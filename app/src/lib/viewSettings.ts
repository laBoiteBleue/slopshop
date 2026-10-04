// The View menu's settings remembered on this machine: Rulers, their unit, and Snap. Hide
// Extras and Hide Panels are not: they hide things for a moment, and must not greet the next
// session hidden.

import { LENGTH_UNITS, type LengthUnit } from "./units";

export type ViewSettings = { rulers: boolean; rulerUnit: LengthUnit; snap: boolean };

/** Rulers off, in pixels, and Snap on at first, as in Photoshop. */
export const DEFAULT_VIEW_SETTINGS: ViewSettings = { rulers: false, rulerUnit: "px", snap: true };

const STORAGE_KEY = "slopshop.view";
type Store = Pick<Storage, "getItem" | "setItem">;

/** The settings saved last, or the defaults (nothing saved, storage unavailable or corrupt). */
export function loadViewSettings(store?: Store): ViewSettings {
  try {
    const saved = JSON.parse((store ?? localStorage).getItem(STORAGE_KEY) ?? "null");
    const flag = (name: "rulers" | "snap") =>
      typeof saved?.[name] === "boolean" ? (saved[name] as boolean) : DEFAULT_VIEW_SETTINGS[name];
    const unit = LENGTH_UNITS.includes(saved?.rulerUnit)
      ? (saved.rulerUnit as LengthUnit)
      : DEFAULT_VIEW_SETTINGS.rulerUnit;
    return { rulers: flag("rulers"), rulerUnit: unit, snap: flag("snap") };
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
