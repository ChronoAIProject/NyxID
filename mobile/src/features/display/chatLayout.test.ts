import assert from "node:assert/strict";
import test from "node:test";
import {
  composerHeightLimit,
  defaultChatLayout,
  normalizeChatLayout,
  offersChatWidth,
  transcriptColumnStyle,
} from "./chatLayout";

test("missing or corrupt device-local layout values retain native defaults", () => {
  for (const value of [
    null,
    undefined,
    "bad",
    [],
    { columnWidth: NaN, composerMaxHeight: "300" },
    { columnWidth: Infinity, composerMaxHeight: -Infinity },
  ])
    assert.deepEqual(normalizeChatLayout(value), defaultChatLayout);
  assert.deepEqual(
    normalizeChatLayout({ columnWidth: 10, composerMaxHeight: 9999 }),
    { columnWidth: 360, composerMaxHeight: 320 },
  );
});
test("composer maximum caps to short keyboard windows without forcing height", () => {
  assert.equal(composerHeightLimit(120, 844), 120);
  assert.equal(composerHeightLimit(320, 390), 92);
  assert.equal(composerHeightLimit(320, 260), 44);
  assert.equal(
    composerHeightLimit(120, 844),
    defaultChatLayout.composerMaxHeight,
  );
  assert.equal(offersChatWidth(393), false);
  assert.equal(offersChatWidth(600), true);
});

test("portrait and wide-window transcripts retain the prior message width inside an edge-to-edge viewport", () => {
  for (const width of [320, 393, 402, 430, 768, 1024, 1440]) {
    for (const column of [360, 758, 1200]) {
      const content = transcriptColumnStyle(column);
      const messageWidth =
        Math.min(width, content.maxWidth) - content.paddingHorizontal * 2;
      assert.equal(messageWidth, Math.min(width - 24, column));
      assert.equal(content.alignSelf, "center");
    }
  }
});
