import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import PrintDialog from "../../src/lib/PrintDialog.svelte";

beforeEach(() => {
  localStorage.clear();
  // The engine renders the page: an (empty) JPEG.
  mockIPC((cmd) => (cmd === "print_page" ? new ArrayBuffer(8) : undefined));
  URL.createObjectURL = () => "blob:page";
  URL.revokeObjectURL = () => {};
});
afterEach(() => clearMocks());

/** A 2000 × 1000 image at 300 ppi. */
function open() {
  const onclose = vi.fn();
  render(PrintDialog, {
    documentId: 1,
    title: "Poster",
    width: 2000,
    height: 1000,
    resolution: 300,
    onclose,
  });
  return { onclose, user: userEvent.setup() };
}

const number = (name: string) => screen.getByRole("spinbutton", { name });

async function typeInto(user: ReturnType<typeof userEvent.setup>, name: string, text: string) {
  await user.clear(number(name));
  await user.type(number(name), text);
  await user.tab();
}

test("a wide image goes landscape, fitted within the margins; Print waits for the page", async () => {
  open();
  expect(screen.getByRole("radio", { name: "Landscape" })).toHaveAttribute("aria-checked", "true");
  expect(screen.getByRole("checkbox", { name: "Scale to fit the paper" })).toBeChecked();
  // A4 landscape less 1 cm margins: 27.7 cm wide.
  expect(number("Width")).toHaveValue(27.7);
  expect(screen.getByRole("button", { name: "Print…" })).toBeDisabled();
  await vi.waitFor(() => expect(screen.getByRole("button", { name: "Print…" })).toBeEnabled());
});

test("a printed width sets the resolution and stops fitting", async () => {
  const { user } = open();
  await typeInto(user, "Width", "10");
  expect(screen.getByRole("checkbox", { name: "Scale to fit the paper" })).not.toBeChecked();
  expect(number("Resolution")).toHaveValue(508);
  expect(number("Height")).toHaveValue(5);
});

test("a low resolution is pointed out", async () => {
  const { user } = open();
  await typeInto(user, "Resolution", "100");
  expect(screen.getByText(/100 ppi: the print may look soft/)).toBeInTheDocument();
  expect(number("Width")).toHaveValue(50.8);
});

test("the position is free once not fitted nor centered", async () => {
  const { user } = open();
  expect(number("Top")).toBeDisabled();
  await user.click(screen.getByRole("checkbox", { name: "Scale to fit the paper" }));
  await user.click(screen.getByRole("checkbox", { name: "Center" }));
  await typeInto(user, "Top", "2");
  expect(number("Top")).toHaveValue(2);
});

test("the paper is remembered for the next print; Cancel closes", async () => {
  const first = open();
  await first.user.selectOptions(screen.getByRole("combobox", { name: "Paper" }), "a3");
  await first.user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(first.onclose).toHaveBeenCalledOnce();
  document.body.innerHTML = "";
  open();
  expect(screen.getByRole("combobox", { name: "Paper" })).toHaveValue("a3");
});
