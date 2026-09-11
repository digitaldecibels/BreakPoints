import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";

// Tauri drives this dev server, so the port is fixed and failures must be loud.
export default defineConfig({
  plugins: [tailwindcss()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    // Prose at the repo root is not part of the chrome, but Vite watches the
    // whole root, so editing CLAUDE.md or tasks.md forced a full page reload
    // of the chrome while the app was running. That reload takes the app down
    // with it, which made documentation edits look like random crashes.
    watch: { ignored: ["**/src-tauri/**", "**/*.md", "**/docs/**"] },
  },
  build: { target: "safari15", sourcemap: false },
});
