// The UI tests (`npm test`): the plain modules of src/lib under Node, the Svelte components in
// a simulated DOM (jsdom).
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { svelteTesting } from "@testing-library/svelte/vite";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [svelte(), svelteTesting()],
  test: {
    projects: [
      {
        extends: true,
        test: { name: "modules", include: ["tests/*.test.ts"], environment: "node" },
      },
      {
        extends: true,
        test: {
          name: "components",
          include: ["tests/components/**/*.test.ts"],
          environment: "jsdom",
          setupFiles: ["tests/components/setup.ts"],
        },
      },
    ],
  },
});
