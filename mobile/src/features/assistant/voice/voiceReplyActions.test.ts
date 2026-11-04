import assert from "node:assert/strict";
import test from "node:test";
import { hasAssistantActionLink } from "./voiceReplyActions";

const origin = "https://nyx.example.test";
const token = `nyx_clk_${"a".repeat(43)}`;

test("replies with in-app actions join the voice chat; plain replies do not", () => {
  assert.equal(
    hasAssistantActionLink(`Connect it here: [Connect Gmail](${origin}/connect/${token}).`, origin),
    true,
  );
  assert.equal(
    hasAssistantActionLink(`Open ${origin}/channel-bots/connect/telegram?label=Bot.`, origin),
    true,
  );
  assert.equal(hasAssistantActionLink("Done. Nothing else to set up.", origin), false);
  assert.equal(
    hasAssistantActionLink("Read https://example.com/connect/abc for details.", origin),
    false,
    "third-party links are not in-app actions",
  );
});
