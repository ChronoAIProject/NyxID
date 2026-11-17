import { describe, expect, it } from "vitest";
import fixtures from "../../../../shared/voice-message-groups.json";
import { groupVoiceMessages } from "./voice-message-groups";
import type { ChatMessage } from "./chat-types";
describe("voice presentation shared contract", () => {
  for (const fixture of fixtures)
    it(fixture.name, () => {
      const rows = fixture.rows.map((row, index) => ({
        ...row,
        role: "role" in row ? row.role : "assistant",
        content: "text" in row ? row.text : row.id,
        timestamp: index,
        status:
          "status" in row && row.status === "failed" ? "error" : "complete",
      })) as ChatMessage[];
      const grouped = groupVoiceMessages(rows);
      expect(
        grouped.map(
          (row) => row.voiceGroup?.map((segment) => segment.id) ?? [row.id],
        ),
      ).toEqual(fixture.groups);
      if (fixture.anchors)
        expect(grouped.map((row) => row.anchorId ?? row.id)).toEqual(
          fixture.anchors,
        );
      if (fixture.copy)
        expect(grouped.map((row) => row.content)).toEqual(fixture.copy);
      expect(rows).toHaveLength(fixture.rows.length);
    });
});
