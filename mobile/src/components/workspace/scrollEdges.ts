export type ScrollEdges = { start: boolean; end: boolean };
export function scrollEdges(
  viewport: number,
  content: number,
  offset: number,
): ScrollEdges {
  if (viewport <= 0 || content <= viewport + 2)
    return { start: false, end: false };
  const max = Math.max(0, content - viewport);
  const position = Math.min(max, Math.max(0, offset));
  return { start: position > 2, end: max - position > 2 };
}
