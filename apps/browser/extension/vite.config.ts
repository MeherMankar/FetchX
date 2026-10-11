import { defineConfig } from "vite";
import { resolve } from "path";

// Background service worker — ES module (Chrome SW supports type=module)
export default defineConfig({
  build: {
    outDir: "dist",
    emptyOutDir: true,
    rollupOptions: {
      input: {
        background: resolve(__dirname, "src/background/service-worker.ts"),
        content:    resolve(__dirname, "src/content/content.ts"),
      },
      output: {
        entryFileNames: "[name].js",
        format: "es",
      },
    },
    sourcemap: false,
    minify: false,
    target: "es2022",
  },
});
