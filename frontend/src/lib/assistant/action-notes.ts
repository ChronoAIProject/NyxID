export const ACTION_REQUEST_CONFLICT_NOTE =
  "This action request was reissued with conflicting details. NyxID kept the first request and disabled this card.";

export const ACTION_REQUEST_UNREPORTED_COMPLETED_NOTE =
  "A service was connected in NyxID, but this action request could not notify the assistant. Review it in AI Services.";

export function composeUnreportedCompletedNote(
  status: "blocked" | "conflicted",
  outcomeNote: string,
): string {
  if (status !== "conflicted") {
    return ACTION_REQUEST_UNREPORTED_COMPLETED_NOTE;
  }
  if (outcomeNote.includes(ACTION_REQUEST_UNREPORTED_COMPLETED_NOTE)) {
    return outcomeNote;
  }
  const prefix = outcomeNote.includes(ACTION_REQUEST_CONFLICT_NOTE)
    ? ACTION_REQUEST_CONFLICT_NOTE
    : outcomeNote.trim() || ACTION_REQUEST_CONFLICT_NOTE;
  return `${prefix} ${ACTION_REQUEST_UNREPORTED_COMPLETED_NOTE}`.trim();
}

export function composeBlockedUnsupportedNote(
  blockedOutcomeNote: string,
  unsupportedOutcomeNote = "",
): string {
  const blocked = blockedOutcomeNote.trim();
  const unsupported = unsupportedOutcomeNote.trim();
  if (!unsupported) return blocked;
  if (!blocked) return unsupported;
  if (unsupported.includes(blocked)) return unsupported;
  if (blocked.includes(unsupported)) return blocked;
  return `${unsupported} ${blocked}`.trim();
}
