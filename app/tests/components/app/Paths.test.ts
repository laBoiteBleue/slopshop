import { screen, waitFor, within } from "@testing-library/svelte";
import { expect, test } from "vitest";
import type { DocumentView, EditRequest, PathView } from "../../../src/lib/engine";
import { documentView, layer, open, respond, sent } from "./harness";

// The Paths panel (Window > Paths, ADR 0041): the Pen's Path mode drawing the Work Path, a
// chosen path drawn into, and the panel's buttons: Fill, Stroke, Load as Selection, Make Work
// Path from Selection, New Path.

const triangle = {
  kind: "path" as const,
  rule: "nonzero" as const,
  subpaths: [
    {
      start: [200, 200] as [number, number],
      segments: [
        [300, 200],
        [250, 280],
        [200, 200],
      ] as [number, number][],
      closed: true,
    },
  ],
};

let nextPath = 10;

/** The engine's answer to the path edits: the document's paths as they become. */
respond("perform", (args, doc) => {
  const edit = args.edit as EditRequest;
  const paths: PathView[] = [...(doc.paths ?? [])];
  if (edit.kind === "setWorkPath") {
    const at = paths.findIndex((p) => p.work);
    const work = { id: nextPath++, name: "", work: true, geometry: edit.path };
    if (at >= 0) paths[at] = work;
    else paths.push(work);
  } else if (edit.kind === "newPath") {
    paths.push({
      id: nextPath++,
      name: edit.name,
      work: false,
      geometry: { ...triangle, subpaths: [] },
    });
  } else if (edit.kind === "setPathGeometry") {
    const at = paths.findIndex((p) => p.id === edit.id);
    paths[at] = { ...paths[at], geometry: edit.path };
  }
  doc.paths = paths;
  return { ...doc, revision: doc.revision + 1 };
});
for (const cmd of ["fill", "stroke_path", "load_path_selection", "work_path_from_selection"]) {
  respond(cmd, (_, doc) => ({ ...doc, revision: doc.revision + 1 }));
}

function withPaths(paths: PathView[], changes: Partial<DocumentView> = {}): DocumentView {
  return { ...documentView(1, "cat.jpg", [layer(1, "Cat")]), paths, ...changes };
}

async function showPaths(user: ReturnType<typeof open>) {
  await screen.findByText("cat.jpg");
  await user.click(screen.getByRole("tab", { name: /Paths/ }));
  return screen.getByRole("tabpanel", { name: "Paths" });
}

function click(user: ReturnType<typeof open>, ...points: [number, number][]) {
  const svg = document.querySelector("svg.pen-tool") as SVGSVGElement;
  return user.pointer(
    points.map(([x, y]) => ({
      keys: "[MouseLeft]",
      target: svg,
      coords: { clientX: x, clientY: y },
    })),
  );
}

test("Window > Paths lists the paths; the Pen in Path mode draws the Work Path, then chosen", async () => {
  const user = open(withPaths([{ id: 3, name: "Outline", work: false, geometry: triangle }]));
  const panel = await showPaths(user);
  expect(
    within(panel)
      .getAllByRole("option")
      .map((o) => o.textContent?.trim()),
  ).toEqual(["Outline"]);
  await user.keyboard("p");
  await user.selectOptions(screen.getByRole("combobox", { name: "Tool mode" }), "path");
  await click(user, [200, 200], [300, 200], [250, 280], [201, 200]);
  await waitFor(() =>
    expect(sent("perform")).toEqual([
      { documentId: 1, edit: { kind: "setWorkPath", path: triangle } },
    ]),
  );
  // No shape layer: the Work Path, listed and chosen.
  const work = await within(panel).findByRole("option", { name: /Work Path/ });
  await waitFor(() => expect(work).toHaveAttribute("aria-selected", "true"));
  // Drawn again: added to the chosen Work Path.
  await click(user, [10, 10], [60, 10], [10, 50], [11, 10]);
  await waitFor(() => expect(sent("perform")).toHaveLength(2));
  const added = sent("perform")[1].edit as Extract<EditRequest, { kind: "setPathGeometry" }>;
  expect(added.kind).toBe("setPathGeometry");
  expect(added.path.subpaths).toHaveLength(2);
  // Ctrl+Enter makes a selection of the chosen path.
  await user.keyboard("{Control>}{Enter}{/Control}");
  await waitFor(() =>
    expect(sent("load_path_selection")).toEqual([
      { documentId: 1, path: { kind: "path", id: added.id }, mode: "replace" },
    ]),
  );
});

