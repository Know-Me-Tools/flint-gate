import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm", "cjs"],
  outExtension({ format }) {
    return { js: format === "esm" ? ".mjs" : ".cjs" };
  },
  dts: true,
  sourcemap: true,
  clean: true,
  target: "es2022",
  platform: "neutral",
  tsconfig: "./tsconfig.json",
  // Keep ESM output clean; CJS gets a require shim automatically.
  banner: {
    js: "// @know-me/flint-gate — Edge-runtime safe client (no Node.js built-ins)",
  },
});
