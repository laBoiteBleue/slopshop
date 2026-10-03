import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, screen, within } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { AiComponent } from "../../src/lib/engine";
import { setLocale } from "../../src/lib/i18n/index.svelte";
import PreferencesDialog from "../../src/lib/PreferencesDialog.svelte";

const MB = 1024 * 1024;
const license = {
  name: "Apache 2.0",
  url: "https://example.org/apache",
  commercial: true,
  accept: false,
};

function component(id: string, installed: boolean): AiComponent {
  return {
    id,
    downloadSize: 100 * MB,
    installedSize: 150 * MB,
    installed,
    licenses: [license],
  };
}

/** What the engine says is installed, and the commands it received. */
let components: AiComponent[] | null;
let calls: [string, Record<string, unknown> | undefined][];
let removal: unknown;

beforeEach(() => {
  localStorage.clear();
  components = [component("sam2.1-tiny", true), component("birefnet-lite", false)];
  calls = [];
  removal = undefined;
  mockIPC((cmd, payload) => {
    const args = payload as Record<string, unknown> | undefined;
    calls.push([cmd, args]);
    if (cmd === "ai_components") return components;
    if (cmd === "ai_remove") {
      if (removal) throw removal;
      components =
        components?.map((c) => (c.id === args?.id ? { ...c, installed: false } : c)) ?? null;
    }
  });
});

afterEach(() => {
  clearMocks();
  setLocale("en");
});

function open() {
  const onclose = vi.fn();
  render(PreferencesDialog, { onclose });
  return { onclose, user: userEvent.setup() };
}

/** The row of a component in the list. */
const row = (name: string) => within(screen.getByText(name).closest("li") as HTMLElement);

test("the language list switches the interface at once, and the choice is remembered", async () => {
  const { user } = open();
  expect(screen.getByRole("dialog", { name: "Preferences" })).toBeInTheDocument();
  await user.selectOptions(screen.getByRole("combobox", { name: "Language" }), "Français");
  expect(screen.getByRole("dialog", { name: "Préférences" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Fermer" })).toBeInTheDocument();
  expect(localStorage.getItem("slopshop.locale")).toBe("fr");
});

test("each AI component shows whether it is installed, its size and its licenses", async () => {
  open();
  expect(await screen.findByText("SAM 2.1 tiny: object selection")).toBeInTheDocument();
  expect(row("SAM 2.1 tiny: object selection").getByText(/Installed, 150 MB/)).toBeInTheDocument();
  expect(
    row("SAM 2.1 tiny: object selection").getByRole("button", { name: "Remove" }),
  ).toBeEnabled();
  expect(
    row("BiRefNet lite: select the subject").getByText(/Not installed, 100 MB to download/),
  ).toBeInTheDocument();
  expect(
    row("BiRefNet lite: select the subject").getByRole("button", { name: "Download…" }),
  ).toBeEnabled();
});

test("a license opens in the browser", async () => {
  const { user } = open();
  await screen.findByText("SAM 2.1 tiny: object selection");
  await user.click(
    row("SAM 2.1 tiny: object selection").getByRole("button", { name: "Apache 2.0" }),
  );
  expect(calls).toContainEqual(["ai_open_license", { url: "https://example.org/apache" }]);
});

test("Remove removes the component, and the list shows it as not installed", async () => {
  const { user } = open();
  await screen.findByText("SAM 2.1 tiny: object selection");
  await user.click(row("SAM 2.1 tiny: object selection").getByRole("button", { name: "Remove" }));
  expect(calls).toContainEqual(["ai_remove", { id: "sam2.1-tiny" }]);
  await vi.waitFor(() =>
    expect(row("SAM 2.1 tiny: object selection").getByText(/Not installed/)).toBeInTheDocument(),
  );
});

test("a removal that fails says why and keeps the list", async () => {
  removal = { code: "disk", detail: "access denied" };
  const { user } = open();
  await screen.findByText("SAM 2.1 tiny: object selection");
  await user.click(row("SAM 2.1 tiny: object selection").getByRole("button", { name: "Remove" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "The files could not be written: access denied",
  );
  expect(row("SAM 2.1 tiny: object selection").getByText(/Installed/)).toBeInTheDocument();
});

test("Download… asks first, and the list refreshes when the dialog is left", async () => {
  const { user } = open();
  await screen.findByText("BiRefNet lite: select the subject");
  await user.click(
    row("BiRefNet lite: select the subject").getByRole("button", { name: "Download…" }),
  );
  expect(screen.getByRole("dialog", { name: "Download AI components" })).toBeInTheDocument();
  // The engine has it by the time the user changes their mind: the list is read again.
  components = [component("sam2.1-tiny", true), component("birefnet-lite", true)];
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog", { name: "Download AI components" })).not.toBeInTheDocument();
  await vi.waitFor(() =>
    expect(row("BiRefNet lite: select the subject").getByText(/Installed/)).toBeInTheDocument(),
  );
});

test("a platform without AI says so, with nothing to download", async () => {
  components = null;
  open();
  expect(
    await screen.findByText("AI features are not available on this system yet."),
  ).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Download…" })).not.toBeInTheDocument();
});

test("Close and Escape close it, but not behind the download dialog", async () => {
  const { onclose, user } = open();
  await screen.findByText("BiRefNet lite: select the subject");
  await user.click(screen.getByRole("button", { name: "Close" }));
  expect(onclose).toHaveBeenCalledOnce();
  await user.click(
    row("BiRefNet lite: select the subject").getByRole("button", { name: "Download…" }),
  );
  const [preferences] = screen.getAllByRole("dialog", { hidden: true });
  preferences.dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(onclose).toHaveBeenCalledOnce();
});
