/** Native Text exposes the full string even when the visual label is truncated. */
export function sidebarTextProps() {
  return { numberOfLines: 1, ellipsizeMode: "tail" as const };
}
