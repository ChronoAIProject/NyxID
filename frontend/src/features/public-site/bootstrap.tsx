import { createRoot, hydrateRoot } from "react-dom/client";
import { RouterProvider } from "@tanstack/react-router";
import { publicRouter } from "./router";
import "../../app.css";
export async function bootPublicSite() {
  const root = document.getElementById("root");
  if (!root) throw new Error("Root element not found");
  const router = publicRouter();
  await router.load();
  const tree = <RouterProvider router={router} />;
  if (root.dataset.publicRender === "v1")
    hydrateRoot(root, tree, {
      onRecoverableError: (error) => console.error("[public-hydration]", error),
    });
  else createRoot(root).render(tree);
  // Reuse the existing session check, consent, and telemetry only in the browser.
  const { startPublicSession } = await import("./session");
  await startPublicSession(router);
}
