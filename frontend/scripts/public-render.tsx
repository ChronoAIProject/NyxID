import { renderToString } from "react-dom/server";
import { RouterProvider } from "@tanstack/react-router";
import { publicRouter } from "../src/features/public-site/router";
export {
  publishedPages,
  publicationPath,
} from "../src/features/public-site/publication";
import inventory from "../content/publication.json";
export const publicationInventory = inventory.pages;
export { allDocPages } from "../src/features/docs/manifest";
export {
  metadataHead,
  markdownPath,
  PUBLIC_ORIGIN,
} from "../src/features/public-site/metadata";
export async function renderPage(
  path: string,
  readDoc: (slug: string) => Promise<string>,
) {
  const router = publicRouter(path, readDoc);
  await router.load();
  const match = router.state.matches.find((match) => match.loaderData);
  if (!match || match.status !== "success")
    throw new Error(
      `Public render failed: ${path}: ${match?.error ?? "no body"}`,
    );
  const html = renderToString(<RouterProvider router={router} />);
  if (!html.includes("<h1") || html.includes("<!--$!-->"))
    throw new Error(`Incomplete public body: ${path}`);
  return {
    html,
    meta: (
      match.loaderData as unknown as {
        meta: import("../src/features/public-site/metadata").PublicMetadata;
      }
    ).meta,
  };
}

export {
  websiteRelease,
  websiteVisibility,
  websiteConfiguration,
} from "../src/features/website-a/release-config";
