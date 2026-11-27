import test from "node:test";
import assert from "node:assert/strict";
import {
  prepareVoiceRippleFrames,
  voiceRippleCacheInfo,
  voiceRippleDots,
  cachedVoiceRippleFrames,
} from "../../../../../shared/agent-avatars/voice-ripple";
test("voice preparation yields, cancels stale palettes, and keeps one bounded deterministic frame bank", async () => {
  let yielded = false;
  setTimeout(() => {
    yielded = true;
  }, 0);
  const stale = prepareVoiceRippleFrames("nyx");
  const rejected = assert.rejects(stale, /cancelled/);
  const bank = await prepareVoiceRippleFrames("sunrise");
  await rejected;
  assert.ok(yielded);
  assert.equal(bank.frames.length, 12);
  assert.equal(new Set(bank.frames).size, 12);
  assert.equal(bank.still, bank.frames[3]);
  assert.equal(cachedVoiceRippleFrames("nyx"), undefined);
  assert.equal(await prepareVoiceRippleFrames("sunrise"), bank);
  assert.ok(voiceRippleCacheInfo().bytes <= voiceRippleCacheInfo().maxBytes);
  assert.equal(voiceRippleCacheInfo().palettes, 1);
  assert.equal(voiceRippleCacheInfo().preparing, 0);
  assert.ok(voiceRippleDots("lilac").length > 400);
});
