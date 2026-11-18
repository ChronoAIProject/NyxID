import { voiceMessageGroups } from "../../../../../shared/voiceMessageGroups";
import type { VoiceTranscriptMessage } from "./voiceTranscript";
export type GroupedVoiceMessage = VoiceTranscriptMessage & {
  voiceGroup?: readonly VoiceTranscriptMessage[];
  anchorId?: string;
};

export function voiceMessageIndex(
  rows: readonly GroupedVoiceMessage[],
  messageId: string,
): number {
  return rows.findIndex(
    (row) =>
      row.id === messageId ||
      row.voiceGroup?.some((segment) => segment.id === messageId),
  );
}
export function groupVoiceMessages(
  rows: readonly VoiceTranscriptMessage[],
): GroupedVoiceMessage[] {
  return voiceMessageGroups(rows, (row) => ({
    id: row.id,
    text: row.text,
    role: row.role,
    failed: ["failed", "error"].includes(row.status) || !!row.error_code,
    feedback: !!row.feedback,
    voice: row.voice,
    transient: row.voiceTransient,
  })).map((group) =>
    group.segments.length === 1
      ? group.anchor
      : {
          ...group.anchor,
          id: group.segments[0]!.id,
          anchorId: group.anchor.id,
          seq: group.segments[0]!.seq,
          created_at: group.segments[0]!.created_at,
          text: group.text,
          voice: group.segments.at(-1)!.voice,
          voiceGroup: group.segments,
        },
  );
}
