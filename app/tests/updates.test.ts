// @vitest-environment jsdom
// (The messages go through the i18n module, which reads the browser's language.)
import { describe, expect, test } from "vitest";
import {
  CHECK_INTERVAL,
  DEFAULT_UPDATE_SETTINGS,
  checkDue,
  loadUpdateSettings,
  saveUpdateSettings,
  updateFailureMessage,
} from "../src/lib/updates";

/** A storage holding `value` under the settings' key. */
function store(value: string | null) {
  const items = new Map<string, string>();
  if (value !== null) items.set("slopshop.updates", value);
  return {
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, v: string) => void items.set(key, v),
  };
}

describe("update settings", () => {
  test("on at first, never checked", () => {
    expect(loadUpdateSettings(store(null))).toEqual({ automatic: true, lastCheck: null });
  });

  test("what was saved is read back", () => {
    const s = store(null);
    saveUpdateSettings({ automatic: false, lastCheck: 1234 }, s);
    expect(loadUpdateSettings(s)).toEqual({ automatic: false, lastCheck: 1234 });
  });

  test("odd values fall back to the defaults, field by field", () => {
    expect(loadUpdateSettings(store("{not json"))).toEqual(DEFAULT_UPDATE_SETTINGS);
    expect(loadUpdateSettings(store('{"automatic":"no","lastCheck":"yesterday"}'))).toEqual(
      DEFAULT_UPDATE_SETTINGS,
    );
    expect(loadUpdateSettings(store('{"automatic":false}'))).toEqual({
      automatic: false,
      lastCheck: null,
    });
  });

  test("a storage that throws is not fatal", () => {
    const broken = {
      getItem: () => {
        throw new Error("denied");
      },
      setItem: () => {
        throw new Error("denied");
      },
    };
    expect(loadUpdateSettings(broken)).toEqual(DEFAULT_UPDATE_SETTINGS);
    expect(() => saveUpdateSettings(DEFAULT_UPDATE_SETTINGS, broken)).not.toThrow();
  });
});

describe("the automatic check", () => {
  const now = 1_000_000_000_000;

  test("runs when never run, then at most once a day", () => {
    expect(checkDue({ automatic: true, lastCheck: null }, now)).toBe(true);
    expect(checkDue({ automatic: true, lastCheck: now - CHECK_INTERVAL + 1 }, now)).toBe(false);
    expect(checkDue({ automatic: true, lastCheck: now - CHECK_INTERVAL }, now)).toBe(true);
  });

  test("never runs when turned off", () => {
    expect(checkDue({ automatic: false, lastCheck: null }, now)).toBe(false);
  });

  test("a clock set back does not stop it for good", () => {
    expect(checkDue({ automatic: true, lastCheck: now + 1000 }, now)).toBe(true);
  });
});

describe("failure messages", () => {
  test("a known code is translated, with its detail", () => {
    expect(updateFailureMessage({ code: "network", detail: "timed out" })).toBe(
      "The update server could not be reached (timed out). Check the connection and try again.",
    );
    expect(updateFailureMessage({ code: "exporting", detail: "" })).toBe(
      "Exports are running: install the update once they have finished.",
    );
  });

  test("an unknown code or a plain error is a failure, with its text", () => {
    expect(updateFailureMessage({ code: "strange", detail: "oops" })).toBe(
      "The update failed: oops",
    );
    expect(updateFailureMessage("broken pipe")).toBe("The update failed: broken pipe");
  });

  test("a cancelled download says nothing", () => {
    expect(updateFailureMessage({ code: "cancelled", detail: "" })).toBeNull();
  });
});
