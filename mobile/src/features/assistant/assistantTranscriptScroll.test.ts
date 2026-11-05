import assert from "node:assert/strict";
import test from "node:test";
import {
  createTranscriptScrollFollower,
  transcriptEndOffset,
} from "./assistantTranscriptScroll";

test("a short persisted reply remains visible after the live footer shrinks", () => {
  const viewport = 420;
  const liveContent = 900;
  assert.equal(transcriptEndOffset(liveContent, viewport), 480);
  const savedContent = 260;
  assert.equal(transcriptEndOffset(savedContent, viewport), 0);
});

test("following variable-height reply growth uses its measured bottom", () => {
  const viewport = 400;
  assert.deepEqual(
    [180, 430, 710, 560].map((height) => transcriptEndOffset(height, viewport)),
    [0, 30, 310, 160],
  );
  assert.equal(transcriptEndOffset(560, 280), 280);
});

test("passive native scroll events cannot interrupt live reply following", () => {
  const follower = createTranscriptScrollFollower();
  follower.layout(400);
  follower.contentSize(900);
  follower.scroll({ contentHeight: 900, viewportHeight: 400, offset: 200 });
  assert.equal(follower.targetOffset(), 500);

  // The live footer is removed before the saved rows finish measuring.
  follower.contentSize(260);
  follower.scroll({ contentHeight: 260, viewportHeight: 400, offset: 0 });
  assert.equal(follower.targetOffset(), 0);
  follower.contentSize(1000);
  follower.scroll({ contentHeight: 1000, viewportHeight: 400, offset: 0 });
  assert.equal(follower.isFollowing(), true);
  assert.equal(follower.targetOffset(), 600);
});

test("keyboard geometry updates keep the newest reply visible", () => {
  const follower = createTranscriptScrollFollower();
  follower.layout(600);
  follower.contentSize(1200);
  assert.equal(follower.targetOffset(), 600);
  // Native offset and viewport events can arrive in either order.
  follower.scroll({ contentHeight: 1200, viewportHeight: 300, offset: 600 });
  follower.layout(300);
  assert.equal(follower.targetOffset(), 900);
  follower.layout(600);
  follower.scroll({ contentHeight: 1200, viewportHeight: 600, offset: 900 });
  assert.equal(follower.isFollowing(), true);
  assert.equal(follower.targetOffset(), 600);
});

test("a reader dragging into history remains there through layout and new replies", () => {
  const follower = createTranscriptScrollFollower();
  follower.layout(400);
  follower.contentSize(1200);
  follower.beginDrag();
  follower.scroll({ contentHeight: 1200, viewportHeight: 400, offset: 400 });
  assert.equal(follower.targetOffset(), null);
  follower.endDrag({ contentHeight: 1200, viewportHeight: 400, offset: 400 });
  follower.layout(300);
  follower.contentSize(1400);
  follower.scroll({ contentHeight: 1400, viewportHeight: 300, offset: 400 });
  assert.equal(follower.isFollowing(), false);
  assert.equal(follower.targetOffset(), null);
});

test("user momentum finishing at the bottom resumes measured following", () => {
  const follower = createTranscriptScrollFollower();
  follower.layout(400);
  follower.contentSize(1200);
  follower.beginDrag();
  follower.endDrag({ contentHeight: 1200, viewportHeight: 400, offset: 400 });
  follower.beginMomentum();
  follower.scroll({ contentHeight: 1200, viewportHeight: 400, offset: 790 });
  assert.equal(follower.targetOffset(), null);
  follower.endMomentum({
    contentHeight: 1200,
    viewportHeight: 400,
    offset: 800,
  });
  assert.equal(follower.isFollowing(), true);
  follower.contentSize(1400);
  assert.equal(follower.targetOffset(), 1000);
});

