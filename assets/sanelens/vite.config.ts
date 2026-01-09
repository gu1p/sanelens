import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    assetsDir: "",
    cssCodeSplit: false,
    rollupOptions: {
      output: {
        inlineDynamicImports: true,
        entryFileNames: "app.js",
        assetFileNames: (assetInfo) => {
          if (!assetInfo.name) {
            return "asset";
          }
          if (assetInfo.name === "style.css") {
            return "styles.css";
          }
          return assetInfo.name;
        },
      },
    },
  },
});
