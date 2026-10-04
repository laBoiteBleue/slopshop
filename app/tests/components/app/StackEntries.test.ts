// A layer's stack entries edited again from the Layers panel (ADR 0034).
import { screen, within } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { ADJUSTMENT_PARAMS, type LayerView } from "../../../src/lib/engine";
import { documentView, layer, open, row, sent } from "./harness";

const padded = (values: number[]) => [
  ...values,
  ...Array<number>(ADJUSTMENT_PARAMS - values.length).fill(0),
];

/** A pixel layer with Hue/Saturation applied (hue 10), and the entry's edit dialog open. */
async function editing(hidden = false) {
  const photo: LayerView = {
    ...layer(1, "Photo"),
    entries: [
      {
        kind: "effect",
        adjustment: "hueSaturation",
        count: 1,
        hidden,
        steps: [
          {
            id: "hueSaturation",
            values: padded([10, 0, 0]),
            curves: null,
            curveSamples: null,
            gradient: null,
          },
        ],
      },
    ],
  };
  const user = open(documentView(1, "photo", [photo]));
  await screen.findByText("Photo", { selector: "li .name" });
  await user.click(within(row("Photo")).getByRole("button", { expanded: false }));
  await user.click(screen.getByRole("button", { name: "Edit Settings…" }));
  const dialog = await screen.findByRole("dialog", { name: "Hue/Saturation" });
  return { user, dialog };
}

const hue = () => screen.getByRole("spinbutton", { name: "Hue" });

test("the edit icon opens the entry's settings; the canvas follows, OK is one undo entry", async () => {
  const { user } = await editing();
  expect(hue()).toHaveValue(10);
  await user.clear(hue());
  await user.type(hue(), "30");
  await user.tab();
  const steps = [{ adjustment: "hueSaturation", values: padded([30, 0, 0]) }];
  expect(sent("perform_live").at(-1)?.edit).toEqual({
    kind: "setStackEntry",
    id: 1,
    index: 0,
    hidden: false,
    steps,
  });
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(sent("replace_gesture").at(-1)?.edit).toEqual({
    kind: "setStackEntry",
    id: 1,
    index: 0,
    hidden: false,
    steps,
  });
  expect(screen.queryByRole("dialog")).toBeNull();
});

test("Preview off hides the entry meanwhile; Cancel takes everything back", async () => {
  const { user } = await editing();
  await user.click(screen.getByRole("checkbox", { name: "Preview" }));
  expect(sent("perform_live").at(-1)?.edit).toMatchObject({ kind: "setStackEntry", hidden: true });
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(sent("cancel_gesture")).toHaveLength(1);
  expect(sent("replace_gesture")).toHaveLength(0);
});

test("OK without a change leaves no undo entry; a hidden entry keeps its eye", async () => {
  const { user } = await editing(true);
  // Hidden: it shows while it is edited.
  expect(sent("perform_live").at(-1)?.edit).toMatchObject({ hidden: false });
  await user.click(screen.getByRole("button", { name: "OK" }));
  expect(sent("replace_gesture")).toHaveLength(0);
  expect(sent("cancel_gesture")).toHaveLength(1);
});
