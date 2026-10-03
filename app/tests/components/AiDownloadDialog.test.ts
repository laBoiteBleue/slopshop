import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, test, vi } from "vitest";
import AiDownloadDialog from "../../src/lib/AiDownloadDialog.svelte";
import type { AiComponent, AiProgress } from "../../src/lib/engine";

afterEach(() => clearMocks());

const MB = 1024 * 1024;

const permissive = {
  name: "Apache 2.0",
  url: "https://example.org/apache",
  commercial: true,
  accept: false,
};
const restricted = {
  name: "CC BY-NC 4.0",
  url: "https://example.org/nc",
  commercial: false,
  accept: true,
};

function component(id: string, changes: Partial<AiComponent> = {}): AiComponent {
  return {
    id,
    downloadSize: 100 * MB,
    installedSize: 150 * MB,
    installed: false,
    licenses: [permissive],
    ...changes,
  };
}

/** The commands the engine received, with their arguments. */
let calls: [string, Record<string, unknown> | undefined][] = [];

/** `install` answers (or not, to keep the download going) `ai_install`. */
function engineAnswers(
  install: (progress: (p: AiProgress) => void) => Promise<void> | void = () => {},
) {
  calls = [];
  mockIPC(async (cmd, payload) => {
    const args = payload as Record<string, unknown> | undefined;
    calls.push([cmd, args]);
    if (cmd === "ai_install") {
      const channel = args?.progress as { onmessage: (p: AiProgress) => void };
      await install((p) => channel.onmessage(p));
    }
  });
}

function open(components: AiComponent[]) {
  const ondone = vi.fn();
  const onclose = vi.fn();
  render(AiDownloadDialog, { components, ondone, onclose });
  return { ondone, onclose, user: userEvent.setup() };
}

const download = () => screen.getByRole("button", { name: /^(Download|Retry)$/ });

test("it lists what is missing with the sizes to download and to keep on disk", () => {
  engineAnswers();
  open([
    component("sam2.1-tiny"),
    component("birefnet-lite", { downloadSize: 50 * MB, installedSize: 60 * MB }),
    component("vitmatte-small", { installed: true }),
  ]);
  expect(screen.getByText("SAM 2.1 tiny: object selection")).toBeInTheDocument();
  expect(screen.getByText("BiRefNet lite: select the subject")).toBeInTheDocument();
  // Installed ones are skipped.
  expect(screen.queryByText(/ViTMatte/)).not.toBeInTheDocument();
  expect(screen.getByText("To download: 150 MB · On disk: 210 MB")).toBeInTheDocument();
});

test("a license that must be accepted keeps Download disabled until it is", async () => {
  engineAnswers();
  const { user } = open([component("sam2.1-tiny", { licenses: [restricted] })]);
  expect(screen.getByText("Non-commercial")).toBeInTheDocument();
  expect(download()).toBeDisabled();
  await user.click(screen.getByRole("checkbox", { name: "I accept the licenses above" }));
  expect(download()).toBeEnabled();
});

test("permissive licenses need no acceptance, and a license opens in the browser", async () => {
  engineAnswers();
  const { user } = open([component("sam2.1-tiny")]);
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(download()).toBeEnabled();
  await user.click(screen.getByRole("button", { name: "Apache 2.0" }));
  expect(calls).toContainEqual(["ai_open_license", { url: "https://example.org/apache" }]);
});

test("there is nothing to download when everything is installed", () => {
  engineAnswers();
  open([component("sam2.1-tiny", { installed: true })]);
  expect(download()).toBeDisabled();
});

test("Download installs the missing components, shows the progress, then reports done", async () => {
  let finish = () => {};
  engineAnswers(async (progress) => {
    progress({ done: 40 * MB, total: 150 * MB });
    await new Promise<void>((resolve) => (finish = resolve));
  });
  const { ondone, onclose, user } = open([
    component("sam2.1-tiny"),
    component("vitmatte-small", { installed: true }),
    component("birefnet-lite", { downloadSize: 50 * MB }),
  ]);
  await user.click(download());
  expect(calls.find(([cmd]) => cmd === "ai_install")?.[1]).toMatchObject({
    ids: ["sam2.1-tiny", "birefnet-lite"],
  });
  await vi.waitFor(() => expect(screen.getByText("40 MB of 150 MB")).toBeInTheDocument());
  expect(download()).toBeDisabled();
  // While downloading, Cancel stops the download (what was fetched is kept) and the dialog stays
  // until the install reports back.
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(calls.some(([cmd]) => cmd === "ai_cancel_install")).toBe(true);
  expect(onclose).not.toHaveBeenCalled();
  finish();
  await vi.waitFor(() => expect(ondone).toHaveBeenCalledOnce());
});

test("a failure is explained and Retry tries again; a cancelled download says nothing", async () => {
  let failure: unknown = { code: "network", detail: "timeout" };
  engineAnswers(() => {
    throw failure;
  });
  const { user } = open([component("sam2.1-tiny")]);
  await user.click(download());
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "The download failed (timeout). What was downloaded is kept: try again.",
  );
  expect(screen.getByRole("button", { name: "Retry" })).toBeEnabled();
  failure = { code: "cancelled", detail: "" };
  await user.click(screen.getByRole("button", { name: "Retry" }));
  await vi.waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
  expect(screen.getByRole("button", { name: "Download" })).toBeEnabled();
});

test("Cancel and Escape close it before a download starts", async () => {
  engineAnswers();
  const { onclose, user } = open([component("sam2.1-tiny")]);
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(onclose).toHaveBeenCalledTimes(2);
  expect(calls).toEqual([]);
});
