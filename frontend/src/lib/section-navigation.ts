/** Shared path rules for retained settings pages. Parents come from slash paths. */
export function sectionNavigation<const T extends readonly string[]>(sections: T) {
  type Section = T[number];
  function parse(value: unknown): Section | undefined {
    return typeof value === "string" && sections.includes(value) ? (value as Section) : undefined;
  }
  function parent(section: Section): Section | undefined {
    return parse(section.split("/").slice(0, -1).join("/"));
  }
  function ancestry(section?: Section): Section[] {
    return section ? [...ancestry(parent(section)), section] : [];
  }
  function available(
    section: Section | undefined,
    availability: Partial<Record<Section, boolean>>,
  ): Section | undefined {
    if (!section) return undefined;
    const blocked = ancestry(section).find((candidate) => availability[candidate] === false);
    return blocked ? available(parent(blocked), availability) : section;
  }
  function normalize(value: unknown): Section | undefined {
    if (typeof value !== "string" || !/^[a-z][a-z0-9-]*(\/[a-z][a-z0-9-]*)*$/.test(value))
      return undefined;
    return parse(value) ?? normalize(value.split("/").slice(0, -1).join("/"));
  }
  return { parse, parent, ancestry, available, normalize };
}