test("opening earlier messages or disclosure stays paused when rows shrink", () => {
  const follower = createTranscriptScrollFollower();
  follower.layout(400);
  follower.contentSize(1200);
  follower.pause();
  follower.contentSize(300);
  follower.scroll({ contentHeight: 300, viewportHeight: 400, offset: 0 });
  follower.beginMomentum();
  follower.endMomentum({ contentHeight: 300, viewportHeight: 400, offset: 0 });
  assert.equal(follower.isFollowing(), false);
  assert.equal(follower.targetOffset(), null);
});

test("sending explicitly resumes following and ignores the old user gesture", () => {
  const follower = createTranscriptScrollFollower();
  follower.layout(400);
  follower.contentSize(1200);
  follower.beginDrag();
  follower.endDrag({ contentHeight: 1200, viewportHeight: 400, offset: 200 });
  follower.follow();
  follower.beginMomentum();
  follower.endMomentum({
    contentHeight: 1200,
    viewportHeight: 400,
    offset: 250,
  });
  assert.equal(follower.isFollowing(), true);
  assert.equal(follower.targetOffset(), 800);
});

test("switching conversations clears previous geometry and reading intent", () => {
  const follower = createTranscriptScrollFollower();
  follower.layout(400);
  follower.contentSize(1200);
  follower.beginDrag();
  follower.reset();
  assert.equal(follower.isFollowing(), true);
  assert.equal(follower.targetOffset(), null);
  follower.contentSize(200);
  follower.layout(500);
  assert.equal(follower.targetOffset(), 0);
});

test("an upward fling near bottom stays paused before native momentum begins", () => {
  for (const nativeRelease of [{ velocityY: -0.5 }, { targetOffset: 300 }]) {
    const follower = createTranscriptScrollFollower();
    follower.layout(400);
    follower.contentSize(1200);
    follower.beginDrag();
    follower.endDrag({
      contentHeight: 1200,
      viewportHeight: 400,
      offset: 760,
      ...nativeRelease,
    });
    follower.contentSize(1400);
    assert.equal(follower.isFollowing(), false);
    assert.equal(follower.targetOffset(), null);
    follower.beginMomentum();
    follower.endMomentum({
      contentHeight: 1400,
      viewportHeight: 400,
      offset: 300,
    });
    assert.equal(follower.targetOffset(), null);
  }
});

test("content contraction during history dragging cannot resume following from a clamped offset", () => {
  const follower = createTranscriptScrollFollower();
  follower.layout(400);
  follower.contentSize(1200);
  follower.beginDrag();
  follower.scroll({ contentHeight: 1200, viewportHeight: 400, offset: 400 });
  follower.contentSize(260);
  follower.endDrag({ contentHeight: 260, viewportHeight: 400, offset: 0 });
  follower.contentSize(1000);
  assert.equal(follower.isFollowing(), false);
  assert.equal(follower.targetOffset(), null);
  // A later deliberate gesture to the actual saved transcript end can resume.
  follower.beginDrag();
  follower.endDrag({ contentHeight: 1000, viewportHeight: 400, offset: 600 });
  assert.equal(follower.targetOffset(), 600);
});

test("native contraction reported at momentum end does not count as reaching bottom", () => {
  const follower = createTranscriptScrollFollower();
  follower.layout(400);
  follower.contentSize(1200);
  follower.beginDrag();
  follower.endDrag({
    contentHeight: 1200,
    viewportHeight: 400,
    offset: 760,
    velocityY: -1,
  });
  follower.beginMomentum();
  follower.endMomentum({ contentHeight: 260, viewportHeight: 400, offset: 0 });
  follower.contentSize(1000);
  assert.equal(follower.isFollowing(), false);
  assert.equal(follower.targetOffset(), null);
});

test("a release clamped at the actual bottom does not wait for nonexistent momentum", () => {
  const follower = createTranscriptScrollFollower();
  follower.layout(400);
  follower.contentSize(1200);
  follower.beginDrag();
  follower.endDrag({
    contentHeight: 1200,
    viewportHeight: 400,
    offset: 800,
    velocityY: 0.5,
    targetOffset: 800,
  });
  assert.equal(follower.targetOffset(), 800);
});
