import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { resolve } from "node:path";

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 5174, strictPort: true },
  build: {
    target: "es2021",
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        popover: resolve(__dirname, "popover.html"),
        settings: resolve(__dirname, "settings.html"),
      },
    },
  },
  test: {
    include: ["src/**/*.test.ts"],
  },
});
