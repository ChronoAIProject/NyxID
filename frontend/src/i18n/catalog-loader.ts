import type { BackendModule } from "i18next";

type CatalogLoader = (language: string) => Promise<Record<string, string>>;
const loaders = new Map<string, CatalogLoader>();

export function registerCatalogLoader(namespace: string, loader: CatalogLoader) {
  loaders.set(namespace, loader);
}

// Entry points register their own catalogs so standalone wizards stay independent.
export const catalogBackend: BackendModule = {
  type: "backend",
  init() {},
  read(language, namespace, callback) {
    const loader = loaders.get(namespace);
    if (!loader) {
      callback(null, {});
      return;
    }
    void loader(language).then(
      (catalog) => callback(null, catalog),
      (error: unknown) => callback(error instanceof Error ? error : new Error(String(error)), false),
    );
  },
};
