import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import AboutDialog from "../../src/lib/AboutDialog.svelte";

let calls: [string, Record<string, unknown> | undefined][];

beforeEach(() => {
  calls = [];
  mockIPC((cmd, payload) => {
    calls.push([cmd, payload as Record<string, unknown> | undefined]);
    return null;
  });
});

afterEach(() => {
  clearMocks();
});

function open() {
  const onclose = vi.fn();
  render(AboutDialog, {
    info: {
      version: "1.2.3",
      license: "GPL-3.0-only",
      repository: "https://github.com/example/slopshop",
    },
    onclose,
  });
  return { onclose, user: userEvent.setup() };
}

/** What the dialog says for a term. */
const definition = (term: string) =>
  screen.getByText(term, { selector: "dt" }).nextElementSibling as HTMLElement;

test("it shows the name, the version as built, the license and the project", () => {
  open();
  expect(screen.getByRole("dialog", { name: "About SlopShop" })).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "SlopShop" })).toBeInTheDocument();
  expect(screen.getByText("Version 1.2.3 (pre-alpha)")).toBeInTheDocument();
  expect(screen.getByText(/open-source/)).toBeInTheDocument();
  expect(definition("License")).toHaveTextContent("GPL-3.0-only");
  expect(definition("Project")).toHaveTextContent("github.com/example/slopshop");
});

test("the project's link asks the engine to open its page, by name", async () => {
  const { user, onclose } = open();
  await user.click(screen.getByRole("button", { name: "github.com/example/slopshop" }));
  expect(calls).toContainEqual(["open_project_page", { page: "home" }]);
  expect(onclose).not.toHaveBeenCalled();
});

test("OK and Escape close it, and the app's keys wait meanwhile", async () => {
  const { user, onclose } = open();
  const appKeys = vi.fn();
  window.addEventListener("keydown", appKeys);
  await user.keyboard("z");
  window.removeEventListener("keydown", appKeys);
  expect(appKeys).not.toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "OK" }));
  screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(onclose).toHaveBeenCalledTimes(2);
});
