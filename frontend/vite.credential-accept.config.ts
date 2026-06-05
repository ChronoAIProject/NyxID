/// <reference types="vitest/config" />
import { defineConfig } from "vite";
import path from "node:path";
import { releaseIntegrityPlugin } from "./vite-plugins/release-integrity";

export default defineConfig({
  base: "/credential-accept/",
  publicDir: false,
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    outDir: "dist/credential-accept",
    emptyOutDir: true,
    assetsDir: "assets",
    rollupOptions: {
      input: path.resolve(__dirname, "credential-accept.html"),
      output: {
        entryFileNames: "assets/credential-accept-[hash].js",
        chunkFileNames: "assets/credential-accept-[hash].js",
        assetFileNames: "assets/credential-accept-[hash][extname]",
      },
    },
  },
  plugins: [releaseIntegrityPlugin()],
});
