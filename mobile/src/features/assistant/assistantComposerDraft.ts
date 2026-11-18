/** Clear at dispatch, as Web does; only a definite refusal restores the text.
 * A reply/refresh can outlive new typing or navigation, so fence any restore. */
export async function submitAssistantDraft({
  draft,
  updateDraft,
  getRevision,
  send,
}: {
  draft: string;
  updateDraft: (text: string) => void;
  getRevision: () => number;
  /** False means pre-admission refusal; accepted or uncertain sends return true. */
  send: (text: string) => Promise<boolean>;
}) {
  const message = draft.trim();
  updateDraft("");
  const clearedRevision = getRevision();
  const restore = () => {
    if (getRevision() === clearedRevision) updateDraft(message);
  };
  try {
    if (!(await send(message))) restore();
  } catch (cause) {
    // The send hook only rejects for local guards before dispatch.
    restore();
    throw cause;
  }
}
