import type { AssistantMessage } from "../assistantTypes";
import type { VoiceSnapshot } from "./voiceContracts";

export type VoiceTranscriptMessage = AssistantMessage & {
  /** Snapshot captions have no server sequence/time; never send this row to an API. */
  voiceTransient?: true;
};

const terminal = (snapshot: VoiceSnapshot) =>
  snapshot.session.state === "closed" || snapshot.session.state === "failed";

/** Retain captions that have scrolled out of the bounded socket snapshot. */
export function retainVoiceSnapshot(
  previous: VoiceSnapshot | undefined,
  next: VoiceSnapshot,
  history: readonly AssistantMessage[],
): VoiceSnapshot {
  if (!previous || previous.session.id !== next.session.id) return next;
  const persisted = new Set(
    history.filter((row) => row.voice?.sealed).map((row) => row.id),
  );
  const captions = new Map(
    previous.captions
      .filter((row) => !persisted.has(row.id))
      .map((row) => [row.id, row]),
  );
  for (const row of next.captions) {
    const old = captions.get(row.id);
    // Final text must not regress if a snapshot is behind a completed segment.
    if (!old?.sealed || row.sealed) captions.set(row.id, row);
  }
  return { ...next, captions: [...captions.values()] };
}

/** Merge by the server's message/segment ID, keeping history sequence order. */
export function mergeVoiceTranscript(
  history: readonly AssistantMessage[],
  snapshot: VoiceSnapshot | undefined,
): VoiceTranscriptMessage[] {
  if (!snapshot) return [...history];
  const captions = new Map(snapshot.captions.map((row) => [row.id, row]));
  const rows: VoiceTranscriptMessage[] = history.map((row) => {
    const caption = captions.get(row.id);
    if (!caption || caption.speaker !== row.role || row.voice?.sealed)
      return row;
    if (row.voice && row.voice.session_id !== snapshot.session.id) return row;
    // Only voice rows can be overlaid; a caption must not rewrite ordinary chat.
    if (!row.voice && row.via !== "voice") return row;
    return {
      ...row,
      text: caption.text,
      voice: {
        session_id: snapshot.session.id,
        segment_id: caption.id,
        sealed: caption.sealed,
        complete: caption.complete,
      },
    };
  });
  for (const [index, caption] of snapshot.captions.entries()) {
    if (rows.some((row) => row.id === caption.id)) continue;
    if (!caption.text.trim() && (caption.sealed || terminal(snapshot)))
      continue;
    const row: VoiceTranscriptMessage = {
      id: caption.id,
      // Unknown snapshot metadata stays local and is excluded from gap/footer UI.
      seq: 0,
      turn_id: "",
      role: caption.speaker,
      text: caption.text,
      status: "completed",
      error_code: null,
      created_at: snapshot.session.created_at,
      activities: [],
      attachments: [],
      via: "voice",
      voiceTransient: true,
      voice: {
        session_id: snapshot.session.id,
        segment_id: caption.id,
        sealed: caption.sealed,
        complete: caption.complete,
      },
    };
    // A caption already in history anchors unknown rows on either side. Real
    // task/approval messages retain their server order and are never replaced.
    const following = snapshot.captions
      .slice(index + 1)
      .find((entry) => rows.some((known) => known.id === entry.id));
    if (following)
      rows.splice(
        rows.findIndex((known) => known.id === following.id),
        0,
        row,
      );
    else rows.push(row);
  }
  return rows;
}

export function voiceTranscriptLabel(
  row: AssistantMessage,
  snapshot: VoiceSnapshot | undefined,
): string | undefined {
  if (!row.voice) return;
  const live =
    snapshot?.session.id === row.voice.session_id &&
    !terminal(snapshot) &&
    !snapshot.session.end_requested;
  if (!row.voice.sealed)
    return live
      ? row.role === "user"
        ? "Transcribing…"
        : "Speaking…"
      : "Partial transcript";
  return row.voice.complete ? undefined : "Partial transcript";
}

/**
 * While a call is live, the voice composer is the only place that shows the
 * user's unsent words. Their sealed turns, and every assistant caption, still
 * join the chat transcript.
 */
export function withoutLiveUserSpeech(
  snapshot: VoiceSnapshot | undefined,
): VoiceSnapshot | undefined {
  if (!snapshot || terminal(snapshot)) return snapshot;
  const captions = snapshot.captions.filter(
    (row) => row.speaker !== "user" || row.sealed,
  );
  return captions.length === snapshot.captions.length
    ? snapshot
    : { ...snapshot, captions };
}
