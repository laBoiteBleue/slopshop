import { fireEvent, screen, waitFor } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { documentView, layer, open, respond, sent } from "./harness";

// The Eyedropper (I) and Alt held with the Brush: a click takes the color shown.

respond("sample_color", (args) => ((args.x as number) < 100 ? [255, 0, 0] : [0, 0, 255]));
respond("sample_patch", () => new ArrayBuffer(13 * 13 * 4));

const swatch = (name: string) => screen.getByRole("button", { name });

async function openImage() {
  const user = open(documentView(1, "cat.jpg", [layer(1, "Cat"), layer(2, "Hat")]));
  await screen.findByText("cat.jpg");
  return user;
}

const overlay = () => document.querySelector(".eyedropper") as HTMLElement;

test("I picks the Eyedropper: a click takes the foreground color, Alt+click the background", async () => {
  const user = await openImage();
  await user.keyboard("i");
  expect(screen.getByRole("button", { name: "Eyedropper Tool" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  await fireEvent.pointerDown(overlay(), { button: 0, clientX: 50, clientY: 10 });
  await waitFor(() =>
    expect(swatch("Set foreground color").style.background).toBe("rgb(255, 0, 0)"),
  );
  expect(sent("sample_color").at(-1)).toEqual({
    documentId: 1,
    x: 50,
    y: 10,
    size: 1,
    layerId: null,
  });
  await fireEvent.pointerDown(overlay(), { button: 0, clientX: 150, clientY: 10, altKey: true });
  await waitFor(() =>
    expect(swatch("Set background color").style.background).toBe("rgb(0, 0, 255)"),
  );
  expect(swatch("Set foreground color").style.background).toBe("rgb(255, 0, 0)");
});

test("Sample Size and Current Layer are sent with the click", async () => {
  const user = await openImage();
  await user.keyboard("i");
  await user.selectOptions(
    screen.getByRole("combobox", { name: "Sample Size:" }),
    "3 by 3 Average",
  );
  await user.selectOptions(screen.getByRole("combobox", { name: "Sample:" }), "Current Layer");
  await fireEvent.pointerDown(overlay(), { button: 0, clientX: 50, clientY: 10 });
  // The active layer: the top one at first.
  await waitFor(() => expect(sent("sample_color").at(-1)).toMatchObject({ size: 3, layerId: 2 }));
});

test("Alt held with the Brush takes the foreground color, the Brush coming back on release", async () => {
  const user = await openImage();
  await user.keyboard("b");
  expect(overlay()).toBeNull();
  await user.keyboard("[AltLeft>]");
  await fireEvent.pointerDown(overlay(), { button: 0, clientX: 150, clientY: 10, altKey: true });
  await waitFor(() =>
    expect(swatch("Set foreground color").style.background).toBe("rgb(0, 0, 255)"),
  );
  expect(sent("paint_stroke")).toEqual([]);
  await user.keyboard("[/AltLeft]");
  expect(overlay()).toBeNull();
  // Not with the Eraser: Alt there is not the eyedropper.
  await user.keyboard("e");
  await user.keyboard("[AltLeft>]");
  expect(overlay()).toBeNull();
  await user.keyboard("[/AltLeft]");
});
