import { mergeConfig } from "vite";
import base from "./vite.config";

// Browser-only fixtures are dynamically imported after route load. Include them
// in the initial dependency scan so discovery cannot reload a live test document
// while Playwright awaits an import in page.evaluate.
export default mergeConfig(base, {
  cacheDir: "node_modules/.vite-e2e",
  optimizeDeps: { entries: ["index.html", "e2e/fixtures/*.{ts,tsx}"] },
});
