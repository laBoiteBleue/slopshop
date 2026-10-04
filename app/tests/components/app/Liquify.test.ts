// Filter > Liquify (ADR 0037) in the app: its menu entry and shortcut on the active pixel layer,
// its workspace's OK and Cancel, and the Liquify entry of a layer's stack edited again from the
// Layers panel.
import { screen, within } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { LayerView, LiquifyState } from "../../../src/lib/engine";
import { documentView, layer, open, respond, row, sent } from "./harness";

const session: LiquifyState = {
  width: 400,
  height: 300,
  canUndo: false,
  canRedo: false,
  changed: false,
  displaced: false,
};

respond("liquify_open", () => session);
respond("liquify_commit", (_, doc) => ({ ...doc, revision: doc.revision + 1 }));
respond("liquify_close", () => null);

const liquifyEntry = {
  kind: "liquify" as const,
  adjustment: null,
  count: 1,
  hidden: false,
  steps: [],
  filter: null,
  filterSteps: [],
};

async function menu(layers: LayerView[] = [layer(1, "Photo")]) {
  const user = open(documentView(1, "photo.jpg", layers));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  return user;
}

test("Filter > Liquify… opens its workspace on the active layer, with the shortcut shown", async () => {
  const user = await menu();
  const item = screen.getByRole("menuitem", { name: /Liquify…/ });
  expect(item).not.toHaveAttribute("aria-disabled", "true");
  expect(item).toHaveTextContent(/Ctrl\+Shift\+X|Shift\+Ctrl\+X|⇧⌘X/);
  await user.click(item);
  await screen.findByRole("dialog", { name: "Liquify" });
  expect(sent("liquify_open")).toEqual([{ documentId: 1, layerId: 1, index: null }]);
  // Nothing of the document changes meanwhile.
  expect(sent("perform")).toHaveLength(0);
  expect(sent("perform_live")).toHaveLength(0);
});

test("Shift+Ctrl+X opens it too", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.keyboard("{Control>}{Shift>}x{/Shift}{/Control}");
  await screen.findByRole("dialog", { name: "Liquify" });
  expect(sent("liquify_open")).toHaveLength(1);
});

test("it is grayed on a hidden layer, as the other filters", async () => {
  const hidden = { ...layer(1, "Photo"), visible: false };
  await menu([hidden]);
  expect(screen.getByRole("menuitem", { name: /Liquify…/ })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});

test("OK makes the workspace's field one entry: the commit goes to the engine, the workspace closes", async () => {
  const user = await menu();
  await user.click(screen.getByRole("menuitem", { name: /Liquify…/ }));
  await screen.findByRole("dialog", { name: "Liquify" });
  await vi.waitFor(() => expect(screen.getByRole("button", { name: "OK" })).toBeEnabled());
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(sent("liquify_commit")).toEqual([{ documentId: 1 }]));
  expect(screen.queryByRole("dialog", { name: "Liquify" })).toBeNull();
  expect(sent("liquify_close")).toHaveLength(0);
});

test("Cancel and Escape leave the document as it was: the engine closes the session, no commit", async () => {
  const user = await menu();
  await user.click(screen.getByRole("menuitem", { name: /Liquify…/ }));
  await screen.findByRole("dialog", { name: "Liquify" });
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  await vi.waitFor(() => expect(sent("liquify_close")).toEqual([{ documentId: 1 }]));
  expect(screen.queryByRole("dialog", { name: "Liquify" })).toBeNull();
  expect(sent("liquify_commit")).toHaveLength(0);
  // Again, by Escape.
  await user.keyboard("{Control>}{Shift>}x{/Shift}{/Control}");
  await screen.findByRole("dialog", { name: "Liquify" });
  // Escape, as a browser reports it on a modal dialog.
  screen
    .getByRole("dialog", { name: "Liquify" })
    .dispatchEvent(new Event("cancel", { cancelable: true }));
  await vi.waitFor(() => expect(sent("liquify_close")).toHaveLength(2));
  expect(sent("liquify_commit")).toHaveLength(0);
});

test("a Liquify entry is listed, edited again from its edit icon, and has an eye", async () => {
  const photo: LayerView = { ...layer(1, "Photo"), entries: [liquifyEntry] };
  const user = open(documentView(1, "photo.jpg", [photo]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(within(row("Photo")).getByRole("button", { expanded: false }));
  expect(screen.getByText("Liquify", { selector: ".entry-name" })).toBeInTheDocument();
  // Its eye is the stack's, like the other entries'.
  await user.click(screen.getByRole("button", { name: "Hide" }));
  expect(sent("perform").at(-1)?.edit).toEqual({
    kind: "setStackEntry",
    id: 1,
    index: 0,
    hidden: true,
  });
  // Its edit icon reopens the workspace on the entry.
  await user.click(screen.getByRole("button", { name: "Edit Settings…" }));
  await screen.findByRole("dialog", { name: "Liquify" });
  expect(sent("liquify_open")).toEqual([{ documentId: 1, layerId: 1, index: 0 }]);
  await vi.waitFor(() => expect(screen.getByRole("button", { name: "OK" })).toBeEnabled());
  await user.click(screen.getByRole("button", { name: "OK" }));
  await vi.waitFor(() => expect(sent("liquify_commit")).toHaveLength(1));
});

test("another dialog open, Liquify does not open over it", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Blur", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Gaussian Blur…" }));
  await screen.findByRole("dialog", { name: "Gaussian Blur" });
  await user.keyboard("{Control>}{Shift>}x{/Shift}{/Control}");
  expect(screen.queryByRole("dialog", { name: "Liquify" })).toBeNull();
  expect(sent("liquify_open")).toHaveLength(0);
});
