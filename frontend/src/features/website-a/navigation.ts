/** Match a whole route segment so sibling prefixes never select a column. */
export function websiteNavProps(route: string, destination: string) {
  const pathname = route.replace(/\/$/, "") || "/";
  const exact = pathname === destination;
  const section = pathname.startsWith(`${destination}/`);
  return {
    className: exact || section ? "nl on" : "nl",
    "aria-current": exact
      ? ("page" as const)
      : section
        ? ("location" as const)
        : undefined,
  };
}
