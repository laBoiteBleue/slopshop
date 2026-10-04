import { expect, test } from "vitest";
import { defaultLayout, loadLayout, saveLayout, type Layout } from "../src/lib/layout";

function memory(items: Record<string, string> = {}) {
  const map = new Map(Object.entries(items));
  return {
    getItem: (key: string) => map.get(key) ?? null,
    setItem: (key: string, value: string) => void map.set(key, value),
  };
}

test("nothing saved: the default layout", () => {
  expect(loadLayout(memory())).toEqual({
    dock: { open: "properties", height: 280 },
    order: ["properties", "selections", "history"],
    panelWidth: 260,
    optionsBar: true,
    toolbar: true,
  });
});

test("the layout saved last comes back", () => {
  const store = memory();
  const layout: Layout = {
    dock: { open: null, height: 333 },
    order: ["selections", "history", "properties"],
    panelWidth: 410,
    optionsBar: false,
    toolbar: true,
  };
  saveLayout(layout, store);
  expect(JSON.parse(store.getItem("slopshop.layout")!).version).toBe(1);
  expect(loadLayout(store)).toEqual(layout);
});

test("what the separate records of before kept is read", () => {
  const store = memory({
    "slopshop.dock": JSON.stringify({ open: "selections", height: 300 }),
    "slopshop.panelWidth": "420",
  });
  expect(loadLayout(store)).toMatchObject({
    dock: { open: "selections", height: 300 },
    panelWidth: 420,
  });
});

test("unknown panels are dropped, missing ones get their place, odd values the default", () => {
  const store = memory({
    "slopshop.layout": JSON.stringify({
      version: 1,
      dock: { open: "navigator", height: 5 },
      order: ["navigator", "selections", "selections"],
      panelWidth: "wide",
      toolbar: false,
    }),
  });
  expect(loadLayout(store)).toEqual({
    ...defaultLayout(),
    order: ["selections", "properties", "history"],
    toolbar: false,
  });
  expect(loadLayout(memory({ "slopshop.layout": "{not json" }))).toEqual(defaultLayout());
});
