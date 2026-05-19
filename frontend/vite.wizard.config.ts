/// <reference types="vitest/config" />
import { defineConfig } from "vite"
import react from "@vitejs/plugin-react"
import tailwindcss from "@tailwindcss/vite"
import { viteSingleFile } from "vite-plugin-singlefile"
import path from "path"
import { wizardManifest } from "./vite-plugins/wizard-manifest"

/**
 * Separate Vite build for the CLI's locally-served wizard (Mode A).
 *
 * Entry: `wizard.html` + `src/wizard-entry.tsx`.
 * Output: `dist-wizard/index.html` — a single self-contained HTML file with
 * all JS and CSS inlined. The file is copied into `cli/src/wizard/assets/`
 * and embedded into the CLI binary via `rust_embed`.
 *
 * Why single-file: the CLI's embedded axum server serves one request for the
 * wizard page. Rather than juggling multiple assets (chunk splitting, CSS
 * files, asset discovery) inside `rust_embed`, we ship exactly one HTML
 * artifact the server knows how to inject bootstrap config into.
 *
 * Keep the SPA-targeted `vite.config.ts` the authoritative config for the
 * dashboard at port 3000 — this file only governs the wizard bundle build.
 */
export default defineConfig({
  plugins: [react(), tailwindcss(), viteSingleFile(), wizardManifest()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  build: {
    outDir: "dist-wizard",
    emptyOutDir: true,
    rollupOptions: {
      input: path.resolve(__dirname, "wizard.html"),
    },
    // `viteSingleFile` relies on these being inlined. Keep explicit.
    cssCodeSplit: false,
    assetsInlineLimit: 100_000_000,
  },
})
