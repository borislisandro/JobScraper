// Minimal lint gate: catches real bugs (unused vars, hook rule violations,
// unsafe TS patterns) without demanding any reformatting of the codebase's
// dense one-line style. No prettier, no max-len, no stylistic rules.
import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

export default tseslint.config(
  {
    ignores: [
      "dist",
      "src-tauri",
      "node_modules",
      "sidecar/node_modules",
      "vite.config.js",
      "vite.config.d.ts",
      ".tooling",
      "backups",
      "restore-staging",
      "tmp",
      "temp",
      "logs",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  { languageOptions: { globals: { ...globals.node } } },
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: { globals: { ...globals.browser, ...globals.node } },
    plugins: { "react-hooks": reactHooks },
    rules: {
      // Only the two long-stable hook rules. eslint-plugin-react-hooks v7's "recommended"
      // preset is the full React Compiler diagnostic set (refs/purity/immutability/etc.)
      // and flags plain, correct patterns like ref={someLib.setNodeRef} as errors - not
      // appropriate here without adopting the compiler.
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "warn",
      "@typescript-eslint/no-unused-vars": ["warn", { argsIgnorePattern: "^_" }],
      "@typescript-eslint/no-explicit-any": "off",
      "no-empty": "off",
    },
  },
  {
    // worker.mjs's Playwright page.evaluate() callbacks run inside a real
    // browser tab, not the Node process, so DOM globals are legitimate here too.
    files: ["sidecar/**/*.mjs"],
    languageOptions: { globals: { ...globals.node, ...globals.browser } },
    rules: { "no-unused-vars": ["warn", { argsIgnorePattern: "^_" }] },
  },
);
