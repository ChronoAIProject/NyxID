import { voiceMessageGroups } from "../../../../shared/voiceMessageGroups";
import type { ChatMessage } from "./chat-types";
export function groupVoiceMessages(
  rows: readonly ChatMessage[],
): readonly ChatMessage[] {
  return voiceMessageGroups(rows, (row) => ({
    id: row.id,
    text: row.content,
    role: row.role,
    failed: !!row.errorCode || ["error", "failed"].includes(row.status),
    feedback: !!row.feedback,
    voice: row.voice,
  })).map((group) =>
    group.segments.length === 1
      ? group.anchor
      : {
          ...group.anchor,
          id: group.segments[0]!.id,
          anchorId: group.anchor.id,
          timestamp: group.segments[0]!.timestamp,
          content: group.text,
          voiceGroup: group.segments,
        },
  );
}
