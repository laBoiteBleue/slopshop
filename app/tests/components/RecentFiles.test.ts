import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { render, screen, waitFor } from "@testing-library/svelte";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import RecentFiles from "../../src/lib/RecentFiles.svelte";

/** The paths the engine was asked a thumbnail of. */
let asked: string[];

/** The paths that have a thumbnail (a 2 x 1 image); the engine refuses the others. */
let withThumbnail: string[];

/** What the engine sends: width, height (little-endian), then RGBA8 pixels. */
function thumbnail(width: number, height: number): ArrayBuffer {
  const buffer = new ArrayBuffer(8 + width * height * 4);
  const view = new DataView(buffer);
  view.setUint32(0, width, true);
  view.setUint32(4, height, true);
  return buffer;
}

beforeEach(() => {
  asked = [];
  withThumbnail = [];
  mockIPC((cmd, args) => {
    if (cmd !== "recent_thumbnail") return;
    const { path } = args as { path: string };
    asked.push(path);
    return withThumbnail.includes(path) ? thumbnail(2, 1) : Promise.reject("no thumbnail");
  });
});
afterEach(() => clearMocks());

const PATHS = [
  "/home/ana/photos/sunset.jpg",
  "/home/ana/photos/2024/beach.slop",
  "/home/ana/scans",
];

function open(paths = PATHS) {
  const onopen = vi.fn();
  const view = render(RecentFiles, { paths, onopen });
  return { ...view, onopen, user: userEvent.setup() };
}

test("each recent file is shown by its name, newest first", () => {
  open();
  const names = screen.getAllByRole("button").map((button) => button.textContent?.trim());
  expect(names).toEqual(["sunset.jpg", "beach.slop", "scans"]);
  expect(screen.getByRole("heading", { name: "Recent" })).toBeInTheDocument();
});

test("two files with the same name are told apart by their folder", () => {
  open(["/a/photo.jpg", "/b/photo.jpg"]);
  expect(screen.getByText("photo.jpg — a")).toBeInTheDocument();
  expect(screen.getByText("photo.jpg — b")).toBeInTheDocument();
});

test("the full path is the tooltip of an entry", () => {
  open();
  expect(screen.getByRole("button", { name: "beach.slop" })).toHaveAttribute(
    "title",
    "/home/ana/photos/2024/beach.slop",
  );
});

test("clicking an entry opens that path", async () => {
  const { user, onopen } = open();
  await user.click(screen.getByRole("button", { name: "beach.slop" }));
  expect(onopen).toHaveBeenCalledExactlyOnceWith("/home/ana/photos/2024/beach.slop");
});

test("the engine is asked the thumbnail of each entry, and the icon stays without one", async () => {
  open();
  await waitFor(() => expect(asked.toSorted()).toEqual(PATHS.toSorted()));
  // Refused: the canvas stays hidden, the folder icon behind shows.
  expect(document.querySelectorAll("canvas:not([hidden])")).toHaveLength(0);
});

test("a thumbnail the engine has is shown in its entry, sized as received", async () => {
  withThumbnail = ["/home/ana/photos/sunset.jpg"];
  open();
  const shown = () => document.querySelectorAll<HTMLCanvasElement>("canvas:not([hidden])");
  await waitFor(() => expect(shown()).toHaveLength(1));
  expect(shown()[0].closest("button")).toHaveAccessibleName("sunset.jpg");
  expect([shown()[0].width, shown()[0].height]).toEqual([2, 1]);
});

test("a new entry is asked its thumbnail, the others keep theirs", async () => {
  const { rerender, onopen } = open();
  await waitFor(() => expect(asked).toHaveLength(3));
  await rerender({ paths: ["/home/ana/new.png", ...PATHS], onopen });
  await waitFor(() => expect(asked).toHaveLength(4));
  expect(asked.at(-1)).toBe("/home/ana/new.png");
  expect(screen.getAllByRole("button")).toHaveLength(4);
});

test("the list is empty when there is no recent file", () => {
  open([]);
  expect(screen.queryAllByRole("button")).toHaveLength(0);
});
