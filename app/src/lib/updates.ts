// In-app updates (ADR 0039): whether the quiet check after startup is on, and when it last ran,
// remembered on this machine; at most one automatic check a day.

import { t } from "./i18n/index.svelte";
import type { MessageKey } from "./i18n/en";

export type UpdateSettings = {
  /** Check after startup (Edit > Preferences > Updates). */
  automatic: boolean;
  /** When the last automatic check succeeded (ms since the epoch), if ever. */
  lastCheck: number | null;
};

export const DEFAULT_UPDATE_SETTINGS: UpdateSettings = { automatic: true, lastCheck: null };

/** Time between two automatic checks. */
export const CHECK_INTERVAL = 24 * 60 * 60 * 1000;

/** Time after startup before the automatic check: the app's own start goes first. */
export const CHECK_DELAY = 10_000;

const STORAGE_KEY = "slopshop.updates";
type Store = Pick<Storage, "getItem" | "setItem">;

/** The settings saved last, or the defaults (nothing saved, storage unavailable or corrupt). */
export function loadUpdateSettings(store?: Store): UpdateSettings {
  try {
    const saved = JSON.parse((store ?? localStorage).getItem(STORAGE_KEY) ?? "null");
    return {
      automatic:
        typeof saved?.automatic === "boolean" ? saved.automatic : DEFAULT_UPDATE_SETTINGS.automatic,
      lastCheck: Number.isFinite(saved?.lastCheck) ? (saved.lastCheck as number) : null,
    };
  } catch {
    return { ...DEFAULT_UPDATE_SETTINGS };
  }
}

export function saveUpdateSettings(settings: UpdateSettings, store?: Store) {
  try {
    (store ?? localStorage).setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch {
    // Not remembered: it lasts for the session.
  }
}

/** Whether the automatic check should run now. A clock set back counts as a day passed. */
export function checkDue(settings: UpdateSettings, now: number): boolean {
  if (!settings.automatic) return false;
  if (settings.lastCheck === null) return true;
  const elapsed = now - settings.lastCheck;
  return elapsed < 0 || elapsed >= CHECK_INTERVAL;
}

/** Why a check or an install failed, from the engine. */
export type UpdateFailure = { code: string; detail: string };

const FAILURES: Record<string, MessageKey> = {
  network: "update.error.network",
  signature: "update.error.signature",
  exporting: "update.error.exporting",
  busy: "update.error.busy",
  unsupported: "update.error.unsupported",
};

/** A failed check or install, for the user; `null` when the user cancelled it. */
export function updateFailureMessage(error: unknown): string | null {
  const failure = error as Partial<UpdateFailure> | null;
  if (failure?.code === "cancelled") return null;
  const key = (failure?.code && FAILURES[failure.code]) || "update.error.failed";
  return t(key, { detail: failure?.detail ?? String(error) });
}
