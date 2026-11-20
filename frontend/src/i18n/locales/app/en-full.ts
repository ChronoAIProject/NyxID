/** Test-only full English catalog. Runtime English uses en-overrides.json. */
type Catalog = Record<string, string>;

const modules = import.meta.glob<Catalog>("./*/en.json", {
  eager: true,
  import: "default",
});

// Merge by area directory name in code-unit order, the order the catalog tooling
// uses (sorting the paths would put "assistant-account/" before "assistant/").
const area = (file: string) => file.split("/")[1] ?? file;
const catalog: Catalog = {};
for (const [, portion] of Object.entries(modules).sort(([left], [right]) => area(left) < area(right) ? -1 : area(left) > area(right) ? 1 : 0)) {
  Object.assign(catalog, portion);
}

export default catalog;
