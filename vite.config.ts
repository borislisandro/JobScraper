import { defineConfig, mergeConfig } from "vite";
import { defineConfig as defineVitestConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// vitest pins its own (older) vite as a dependency, so importing its defineConfig
// directly here would type-check `plugins` against a different vite's Plugin type
// than @vitejs/plugin-react uses. Merge separately typed configs instead.
export default mergeConfig(
  defineConfig({
    plugins: [react()],
    clearScreen: false,
    server: { port: 1420, strictPort: true, host: "127.0.0.1", watch: { ignored: ["**/src-tauri/**"] } },
    envPrefix: ["VITE_", "TAURI_"]
  }),
  defineVitestConfig({ test: { include: ["src/**/*.test.{ts,tsx}"] } })
);