test("the panel's buttons fill, stroke and load the chosen path, and make new paths", async () => {
  const user = open(
    withPaths([{ id: 3, name: "Outline", work: false, geometry: triangle }], {
      selectionKey: 5,
    }),
  );
  const panel = await showPaths(user);
  await user.click(within(panel).getByRole("option", { name: /Outline/ }));
  const button = (name: string) => within(panel).getByRole("button", { name });
  await user.click(button("Fill path with foreground color (Alt: options)"));
  await user.click(button("Stroke path with the tool chosen last (Alt: options)"));
  await user.click(button("Load path as a selection"));
  await user.click(button("Make work path from selection (Alt: tolerance)"));
  await user.click(button("New Path"));
  const path = { kind: "path", id: 3 };
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  expect(sent("fill")).toEqual([
    {
      documentId: 1,
      layerId: 1,
      target: "layer",
      color: [0, 0, 0],
      opacity: 1,
      stroke: null,
      pattern: null,
      path,
    },
  ]);
  expect(sent("stroke_path")).toEqual([
    expect.objectContaining({
      documentId: 1,
      layerId: 1,
      target: "layer",
      path,
      color: [0, 0, 0],
      simulatePressure: false,
      brush: expect.objectContaining({ size: 30, spacing: 0.25 }),
    }),
  ]);
  expect(sent("load_path_selection")).toEqual([{ documentId: 1, path, mode: "replace" }]);
  expect(sent("work_path_from_selection")).toEqual([{ documentId: 1, tolerance: 2 }]);
  expect(sent("perform")).toEqual([{ documentId: 1, edit: { kind: "newPath", name: "Path 1" } }]);
  // The new path is chosen.
  await waitFor(() =>
    expect(within(panel).getByRole("option", { name: /Path 1/ })).toHaveAttribute(
      "aria-selected",
      "true",
    ),
  );
});

test("Alt on Stroke Path opens its dialog: the Eraser chosen strokes, and stays for the button", async () => {
  const user = open(withPaths([{ id: 3, name: "Outline", work: false, geometry: triangle }]));
  const panel = await showPaths(user);
  await user.click(within(panel).getByRole("option", { name: /Outline/ }));
  const stroke = within(panel).getByRole("button", {
    name: "Stroke path with the tool chosen last (Alt: options)",
  });
  await user.keyboard("{Alt>}");
  await user.click(stroke);
  await user.keyboard("{/Alt}");
  const dialog = await screen.findByRole("dialog", { name: "Stroke Path" });
  await user.selectOptions(within(dialog).getByRole("combobox", { name: "Tool" }), "eraser");
  await user.click(within(dialog).getByRole("checkbox", { name: "Simulate Pressure" }));
  await user.click(within(dialog).getByRole("button", { name: "OK" }));
  await user.click(stroke);
  await waitFor(() => expect(sent("stroke_path")).toHaveLength(2));
  for (const call of sent("stroke_path")) {
    expect(call).toMatchObject({ color: null, simulatePressure: true });
    expect(call.brush).toMatchObject({ size: 50 });
  }
});

test("Alt on Fill Path opens Edit > Fill's dialog, Alt on Make Work Path its tolerance", async () => {
  const user = open(
    withPaths([{ id: 3, name: "Outline", work: false, geometry: triangle }], {
      selectionKey: 5,
    }),
  );
  const panel = await showPaths(user);
  await user.click(within(panel).getByRole("option", { name: /Outline/ }));
  await user.keyboard("{Alt>}");
  await user.click(
    within(panel).getByRole("button", { name: "Fill path with foreground color (Alt: options)" }),
  );
  await user.keyboard("{/Alt}");
  const fill = await screen.findByRole("dialog", { name: "Fill" });
  await user.click(within(fill).getByRole("button", { name: "OK" }));
  await waitFor(() =>
    expect(sent("fill")).toEqual([
      expect.objectContaining({ layerId: 1, path: { kind: "path", id: 3 } }),
    ]),
  );
  await user.keyboard("{Alt>}");
  await user.click(
    within(panel).getByRole("button", { name: "Make work path from selection (Alt: tolerance)" }),
  );
  await user.keyboard("{/Alt}");
  const dialog = await screen.findByRole("dialog", { name: "Make Work Path" });
  const tolerance = within(dialog).getByRole("spinbutton", { name: "Tolerance" });
  await user.clear(tolerance);
  await user.type(tolerance, "4.5{Enter}");
  await waitFor(() =>
    expect(sent("work_path_from_selection")).toEqual([{ documentId: 1, tolerance: 4.5 }]),
  );
});

test("a chosen path's anchors show with the Pen; Ctrl-drag moves one, in one edit of the path", async () => {
  const user = open(withPaths([{ id: 3, name: "Outline", work: false, geometry: triangle }]));
  const panel = await showPaths(user);
  await user.click(within(panel).getByRole("option", { name: /Outline/ }));
  await user.keyboard("p");
  await waitFor(() => expect(screen.getAllByTestId("pen-layer-anchor")).toHaveLength(3));
  const svg = document.querySelector("svg.pen-tool") as SVGSVGElement;
  await user.keyboard("{Control>}");
  await user.pointer([
    { keys: "[MouseLeft>]", target: svg, coords: { clientX: 300, clientY: 200 } },
    { target: svg, coords: { clientX: 310, clientY: 200 } },
    { target: svg, coords: { clientX: 320, clientY: 210 } },
    { keys: "[/MouseLeft]", target: svg, coords: { clientX: 320, clientY: 210 } },
  ]);
  await user.keyboard("{/Control}");
  await waitFor(() => expect(sent("perform")).toHaveLength(1));
  const edit = sent("perform")[0].edit as Extract<EditRequest, { kind: "setPathGeometry" }>;
  expect(edit).toMatchObject({ kind: "setPathGeometry", id: 3 });
  expect(edit.path.subpaths[0].segments[0]).toEqual([320, 210]);
});
