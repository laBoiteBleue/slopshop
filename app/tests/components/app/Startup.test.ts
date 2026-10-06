import { screen } from "@testing-library/svelte";
import { expect, test } from "vitest";
import { open, respond } from "./harness";

// Before the engine is ready (the GPU starting up), the page says it is starting.

let start: () => void = () => {};
const started = new Promise<void>((resolve) => (start = resolve));
respond("presenter_mode", () => started.then(() => "frames"));

test("it says it is starting until the engine is ready, then shows the welcome page", async () => {
  open();
  expect(await screen.findByRole("status")).toHaveTextContent("Starting…");
  expect(screen.queryByText("Open an image or create a document")).not.toBeInTheDocument();
  start();
  expect(await screen.findByText("Open an image or create a document")).toBeInTheDocument();
  expect(screen.queryByText("Starting…")).not.toBeInTheDocument();
});
