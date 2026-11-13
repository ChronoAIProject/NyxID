/** One line of 14/20pt prose with the normal field’s 12/10pt insets. */
export function composerInputMinHeight(fontScale = 1) {
  return Math.max(44, 20 * fontScale + 22);
}

/** The full-width field owns its toolbar; only the crew recipient strip adds a gap. */
export function composerVoiceMinHeight(fontScale = 1, crew = false) {
  const recipientStrip = crew ? Math.max(36, 17 * fontScale + 14) + 6 : 0;
  return composerInputMinHeight(fontScale) + 44 + recipientStrip;
}
