import { assertPublicConfiguration } from "./configuration-boundary";
import { createRoot, hydrateRoot } from "react-dom/client";
import { RouterProvider } from "@tanstack/react-router";
import { publicRouter } from "./router";
import { crashRootOptions } from "@/lib/crash-reporting/install";
import { reportCrash } from "@/lib/crash-reporting/reporter";
import "../../app.css";
export async function bootPublicSite() {
  const root = document.getElementById("root");
  if (!root) throw new Error("Root element not found");
  assertPublicConfiguration(root);
  const router = publicRouter();
  // TanStack's native SSR marker preserves its server boundary shape. Loaders
  // are resolved explicitly here, so no serialized private application state is needed.
  if (root.dataset.publicRender === "v1") router.ssr = { manifest: undefined };
  await router.load();
  if (root.dataset.publicRender === "v1") {
    // This public-only hydration resolves loaders itself. Seed the initial
    // location as TanStack's SSR hydrator does, so the first navigation can
    // save its scroll position and Back can restore that document correctly.
    router.stores.resolvedLocation.setState(() => router.stores.location.state);
  }
  const tree = <RouterProvider router={router} />;
  if (root.dataset.publicRender === "v1")
    hydrateRoot(root, tree, {
      ...crashRootOptions,
      onRecoverableError: (error) => {
        console.error("[public-hydration]", error);
        void reportCrash(error, "recoverable_error");
      },
    });
  else createRoot(root, crashRootOptions).render(tree);
  if (import.meta.env.VITE_STATIC_PREVIEW === "true") {
    // Static previews have no API or session. Keep docs display settings and public controls.
    const [{ updatePublicDocsRuntime }, { PublicDocsTheme }] = await Promise.all([
      import("../docs/docs-layout-view"),
      import("./docs-theme"),
    ]);
    updatePublicDocsRuntime(false, <PublicDocsTheme />);
  } else {
    // Reuse the existing session check, consent, and telemetry only in the browser.
    const { startPublicSession } = await import("./session");
    await startPublicSession(router);
  }
}
