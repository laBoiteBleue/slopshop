import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { UpdateInfo } from "../../src/lib/engine";
import UpdateDialog from "../../src/lib/UpdateDialog.svelte";

const update: UpdateInfo = {
  version: "0.2.0",
  currentVersion: "0.1.0",
  notes: "Faster brushes.\nIn-app updates.",
};

let calls: string[];
/** Settles the install the engine is running (it never resolves when it succeeds). */
let failInstall: ((failure: unknown) => void) | null;

beforeEach(() => {
  calls = [];
  failInstall = null;
  mockIPC((cmd) => {
    calls.push(cmd);
    if (cmd === "update_install") {
      return new Promise((_, reject) => (failInstall = reject));
    }
  });
});

afterEach(() => clearMocks());

function open(confirm = true) {
  const onclose = vi.fn();
  const confirmInstall = vi.fn(async () => confirm);
  render(UpdateDialog, { update, confirmInstall, onclose });
  return { onclose, confirmInstall, user: userEvent.setup() };
}

test("it shows the new version, the one installed and the release notes", () => {
  open();
  expect(screen.getByRole("dialog", { name: "SlopShop 0.2.0 is available" })).toBeInTheDocument();
  expect(screen.getByText("You have version 0.1.0.")).toBeInTheDocument();
  expect(screen.getByText(/Faster brushes\.\s+In-app updates\./)).toBeInTheDocument();
});

test("Install asks about unsaved documents, then downloads and installs", async () => {
  const { confirmInstall, user } = open();
  await user.click(screen.getByRole("button", { name: "Install and Restart" }));
  expect(confirmInstall).toHaveBeenCalledOnce();
  await vi.waitFor(() => expect(calls).toContain("update_install"));
  expect(screen.getByRole("button", { name: "Install and Restart" })).toBeDisabled();
});

test("nothing is installed when the user keeps their documents", async () => {
  const { user } = open(false);
  await user.click(screen.getByRole("button", { name: "Install and Restart" }));
  expect(calls).not.toContain("update_install");
  expect(screen.getByRole("button", { name: "Install and Restart" })).toBeEnabled();
});

test("a failed install says why, and can be tried again", async () => {
  const { user } = open();
  await user.click(screen.getByRole("button", { name: "Install and Restart" }));
  await vi.waitFor(() => expect(failInstall).not.toBeNull());
  failInstall?.({ code: "signature", detail: "bad signature" });
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "The downloaded update failed its signature check and was not installed (bad signature).",
  );
  expect(screen.getByRole("button", { name: "Install and Restart" })).toBeEnabled();
});

test("Later closes it; during the download, Cancel stops the download instead", async () => {
  const { onclose, user } = open();
  await user.click(screen.getByRole("button", { name: "Later" }));
  expect(onclose).toHaveBeenCalledOnce();
  await user.click(screen.getByRole("button", { name: "Install and Restart" }));
  await vi.waitFor(() => expect(calls).toContain("update_install"));
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(calls).toContain("update_cancel");
  expect(onclose).toHaveBeenCalledOnce();
  // The engine ends the download as cancelled: nothing to report.
  failInstall?.({ code: "cancelled", detail: "" });
  await vi.waitFor(() =>
    expect(screen.getByRole("button", { name: "Install and Restart" })).toBeEnabled(),
  );
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});
