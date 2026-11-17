import { defineConfig } from "vite";
import { resolve } from "path";
import { cpSync, existsSync } from "fs";

// Plugin that copies src/widevine/l3/ into dist/widevine/l3/ as static assets.
function copyWidevinL3StaticAssets() {
  return {
    name: "copy-widevine-l3-static",
    closeBundle() {
      const src = resolve(__dirname, "src/widevine/l3");
      const dest = resolve(__dirname, "dist/widevine/l3");
      if (existsSync(src)) {
        cpSync(src, dest, { recursive: true });
      }
    },
  };
}

// Background service worker — ES module (Chrome SW supports type=module)
export default defineConfig({
  plugins: [copyWidevinL3StaticAssets()],
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
