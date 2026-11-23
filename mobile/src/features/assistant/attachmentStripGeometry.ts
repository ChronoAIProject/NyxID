export const ATTACHMENT_GAP = 8;
export const ATTACHMENT_INSET = 4;
const MIN_ATTACHMENT_TILE = 64;

/** The viewport is measured inside the composer, never from window width. */
export function attachmentStripGeometry(
  width: number,
  count: number,
  offset: number,
) {
  const tile = width > 0
    ? Math.max(
        MIN_ATTACHMENT_TILE,
        Math.floor((width - 2 * ATTACHMENT_INSET - 2 * ATTACHMENT_GAP) / 3),
      )
    : 0;
  const stride = tile + ATTACHMENT_GAP;
  const content =
    2 * ATTACHMENT_INSET +
    count * tile +
    Math.max(0, count - 1) * ATTACHMENT_GAP;
  const maxOffset = Math.max(0, content - width);
  const x = Math.max(0, Math.min(offset, maxOffset));
  const overflow = tile > 0 && count > 0 && maxOffset > 2;
  // Include tiles with visible pixels, excluding a gap touching the viewport.
  const start = Math.min(
    count,
    Math.max(
      1,
      Math.floor((x + ATTACHMENT_GAP - ATTACHMENT_INSET) / stride) + 1,
    ),
  );
  const end = Math.min(
    count,
    Math.max(start, Math.ceil((x + width - ATTACHMENT_INSET) / stride)),
  );
  return {
    tile,
    content,
    maxOffset,
    offset: x,
    overflow,
    start,
    end,
    before: overflow && x > 2,
    after: overflow && maxOffset - x > 2,
  };
}
