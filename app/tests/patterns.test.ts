import { expect, test } from "vitest";
import { nextPatternName, patternName } from "../src/lib/patterns";

const t = (key: string) => `<${key}>`;

test("a generated pattern is named by the interface, the user's by its name", () => {
  expect(patternName({ id: "builtin:dots", name: "" }, t as never)).toBe("<patterns.builtin.dots>");
  expect(patternName({ id: "file:pattern-1.png", name: "Bricks" }, t as never)).toBe("Bricks");
  // Unnamed (an index edited by hand): its identifier.
  expect(patternName({ id: "file:x.png", name: "" }, t as never)).toBe("file:x.png");
});

test("a new pattern takes the first free number", () => {
  const named = (n: number) => `Pattern ${n}`;
  expect(nextPatternName([], named)).toBe("Pattern 1");
  expect(
    nextPatternName(
      [
        { id: "file:a.png", name: "Pattern 1" },
        { id: "file:b.png", name: "Pattern 3" },
      ],
      named,
    ),
  ).toBe("Pattern 2");
});
