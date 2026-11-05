import assert from "node:assert/strict";
import test from "node:test";
import {
  createReplyReportApi,
  isReported,
  reportComment,
} from "./replyReportTransport";

const CONVERSATION = `nyxa-${"a".repeat(32)}`;
const GROUP = `nyxg-${"b".repeat(32)}`;

test("reports a reply through the existing feedback route as a tagged down vote", async () => {
  const calls: Array<{ path: string; options: any }> = [];
  const api = createReplyReportApi(async (path, options) => {
    calls.push({ path, options });
    return {};
  });
  await api.report(
    { kind: "conversation", id: CONVERSATION, messageId: "m/1" },
    "Offensive or abusive",
  );
  await api.report(
    { kind: "group", id: GROUP, messageId: "m2" },
    "Something else",
    "  it told me to ignore safety advice  ",
  );
  assert.deepEqual(calls, [
    {
      path: `/assistant/nyxagent/conversations/${CONVERSATION}/messages/m%2F1/feedback`,
      options: {
        method: "PUT",
        body: { vote: "down", comment: "Report: Offensive or abusive" },
      },
    },
    {
      path: `/assistant/nyxagent/groups/${GROUP}/messages/m2/feedback`,
      options: {
        method: "PUT",
        body: {
          vote: "down",
          comment: "Report: Something else. it told me to ignore safety advice",
        },
      },
    },
  ]);
});

test("rejects malformed parent ids before any request", async () => {
  let calls = 0;
  const api = createReplyReportApi(async () => {
    calls++;
  });
  await assert.rejects(
    api.report(
      { kind: "conversation", id: "../admin", messageId: "m" },
      "Harmful or unsafe",
    ),
  );
  await assert.rejects(
    api.report(
      { kind: "group", id: "nyxg-x", messageId: "m" },
      "Harmful or unsafe",
    ),
  );
  assert.equal(calls, 0);
});

test("report comments fit the 1,000-character feedback limit", () => {
  assert.equal(
    reportComment("Inaccurate or misleading", "x".repeat(2000)).length,
    1000,
  );
});

test("only tagged down votes count as reported", () => {
  assert.equal(
    isReported({ vote: "down", comment: "Report: Harmful or unsafe" }),
    true,
  );
  assert.equal(isReported({ vote: "down", comment: "too long" }), false);
  assert.equal(isReported({ vote: "up", comment: "Report: x" }), false);
  assert.equal(isReported(null), false);
});
