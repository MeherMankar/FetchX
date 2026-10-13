import { defineConfig } from "vite";
import { resolve } from "path";

// Popup is built as IIFE (no ES module imports) so it works inside
// a plain <script src="dist/popup.js"> tag in popup.html.
export default defineConfig({
  build: {
    outDir: "dist",
    emptyOutDir: false,
    lib: {
      entry: resolve(__dirname, "src/popup/popup.ts"),
      name: "FetchXPopup",
      fileName: () => "popup.js",
      formats: ["iife"],
    },
    rollupOptions: {
      output: {
        assetFileNames: "[name].[ext]",
      },
    },
    sourcemap: false,
    minify: false,
    target: "es2022",
  },
});
