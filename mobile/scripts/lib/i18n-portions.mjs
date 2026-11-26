/**
 * Path -> area map for the mobile catalogs. Every `src/features/<name>/`
 * directory owns the area `<name>`, so a new feature gets its own area without
 * editing this file. The navigation shell in `src/app/` owns `navigation`, and
 * remaining shared code (components, hooks, lib, theme, i18n) maps to `shared`,
 * which also holds keys used by several areas. `unattributed` holds keys with
 * no literal reference. The adoption tool uses this map to place keys added on
 * pre-split branches.
 */
export const RESERVED_AREAS = ["navigation", "shared", "unattributed"];

/** The area of one feature directory; fails if it would collide with a reserved area. */
export function areaForFeature(name) {
  if (RESERVED_AREAS.includes(name)) {
    throw new Error(`mobile feature directory src/features/${name} collides with the reserved catalog area "${name}" (reserved: ${RESERVED_AREAS.join(", ")}); rename the feature directory`);
  }
  return name;
}

export function portionForPath(file) {
  const feature = file.match(/^src\/features\/([^/]+)\//);
  if (feature) return areaForFeature(feature[1]);
  if (file.startsWith("src/app/")) return "navigation";
  return "shared";
}
