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
      (error: unknown) => callback(error instanceof Error ? error : new Error(String(error)), true),
    );
  },
};

/** Render with English fallback if downloading a catalog stalls. */
export function waitForCatalogs(ready: PromiseLike<unknown>, timeoutMs = 1500): Promise<void> {
  return new Promise((resolve) => {
    const timer = setTimeout(resolve, timeoutMs);
    const finish = () => {
      clearTimeout(timer);
      resolve();
    };
    void Promise.resolve(ready).then(finish, finish);
  });
}
