import { submitAssistantDraft } from "../assistant/assistantComposerDraft";
import { NO_RECIPIENT_EDITS, type RecipientSelection } from "./groupState";

/** Text and recipient pills share the same revision, including manual edits. */
export function submitGroupDraft({
  draft,
  selection,
  updateComposer,
  getRevision,
  send,
}: {
  draft: string;
  selection: RecipientSelection;
  updateComposer: (text: string, selection: RecipientSelection) => void;
  getRevision: () => number;
  send: (text: string) => Promise<boolean>;
}) {
  const savedSelection: RecipientSelection = {
    mentioned: [...selection.mentioned],
    added: [...selection.added],
    removed: [...selection.removed],
  };
  let firstUpdate = true;
  return submitAssistantDraft({
    draft,
    getRevision,
    send,
    updateDraft: (text) => {
      const nextSelection = firstUpdate ? NO_RECIPIENT_EDITS : savedSelection;
      firstUpdate = false;
      updateComposer(text, nextSelection);
    },
  });
}
