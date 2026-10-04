// The Filter menu (ADR 0034): filters on the active pixel layer, Repeat (Ctrl+F), and filter
// entries edited again from the Layers panel.
import { screen, within } from "@testing-library/svelte";
import { expect, test, vi } from "vitest";
import type { LayerView } from "../../../src/lib/engine";
import { documentView, layer, open, row, sent } from "./harness";

async function blurDialog(layers: LayerView[] = [layer(1, "Photo")]) {
  const user = open(documentView(1, "photo.jpg", layers));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Blur", { selector: ".label" }));
  await user.click(screen.getByRole("menuitem", { name: "Gaussian Blur…" }));
  await screen.findByRole("dialog", { name: "Gaussian Blur" });
  return user;
}

const radius = () => screen.getByRole("spinbutton", { name: "Radius" });

test("the Filter menu: Repeat and its settings grayed until a filter is applied, then Blur", async () => {
  const user = open(documentView(1, "photo.jpg", [layer(1, "Photo")]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  expect(screen.getByRole("menuitem", { name: /Repeat Last Filter/ })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  expect(screen.getByRole("menuitem", { name: /Last Filter Settings/ })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  expect(screen.getByText("Blur", { selector: ".label" })).toBeInTheDocument();
});

test("Gaussian Blur shows live on the active layer, OK is one undo entry, Ctrl+F repeats it", async () => {
  const user = await blurDialog();
  await vi.waitFor(() =>
    expect(sent("perform_live").at(-1)).toEqual({
      documentId: 1,
      edit: { kind: "applyFilter", id: 1, filter: "gaussianBlur", values: [1] },
      replace: true,
    }),
  );
  await user.clear(radius());
  await user.type(radius(), "6");
  await vi.waitFor(() => expect(sent("perform_live").at(-1)?.edit).toMatchObject({ values: [6] }));
  await user.click(screen.getByRole("button", { name: "OK" }));
  const applied = { kind: "applyFilter", id: 1, filter: "gaussianBlur", values: [6] };
  expect(sent("replace_gesture").at(-1)?.edit).toEqual(applied);
  // Repeat: the same filter, a new entry.
  await user.keyboard("{Control>}f{/Control}");
  await vi.waitFor(() => expect(sent("perform").at(-1)?.edit).toEqual(applied));
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  expect(screen.getByRole("menuitem", { name: /Repeat Gaussian Blur/ })).not.toHaveAttribute(
    "aria-disabled",
    "true",
  );
});

test("Preview off takes the blur off the canvas; Cancel leaves no undo entry", async () => {
  const user = await blurDialog();
  await user.click(screen.getByRole("checkbox", { name: "Preview" }));
  await vi.waitFor(() => expect(sent("cancel_gesture").length).toBeGreaterThan(0));
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(sent("replace_gesture")).toHaveLength(0);
});

test("filters are grayed on a hidden layer", async () => {
  const hidden = { ...layer(1, "Photo"), visible: false };
  const user = open(documentView(1, "photo.jpg", [hidden]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(screen.getByRole("menuitem", { name: "Filter" }));
  await user.hover(screen.getByText("Blur", { selector: ".label" }));
  expect(screen.getByRole("menuitem", { name: "Gaussian Blur…" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
});

test("a filter entry is edited again with its icon: its radius, live, then one undo entry", async () => {
  const photo: LayerView = {
    ...layer(1, "Photo"),
    entries: [
      {
        kind: "filter",
        adjustment: null,
        count: 1,
        hidden: false,
        steps: [],
        filter: "gaussianBlur",
        filterSteps: [{ id: "gaussianBlur", values: [3] }],
      },
    ],
  };
  const user = open(documentView(1, "photo.jpg", [photo]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(within(row("Photo")).getByRole("button", { expanded: false }));
  expect(screen.getByText("Gaussian Blur", { selector: ".entry-name" })).toBeInTheDocument();
  await user.click(screen.getByRole("button", { name: "Edit Settings…" }));
  await screen.findByRole("dialog", { name: "Gaussian Blur" });
  expect(radius()).toHaveValue(3);
  await user.clear(radius());
  await user.type(radius(), "8");
  const edit = {
    kind: "setStackEntry",
    id: 1,
    index: 0,
    hidden: false,
    filters: [{ filter: "gaussianBlur", values: [8] }],
  };
  await vi.waitFor(() => expect(sent("perform_live").at(-1)?.edit).toEqual(edit));
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(sent("replace_gesture").at(-1)?.edit).toEqual(edit);
});
