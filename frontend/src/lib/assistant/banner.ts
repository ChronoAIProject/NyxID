/**
 * Transcript banners (turn errors, NyxID notices, activity summaries, chat
 * notices) share one frame and one row so they match in height, radius and
 * where their icon and text start.
 */
export const BANNER_FRAME = "overflow-hidden rounded-lg border text-11";
/** Tool-call receipts (machine tool cards). */
export const RECEIPT_MAX_W = "max-w-[900px]";
/** The two transcript accordions, kept the same width: a turn's summary and the NyxID notices that woke it. */
export const ACCORDION_MAX_W = "max-w-[680px]";
/** Every other transcript status card: errors, notices, CMA, connect, approval and action cards. */
export const STATUS_CARD_MAX_W = "max-w-[600px]";
export const BANNER_NEUTRAL = "border-hairline bg-overlay/35";
export const BANNER_DESTRUCTIVE = "border-destructive/25 bg-destructive/[0.05] text-destructive";
/** One line tall (32px); longer content grows from there. */
export const BANNER_ROW = "flex min-h-8 w-full items-center gap-2 px-3 py-1.5 text-left";
/** Leading icon slot; trailing disclosure chevrons use `BANNER_CHEVRON`. */
export const BANNER_ICON = "flex h-3.5 w-3.5 shrink-0 items-center justify-center";
export const BANNER_CHEVRON = "h-3 w-3 shrink-0 text-text-tertiary transition-transform";
