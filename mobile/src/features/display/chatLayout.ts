export const TRANSCRIPT_SIDE_PADDING = 12;

export type ChatLayout = { columnWidth: number; composerMaxHeight: number };
export const defaultChatLayout: ChatLayout = {
  columnWidth: 758,
  composerMaxHeight: 120,
};
export const chatLayoutBounds = {
  columnWidth: [360, 1200],
  composerMaxHeight: [80, 320],
} as const;
const bounded = (
  value: unknown,
  fallback: number,
  [min, max]: readonly [number, number],
) =>
  typeof value === "number" && Number.isFinite(value)
    ? Math.round(Math.min(max, Math.max(min, value)))
    : fallback;
export function normalizeChatLayout(raw: unknown): ChatLayout {
  const data =
    raw && typeof raw === "object" ? (raw as Partial<ChatLayout>) : {};
  return {
    columnWidth: bounded(
      data.columnWidth,
      defaultChatLayout.columnWidth,
      chatLayoutBounds.columnWidth,
    ),
    composerMaxHeight: bounded(
      data.composerMaxHeight,
      defaultChatLayout.composerMaxHeight,
      chatLayoutBounds.composerMaxHeight,
    ),
  };
}
export function composerHeightLimit(
  preference: number,
  availableHeight: number,
) {
  return Math.max(
    44,
    Math.min(preference, Math.floor(Math.max(0, availableHeight - 160) * 0.4)),
  );
}
export function offersChatWidth(windowWidth: number) {
  return windowWidth >= 600;
}

/** The viewport stays at the safe-area edge; only its content is inset. */
export function transcriptColumnStyle(
  columnWidth = defaultChatLayout.columnWidth,
) {
  return {
    width: "100%" as const,
    maxWidth: columnWidth + 2 * TRANSCRIPT_SIDE_PADDING,
    alignSelf: "center" as const,
    paddingHorizontal: TRANSCRIPT_SIDE_PADDING,
  };
}
