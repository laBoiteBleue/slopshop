import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, test, vi } from "vitest";
import type { VectorInfo } from "../../src/lib/engine";
import VectorImportDialog from "../../src/lib/VectorImportDialog.svelte";

beforeEach(() => localStorage.clear());

const LETTER = { width: 612, height: 792 };

/** A 4-page PDF of Letter pages, as the engine describes it. */
const PDF: VectorInfo = { kind: "pdf", defaultDpi: 300, pages: Array(4).fill(LETTER) };

/** An SVG: one 100 × 50 "page", at the 96 ppi of its own size. */
const SVG: VectorInfo = { kind: "svg", defaultDpi: 96, pages: [{ width: 100, height: 50 }] };

function open(info: VectorInfo = PDF) {
  const onopen = vi.fn();
  const onclose = vi.fn();
  render(VectorImportDialog, { path: "C:\\docs\\file", name: "file", info, onopen, onclose });
  return { onopen, onclose, user: userEvent.setup() };
}

const field = (name: string) => screen.getByRole("spinbutton", { name });
const page = (n: number) => screen.getByRole("button", { name: `Page ${n}` });
const ok = () => screen.getByRole("button", { name: "OK" });

test("the first page is picked at the PDF's resolution, and OK opens it", async () => {
  const { onopen, user } = open();
  expect(screen.getByRole("dialog", { name: "Import PDF — file" })).toBeInTheDocument();
  expect(page(1)).toHaveAttribute("aria-pressed", "true");
  expect(page(2)).toHaveAttribute("aria-pressed", "false");
  expect(screen.getByText("1 of 4 selected")).toBeInTheDocument();
  expect(field("Resolution")).toHaveValue(300);
  // The first picked page, as rendered at 300 ppi.
  expect(field("Width")).toHaveValue(2550);
  expect(field("Height")).toHaveValue(3300);
  await user.click(ok());
  expect(onopen).toHaveBeenCalledWith([0], 300);
});

test("pages are picked one by one, by Shift+click as a range, and all or none", async () => {
  const { onopen, user } = open();
  // Page 1 starts picked: click it off, then page 2 on.
  await user.click(page(1));
  await user.click(page(2));
  expect(screen.getByText("1 of 4 selected")).toBeInTheDocument();
  await user.keyboard("{Shift>}");
  await user.click(page(4));
  await user.keyboard("{/Shift}");
  // A range from the last page clicked (2) to this one (4).
  await user.click(ok());
  expect(onopen).toHaveBeenLastCalledWith([1, 2, 3], 300);
  await user.click(screen.getByRole("button", { name: "All" }));
  expect(screen.getByText("4 of 4 selected")).toBeInTheDocument();
  await user.click(ok());
  expect(onopen).toHaveBeenLastCalledWith([0, 1, 2, 3], 300);
  await user.click(screen.getByRole("button", { name: "None" }));
  expect(screen.getByText("Select at least one page")).toBeInTheDocument();
  expect(ok()).toBeDisabled();
});

test("a pixel size typed sets the resolution that gives it, and the other side follows", async () => {
  const { onopen, user } = open();
  await user.clear(field("Width"));
  await user.type(field("Width"), "1275");
  expect(field("Resolution")).toHaveValue(150);
  expect(field("Height")).toHaveValue(1650);
  await user.click(ok());
  expect(onopen).toHaveBeenCalledWith([0], 150);
});

test("a resolution typed changes the pixel size of the pages", async () => {
  const { user } = open();
  await user.clear(field("Resolution"));
  await user.type(field("Resolution"), "72");
  expect(field("Width")).toHaveValue(612);
  expect(field("Height")).toHaveValue(792);
});

test("a resolution that makes a page too large cannot be applied", async () => {
  const { onopen, user } = open();
  await user.clear(field("Resolution"));
  await user.type(field("Resolution"), "10000");
  expect(screen.getByText("Too large: at most 65,535 pixels per side")).toBeInTheDocument();
  expect(ok()).toBeDisabled();
  // An empty or null resolution is no better.
  await user.clear(field("Resolution"));
  expect(ok()).toBeDisabled();
  await user.type(field("Resolution"), "0");
  expect(ok()).toBeDisabled();
  expect(onopen).not.toHaveBeenCalled();
});

test("the resolution used is remembered for the next file of the same kind", async () => {
  const first = open();
  await first.user.clear(field("Resolution"));
  await first.user.type(field("Resolution"), "150");
  await first.user.click(ok());
  document.body.innerHTML = "";
  open();
  expect(field("Resolution")).toHaveValue(150);
  document.body.innerHTML = "";
  // Not for another kind, which has its own default.
  open(SVG);
  expect(field("Resolution")).toHaveValue(96);
});

test("a saved resolution out of range is ignored", () => {
  localStorage.setItem("slopshop.pdfImport.dpi", "99999");
  open();
  expect(field("Resolution")).toHaveValue(300);
});

test("an SVG is one page, previewed rather than picked", async () => {
  const { onopen, user } = open(SVG);
  expect(screen.getByRole("dialog", { name: "Import SVG — file" })).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Page 1" })).not.toBeInTheDocument();
  expect(screen.queryByText(/selected/)).not.toBeInTheDocument();
  expect(field("Width")).toHaveValue(133);
  expect(field("Height")).toHaveValue(67);
  await user.click(ok());
  expect(onopen).toHaveBeenCalledWith([0], 96);
});

test("Cancel and Escape close it without opening anything", async () => {
  const { onopen, onclose, user } = open();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(onclose).toHaveBeenCalledTimes(2);
  expect(onopen).not.toHaveBeenCalled();
});
