import { render, screen } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { expect, test, vi } from "vitest";
import ClearDialog from "../../src/lib/ClearDialog.svelte";

function open(generative = true) {
  const onchoose = vi.fn();
  const onclose = vi.fn();
  render(ClearDialog, {
    colors: { foreground: "#000000", background: "#336699" },
    generative,
    onchoose,
    onclose,
  });
  return { onchoose, onclose, user: userEvent.setup() };
}

const choice = (name: string) => screen.getByRole("radio", { name });
const ok = () => screen.getByRole("button", { name: "OK" });

// The dialog remembers its last choice in the module, for the whole session: the first test
// below is the only one that sees the default, the others choose what they depend on.

test("Delete offers four choices, transparency first and chosen", async () => {
  const { onchoose, user } = open();
  expect(screen.getByRole("dialog", { name: "Delete Selection" })).toBeInTheDocument();
  expect(screen.getAllByRole("radio").map((r) => r.getAttribute("value"))).toEqual([
    "transparent",
    "background",
    "foreground",
    "generative",
  ]);
  expect(choice("Transparency")).toBeChecked();
  await user.click(ok());
  expect(onchoose).toHaveBeenCalledWith("transparent");
});

test("a color choice shows its swatch, applies, and comes back next time", async () => {
  const first = open();
  const swatch = choice("Background Color").parentElement?.querySelector(".swatch");
  expect(swatch).toHaveStyle({ background: "rgb(51, 102, 153)" });
  await first.user.click(choice("Background Color"));
  await first.user.click(ok());
  expect(first.onchoose).toHaveBeenCalledWith("background");
  document.body.innerHTML = "";
  open();
  expect(choice("Background Color")).toBeChecked();
});

test("arrows move between the choices and Enter applies", async () => {
  const { onchoose, user } = open();
  choice("Transparency").focus();
  await user.click(choice("Transparency"));
  await user.keyboard("{ArrowDown}{ArrowDown}{Enter}");
  expect(onchoose).toHaveBeenCalledWith("foreground");
});

test("generative fill is chosen like the others, and explained", async () => {
  const { onchoose, user } = open();
  expect(screen.getByText(/rebuilds the background around the selection/)).toBeInTheDocument();
  await user.click(choice("Generative Fill"));
  await user.click(ok());
  expect(onchoose).toHaveBeenCalledWith("generative");
});

test("where generative fill is not offered, it is disabled and never preselected", async () => {
  // The last choice was generative fill (the test above).
  const { onchoose, user } = open(false);
  expect(choice("Generative Fill")).toBeDisabled();
  expect(choice("Transparency")).toBeChecked();
  expect(screen.getByText(/only available on Windows/)).toBeInTheDocument();
  await user.click(ok());
  expect(onchoose).toHaveBeenCalledWith("transparent");
});

test("Cancel and Esc close it without choosing", async () => {
  const { onchoose, onclose, user } = open();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onclose).toHaveBeenCalledTimes(1);
  // Esc: the dialog's cancel event (what the browser sends on Escape).
  screen.getByRole("dialog").dispatchEvent(new Event("cancel", { cancelable: true }));
  expect(onclose).toHaveBeenCalledTimes(2);
  expect(onchoose).not.toHaveBeenCalled();
});
