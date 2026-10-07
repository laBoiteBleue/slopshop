import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import SourcesPanel from "../../src/lib/SourcesPanel.svelte";
import type { SourceView } from "../../src/lib/engine";

const photo: SourceView = { id: 7, name: "photo.jpg", width: 4000, height: 3000, layers: [1, 2] };
const pasted: SourceView = { id: 8, name: "", width: 200, height: 100, layers: [3] };

function open(sources = [photo, pasted], active: number | null = null) {
  const props = {
    documentId: 1,
    sources,
    name: (s: SourceView) => (s.name ? s.name : s.id === 8 ? "Pasted Layer" : ""),
    active,
    onselect: vi.fn(),
    onnewlayer: vi.fn(),
  };
  render(SourcesPanel, props);
  return { ...props, user: userEvent.setup() };
}

const row = (name: string) => screen.getByRole("option", { name: new RegExp(name) });

test("each source: its name, its size and how many layers show it; the active one marked", () => {
  open([photo, pasted], 7);
  expect(row("photo.jpg")).toHaveTextContent("4,000 × 3,000 · 2 layers");
  expect(row("photo.jpg")).toHaveAttribute("aria-selected", "true");
  // Pixels pasted have no name of their own: their first layer's.
  expect(row("Pasted Layer")).toHaveTextContent("200 × 100 · 1 layer");
  expect(row("Pasted Layer")).toHaveAttribute("aria-selected", "false");
});

test("a click selects the layers showing a source", async () => {
  const { onselect, user } = open();
  await user.click(row("photo.jpg"));
  expect(onselect).toHaveBeenCalledWith([1, 2]);
});

test("the right-click menu selects the layers or makes a new layer showing the source", async () => {
  const { onselect, onnewlayer, user } = open();
  await user.pointer({ keys: "[MouseRight]", target: row("Pasted Layer") });
  await user.click(screen.getByRole("menuitem", { name: "New Layer from Source" }));
  expect(onnewlayer).toHaveBeenCalledWith(pasted);
  await user.pointer({ keys: "[MouseRight]", target: row("photo.jpg") });
  await user.click(screen.getByRole("menuitem", { name: "Select Layers" }));
  expect(onselect).toHaveBeenCalledWith([1, 2]);
});

test("without sources, what will show there", () => {
  open([]);
  expect(screen.queryByRole("option")).not.toBeInTheDocument();
  expect(screen.getByText(/duplicated layers share theirs/)).toBeInTheDocument();
});
