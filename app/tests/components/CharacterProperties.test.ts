import { render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import CharacterProperties from "../../src/lib/CharacterProperties.svelte";
import type { TextDto } from "../../src/lib/engine";

// A text layer's Character settings (ADR 0041): the characters selected, or the whole text.

function text(spans: TextDto["spans"] = []): TextDto {
  return {
    content: "Hello world",
    family: "Inter Variable",
    weight: 400,
    italic: false,
    // 12 pt at 144 ppi.
    size: 24,
    color: [1, 0, 0, 1],
    tracking: 0,
    leading: null,
    align: "left",
    position: [0, 0],
    spans,
  };
}

function show(t: TextDto, range: [number, number] | null = null) {
  const props = {
    text: t,
    range,
    ppi: 144,
    fonts: { families: ["Arial", "Inter Variable"], defaultFamily: "Inter Variable" },
    stylesOf: vi.fn(async () => [
      { weight: 400, italic: false },
      { weight: 700, italic: false },
      { weight: 400, italic: true },
    ]),
    onstyle: vi.fn(),
    onalign: vi.fn(),
    onpickcolor: vi.fn(),
  };
  const shown = render(CharacterProperties, props);
  return { props, user: userEvent.setup(), rerender: shown.rerender };
}

const field = (name: string) => screen.getByRole("spinbutton", { name });
const combo = (name: string) => screen.getByRole("combobox", { name });

test("shows the whole text's settings: family, style, size and leading in points", async () => {
  show(text());
  expect(screen.getByRole("region", { name: "Character" })).toBeInTheDocument();
  expect(combo("Font family")).toHaveValue("Inter Variable");
  await waitFor(() => expect(combo("Font style")).toHaveValue("400"));
  expect(field("Font size")).toHaveValue(12);
  // Automatic leading: empty, "Auto" shown.
  expect(field("Leading")).toHaveValue(null);
  expect(field("Leading")).toHaveAttribute("placeholder", "Auto");
  expect(field("Tracking")).toHaveValue(0);
  expect(screen.getByRole("button", { name: "Left align text" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
});

test("shows the characters selected; what they do not share shows empty", async () => {
  const styled = text([{ start: 0, end: 5, style: { weight: 700, size: 48 } }]);
  show(styled, [0, 5]);
  await waitFor(() => expect(combo("Font style")).toHaveValue("700"));
  expect(field("Font size")).toHaveValue(24);
  // Across the span: weight and size differ.
  document.body.innerHTML = "";
  show(styled, [3, 8]);
  expect(field("Font size")).toHaveValue(null);
  expect(combo("Font style")).toHaveValue("");
  expect(combo("Font family")).toHaveValue("Inter Variable");
});

test("each setting sends a style of what it changes", async () => {
  const { props, user } = show(text());
  await user.selectOptions(combo("Font family"), "Arial");
  expect(props.onstyle).toHaveBeenLastCalledWith({ family: "Arial" });
  await waitFor(() => expect(combo("Font style")).toHaveValue("400"));
  await user.selectOptions(combo("Font style"), "Bold");
  expect(props.onstyle).toHaveBeenLastCalledWith({ weight: 700, italic: false });
  // Points to pixels at 144 ppi.
  await user.clear(field("Font size"));
  await user.type(field("Font size"), "20{Enter}");
  await user.tab();
  expect(props.onstyle).toHaveBeenLastCalledWith({ size: 40 });
  await user.type(field("Leading"), "15");
  await user.tab();
  expect(props.onstyle).toHaveBeenLastCalledWith({ leading: 30 });
  await user.clear(field("Tracking"));
  await user.type(field("Tracking"), "-50");
  await user.tab();
  expect(props.onstyle).toHaveBeenLastCalledWith({ tracking: -50 });
  await user.click(screen.getByRole("button", { name: "Center text" }));
  expect(props.onalign).toHaveBeenCalledWith("center");
  await user.click(screen.getByRole("button", { name: "Text color" }));
  expect(props.onpickcolor).toHaveBeenCalled();
});

test("leading emptied goes back to automatic", async () => {
  const { props, user } = show({ ...text(), leading: 30 });
  expect(field("Leading")).toHaveValue(15);
  await user.clear(field("Leading"));
  await user.tab();
  expect(props.onstyle).toHaveBeenLastCalledWith({ leading: null });
});

test("typing does not ask the family's styles again", async () => {
  const { props, rerender } = show(text());
  await waitFor(() => expect(props.stylesOf).toHaveBeenCalledTimes(1));
  await rerender({ text: { ...text(), content: "Hello world!" } });
  await new Promise((resolve) => setTimeout(resolve, 20));
  expect(props.stylesOf).toHaveBeenCalledTimes(1);
  await rerender({ text: { ...text(), content: "Hello world!!", family: "Arial" } });
  await waitFor(() => expect(props.stylesOf).toHaveBeenCalledTimes(2));
  expect(props.stylesOf).toHaveBeenLastCalledWith("Arial");
});

const button = (name: string) => screen.getByRole("button", { name });

test("Photoshop's style buttons show what the characters are and toggle it", async () => {
  const styled = text([
    { start: 0, end: 5, style: { underline: true, baseline: "superscript" } },
    { start: 6, end: 11, style: { fauxBold: true } },
  ]);
  const { props, user } = show(styled, [0, 5]);
  const order = [
    "Faux Bold",
    "Faux Italic",
    "All Caps",
    "Small Caps",
    "Superscript",
    "Subscript",
    "Underline",
    "Strikethrough",
  ];
  for (const name of order) {
    const on = name === "Underline" || name === "Superscript";
    expect(button(name)).toHaveAttribute("aria-pressed", String(on));
  }
  expect(button("Standard Ligatures")).toHaveAttribute("aria-pressed", "true");
  await user.click(button("Underline"));
  expect(props.onstyle).toHaveBeenLastCalledWith({ underline: false });
  // Subscript replaces Superscript; Faux Bold on.
  await user.click(button("Subscript"));
  expect(props.onstyle).toHaveBeenLastCalledWith({ baseline: "subscript" });
  await user.click(button("Faux Bold"));
  expect(props.onstyle).toHaveBeenLastCalledWith({ fauxBold: true });
  await user.click(button("Standard Ligatures"));
  expect(props.onstyle).toHaveBeenLastCalledWith({ ligatures: false });
});

test("a style button shows released over characters set apart, and sets them all", async () => {
  const styled = text([{ start: 0, end: 5, style: { fauxBold: true } }]);
  const { props, user } = show(styled, [3, 8]);
  expect(button("Faux Bold")).toHaveAttribute("aria-pressed", "false");
  await user.click(button("Faux Bold"));
  expect(props.onstyle).toHaveBeenLastCalledWith({ fauxBold: true });
});

test("at a caret, the character before it with the style pending there", () => {
  const styled = text([{ start: 0, end: 5, style: { fauxItalic: true } }]);
  render(CharacterProperties, {
    text: styled,
    range: [5, 5],
    pending: { strikethrough: true, size: 48 },
    ppi: 144,
    fonts: null,
    stylesOf: vi.fn(async () => []),
    onstyle: vi.fn(),
    onalign: vi.fn(),
    onpickcolor: vi.fn(),
  });
  expect(button("Faux Italic")).toHaveAttribute("aria-pressed", "true");
  expect(button("Strikethrough")).toHaveAttribute("aria-pressed", "true");
  expect(field("Font size")).toHaveValue(24);
});
