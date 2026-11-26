import assert from "node:assert/strict";
import test from "node:test";
import fixtures from "../../../../../shared/voice-message-groups.json";
import { groupVoiceMessages, voiceMessageIndex } from "./voiceMessageGroups";
import type { VoiceTranscriptMessage } from "./voiceTranscript";
for (const fixture of fixtures)
  test(fixture.name, () => {
    const rows = fixture.rows.map((row, index) => ({
      ...row,
      seq: index + 1,
      role: "role" in row ? row.role : "assistant",
      text: "text" in row ? row.text : row.id,
      status: "status" in row ? row.status : "completed",
      turn_id: "turn_id" in row ? row.turn_id : "ignored",
      created_at: "2026-11-14T00:00:00Z",
      activities: [],
      attachments: [],
      error_code: null,
    })) as VoiceTranscriptMessage[];
    const grouped = groupVoiceMessages(rows);
    assert.deepEqual(
      grouped.map(
        (row) => row.voiceGroup?.map((segment) => segment.id) ?? [row.id],
      ),
      fixture.groups,
    );
    if (fixture.anchors)
      assert.deepEqual(
        grouped.map((row) => row.anchorId ?? row.id),
        fixture.anchors,
      );
    if (fixture.copy)
      assert.deepEqual(
        grouped.map((row) => row.text),
        fixture.copy,
      );
    assert.equal(rows.length, fixture.rows.length);
    for (const [index, ids] of fixture.groups.entries()) {
      for (const id of ids) assert.equal(voiceMessageIndex(grouped, id), index);
    }
    assert.equal(voiceMessageIndex(grouped, "missing-message"), -1);
  });

test("transient live captions break persistent speech groups", () => {
  const rows = ["a", "live", "b"].map((id, index) => ({
    id,
    seq: index,
    role: "assistant",
    text: id,
    status: "completed",
    voiceTransient: id === "live" ? true : undefined,
    voice: { session_id: "s", segment_id: id, sealed: true, complete: true },
  })) as VoiceTranscriptMessage[];
  assert.deepEqual(
    groupVoiceMessages(rows).map((row) => row.id),
    ["a", "live", "b"],
  );
});
