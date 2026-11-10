import assert from "node:assert/strict";
import test from "node:test";
import { messageSchema } from "../assistantTypes";
import { voiceSnapshotSchema } from "./voiceContracts";
import {
  mergeVoiceTranscript,
  retainVoiceSnapshot,
  voiceTranscriptLabel,
} from "./voiceTranscript";

function message(
  id: string,
  seq: number,
  text: string,
  voice?: { sealed: boolean; complete: boolean },
) {
  return messageSchema.parse({
    id,
    seq,
    text,
    turn_id: `turn-${id}`,
    role: "user",
    status: "completed",
    error_code: null,
    created_at: "2026-11-09T01:00:00Z",
    via: voice ? "voice" : null,
    voice: voice
      ? {
          session_id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
          segment_id: id,
          ...voice,
        }
      : undefined,
  });
}
function snapshot(
  captions: Array<{
    id: string;
    text: string;
    speaker?: "user" | "assistant";
    sealed?: boolean;
    complete?: boolean;
  }>,
) {
  return voiceSnapshotSchema.parse({
    type: "snapshot",
    session: {
      id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
      created_at: "2026-11-09T01:00:00Z",
      closed_at: null,
      generation: 1,
      state: "active",
      muted: false,
      input_muted: false,
      end_requested: false,
      control_revision: 1,
      observed_seconds: 2,
      reserved_until: 30,
      final_usage_confirmed: false,
      end_reason: null,
      idle_warning: false,
    },
    captions: captions.map((row) => ({
      speaker: "user",
      sealed: false,
      complete: false,
      ...row,
    })),
    tasks: [],
  });
}

test("history retains the existing server voice segment metadata", () => {
  assert.equal(
    message("u", 1, "hello", { sealed: false, complete: false }).voice
      ?.segment_id,
    "u",
  );
  assert.equal(message("text", 1, "hello").voice, undefined);
});

test("partial snapshots update one bubble and history takes it over by message ID", () => {
  const first = snapshot([{ id: "u", text: "上午" }]);
  assert.equal(mergeVoiceTranscript([], first)[0]?.text, "上午");
  const history = [message("u", 1, "上午", { sealed: false, complete: false })];
  const updated = mergeVoiceTranscript(
    history,
    snapshot([{ id: "u", text: "上午写方案" }]),
  );
  assert.equal(updated.length, 1);
  assert.equal(updated[0]?.id, "u");
  assert.equal(updated[0]?.text, "上午写方案");
  assert.equal(voiceTranscriptLabel(updated[0]!, first), "Transcribing…");
  const sealed = message("u", 1, "上午写方案。", {
    sealed: true,
    complete: true,
  });
  const final = mergeVoiceTranscript([sealed], first);
  assert.equal(final[0], sealed);
  assert.equal(voiceTranscriptLabel(final[0]!, first), undefined);
});

test("unpersisted captions use no invented sequence or timestamp footer and retain caption order", () => {
  const s = snapshot([
    { id: "a", text: "first" },
    { id: "b", text: "reply", speaker: "assistant" },
    { id: "c", text: "next" },
  ]);
  const rows = mergeVoiceTranscript(
    [message("b", 5, "reply", { sealed: false, complete: false })],
    s,
  );
  // A role mismatch is never rewritten; IDs still prevent duplicates.
  assert.deepEqual(
    rows.map((row) => row.id),
    ["a", "b", "c"],
  );
  assert.equal(rows[0]?.voiceTransient, true);
  assert.equal(rows[0]?.seq, 0);
  assert.equal(rows[1]?.seq, 5);
});

test("captions cannot overwrite a normal message or another session's voice row", () => {
  const ordinary = message("u", 1, "normal");
  assert.equal(
    mergeVoiceTranscript([ordinary], snapshot([{ id: "u", text: "wrong" }]))[0],
    ordinary,
  );
  const other = message("u", 1, "other session", {
    sealed: false,
    complete: false,
  });
  other.voice!.session_id = "another-session";
  assert.equal(
    mergeVoiceTranscript([other], snapshot([{ id: "u", text: "wrong" }]))[0],
    other,
  );
});

test("captions falling out of the 16-row socket window remain until history catches up", () => {
  const old = snapshot([
    { id: "old", text: "finished", sealed: true, complete: true },
  ]);
  const next = snapshot([{ id: "new", text: "new words" }]);
  const retained = retainVoiceSnapshot(old, next, []);
  assert.deepEqual(
    retained.captions.map((row) => row.id),
    ["old", "new"],
  );
  const persisted = message("old", 1, "finished", {
    sealed: true,
    complete: true,
  });
  assert.deepEqual(
    retainVoiceSnapshot(retained, next, [persisted]).captions.map(
      (row) => row.id,
    ),
    ["new"],
  );
});

test("final captions cannot regress and a new session never inherits old captions", () => {
  const old = snapshot([
    { id: "u", text: "final", sealed: true, complete: true },
  ]);
  const next = snapshot([{ id: "u", text: "partial" }]);
  assert.equal(retainVoiceSnapshot(old, next, []).captions[0]?.text, "final");
  next.session.id = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
  assert.equal(retainVoiceSnapshot(old, next, []), next);
});

test("closing and incomplete segments stay truthful and empty terminal captions are omitted", () => {
  const s = snapshot([
    { id: "u", text: "unfinished" },
    { id: "empty", text: "" },
  ]);
  s.session.end_requested = true;
  assert.equal(
    voiceTranscriptLabel(mergeVoiceTranscript([], s)[0]!, s),
    "Partial transcript",
  );
  s.session.state = "closed";
  assert.deepEqual(
    mergeVoiceTranscript([], s).map((row) => row.id),
    ["u"],
  );
  const row = message("u", 1, "unfinished", { sealed: true, complete: false });
  assert.equal(voiceTranscriptLabel(row, s), "Partial transcript");
});
