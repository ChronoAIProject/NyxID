export function transcriptEndOffset(
  contentHeight: number,
  viewportHeight: number,
): number {
  return Math.max(0, contentHeight - viewportHeight);
}
