/** Header Back and Android's modal Back must follow the same local hierarchy. */
export function skillModalBackAction(
  childView: boolean,
  mutationPending: boolean,
): "stay" | "parent" | "close" {
  if (mutationPending) return "stay";
  return childView ? "parent" : "close";
}
