import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "node:path";
export default defineConfig({
  plugins: [react()],
  resolve: { alias: { "@": path.resolve(import.meta.dirname, "src") } },
  build: {
    ssr: "scripts/public-render.tsx",
    outDir: ".public-render",
    sourcemap: false,
    rollupOptions: { output: { entryFileNames: "public-render.js" } },
  },
});
