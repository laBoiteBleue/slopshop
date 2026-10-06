// In-app updates (ADR 0039): Help > Check for Updates, the quiet check after startup and the
// menu bar's notice, in builds that update themselves.
import { screen } from "@testing-library/svelte";
import { beforeEach, expect, test, vi } from "vitest";
import type { UpdateInfo } from "../../../src/lib/engine";
import { documentView, layer, menuLabels, open, respond, sent } from "./harness";

// The automatic check waits for the app's start; not in tests.
vi.mock("../../../src/lib/updates", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../src/lib/updates")>()),
  CHECK_DELAY: 0,
}));

const newer: UpdateInfo = { version: "0.2.0", currentVersion: "0.1.0", notes: "Faster." };

let updatable: boolean;
/** What the check finds, or the failure it throws. */
let found: UpdateInfo | null | { code: string; detail: string };
let failing: boolean;

respond("update_supported", () => updatable);
respond("update_check", () => {
  if (failing) throw found;
  return found;
});

beforeEach(() => {
  updatable = true;
  found = null;
  failing = false;
});

async function start(dirty = false) {
  const user = open({ ...documentView(1, "photo.jpg", [layer(1, "Photo")]), dirty });
  await screen.findByText("photo.jpg");
  return user;
}

/** The text of the message boxes shown (Tauri's dialog plugin). */
const messages = () => sent("plugin:dialog|message").map((args) => args.message);

test("builds that do not update themselves neither check nor offer it", async () => {
  updatable = false;
  const user = await start();
  await user.click(screen.getByRole("menuitem", { name: "Help" }));
  expect(menuLabels()).not.toContain("Check for Updates…");
  expect(sent("update_check")).toEqual([]);
});

test("a quiet check after startup shows a notice, which opens the Update dialog", async () => {
  found = newer;
  const user = await start();
  const notice = await screen.findByRole("button", { name: "Update 0.2.0" });
  expect(sent("update_check")).toHaveLength(1);
  expect(JSON.parse(localStorage.getItem("slopshop.updates") ?? "null").lastCheck).toBeTypeOf(
    "number",
  );
  // Nothing opened by itself.
  expect(screen.queryByRole("dialog", { name: /is available/ })).not.toBeInTheDocument();
  await user.click(notice);
  expect(screen.getByRole("dialog", { name: "SlopShop 0.2.0 is available" })).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Later" }));
  expect(screen.queryByRole("dialog", { name: /is available/ })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Update 0.2.0" })).toBeInTheDocument();
});

test("no automatic check when it is turned off, but the menu still offers it", async () => {
  localStorage.setItem("slopshop.updates", JSON.stringify({ automatic: false, lastCheck: null }));
  const user = await start();
  await user.click(screen.getByRole("menuitem", { name: "Help" }));
  expect(menuLabels()).toContain("Check for Updates…");
  expect(sent("update_check")).toEqual([]);
});

test("a failed automatic check says nothing", async () => {
  failing = true;
  found = { code: "network", detail: "offline" };
  await start();
  await vi.waitFor(() => expect(sent("update_check")).toHaveLength(1));
  await new Promise((resolve) => setTimeout(resolve, 0));
  expect(messages()).toEqual([]);
  expect(screen.queryByRole("button", { name: /^Update / })).not.toBeInTheDocument();
});

test("Help > Check for Updates says when this version is the latest", async () => {
  localStorage.setItem("slopshop.updates", JSON.stringify({ automatic: false, lastCheck: null }));
  const user = await start();
  await user.click(screen.getByRole("menuitem", { name: "Help" }));
  expect(menuLabels().slice(-3)).toEqual(["—", "Check for Updates…", "About SlopShop"]);
  await user.click(screen.getByRole("menuitem", { name: "Check for Updates…" }));
  await vi.waitFor(() => expect(messages()).toEqual(["SlopShop is up to date."]));
});

test("Help > Check for Updates opens the Update dialog when there is one", async () => {
  localStorage.setItem("slopshop.updates", JSON.stringify({ automatic: false, lastCheck: null }));
  found = newer;
  const user = await start();
  await user.click(screen.getByRole("menuitem", { name: "Help" }));
  await user.click(screen.getByRole("menuitem", { name: "Check for Updates…" }));
  expect(
    await screen.findByRole("dialog", { name: "SlopShop 0.2.0 is available" }),
  ).toBeInTheDocument();
});

test("installing asks about unsaved documents first, and stops if the user keeps them", async () => {
  found = newer;
  const user = await start(true);
  await user.click(await screen.findByRole("button", { name: "Update 0.2.0" }));
  await user.click(screen.getByRole("button", { name: "Install and Restart" }));
  await vi.waitFor(() =>
    expect(messages()).toEqual([
      "Some documents have unsaved changes: photo.jpg. Install the update without saving them?",
    ]),
  );
  // The message box answered Cancel.
  expect(sent("update_install")).toEqual([]);
});

test("with every document saved, installing starts at once", async () => {
  found = newer;
  const user = await start();
  await user.click(await screen.findByRole("button", { name: "Update 0.2.0" }));
  await user.click(screen.getByRole("button", { name: "Install and Restart" }));
  await vi.waitFor(() => expect(sent("update_install")).toHaveLength(1));
  expect(messages()).toEqual([]);
});

test("Help > Check for Updates says why it failed", async () => {
  localStorage.setItem("slopshop.updates", JSON.stringify({ automatic: false, lastCheck: null }));
  failing = true;
  found = { code: "network", detail: "offline" };
  const user = await start();
  await user.click(screen.getByRole("menuitem", { name: "Help" }));
  await user.click(screen.getByRole("menuitem", { name: "Check for Updates…" }));
  await vi.waitFor(() =>
    expect(messages()).toEqual([
      "The update server could not be reached (offline). Check the connection and try again.",
    ]),
  );
});
