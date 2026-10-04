// The Help menu: the shortcuts list, the project's pages (opened by name, the engine knows the
// addresses) and About.
import { screen } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import { documentView, layer, menuLabels, open, respond, sent } from "./harness";

respond("app_info", () => ({
  version: "1.2.3",
  license: "GPL-3.0-only",
  repository: "https://github.com/example/slopshop",
}));

async function helpMenu() {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("photo.jpg");
  await user.click(screen.getByRole("menuitem", { name: "Help" }));
  return user;
}

test("the Help menu's entries", async () => {
  await helpMenu();
  expect(menuLabels()).toEqual([
    "Keyboard Shortcuts",
    "—",
    "Report a Bug",
    "Contribute to SlopShop",
    "—",
    "About SlopShop",
  ]);
});

test("Keyboard Shortcuts shows the list Edit > Keyboard Shortcuts shows", async () => {
  const user = await helpMenu();
  await user.click(screen.getByRole("menuitem", { name: "Keyboard Shortcuts" }));
  expect(screen.getByRole("dialog", { name: "Keyboard Shortcuts" })).toBeInTheDocument();
});

test("Report a Bug and Contribute open the project's pages in the browser", async () => {
  const user = await helpMenu();
  await user.click(screen.getByRole("menuitem", { name: "Report a Bug" }));
  expect(sent("open_project_page")).toEqual([{ page: "newIssue" }]);
  await user.click(screen.getByRole("menuitem", { name: "Help" }));
  await user.click(screen.getByRole("menuitem", { name: "Contribute to SlopShop" }));
  expect(sent("open_project_page")).toEqual([{ page: "newIssue" }, { page: "contributing" }]);
});

test("About SlopShop shows the version the engine reports", async () => {
  const user = await helpMenu();
  await user.click(screen.getByRole("menuitem", { name: "About SlopShop" }));
  await vi.waitFor(() =>
    expect(screen.getByRole("dialog", { name: "About SlopShop" })).toBeInTheDocument(),
  );
  expect(screen.getByText("Version 1.2.3 (pre-alpha)")).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(screen.queryByRole("dialog", { name: "About SlopShop" })).not.toBeInTheDocument();
});
