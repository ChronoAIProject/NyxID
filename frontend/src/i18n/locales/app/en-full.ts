/** Test-only full English catalog. Runtime English uses en-overrides.json. */
type Catalog = Record<string, string>;

const modules = import.meta.glob<Catalog>("./*/en.json", {
  eager: true,
  import: "default",
});

const catalog: Catalog = {};
for (const [, portion] of Object.entries(modules).sort(([left], [right]) => left < right ? -1 : left > right ? 1 : 0)) {
  Object.assign(catalog, portion);
}

export default catalog;
