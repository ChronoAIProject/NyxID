import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import * as scrollFollower from "./assistantTranscriptScroll";
import { hookHarness } from "./assistantHookTestHarness";
import type { useTranscriptScrollFollow } from "./useTranscriptScrollFollow";

function harness() {
  let scope = "conversation-1";
  const offsets: number[] = [];
  const list = {
    current: {
      scrollToOffset: ({ offset }: { offset: number; animated: boolean }) => {
        offsets.push(offset);
      },
    },
  };
  const h = hookHarness<ReturnType<typeof useTranscriptScrollFollow>>({
    "./assistantTranscriptScroll": scrollFollower,
  });
  const hook = h.load(
    fileURLToPath(new URL("./useTranscriptScrollFollow.ts", import.meta.url)),
  ).useTranscriptScrollFollow as typeof useTranscriptScrollFollow;
  h.mount(() => hook(list, scope));
  return {
    h,
    offsets,
    switchChat: (next = "conversation-2") => {
      scope = next;
      return h.render();
    },
  };
}

const layout = (height: number) => ({ nativeEvent: { layout: { height } } });
const scroll = (height: number, viewport: number, y: number) => ({
  nativeEvent: {
    contentOffset: { y },
    contentSize: { height },
    layoutMeasurement: { height: viewport },
  },
});

test("native callbacks keep following across a saved reply and keyboard resize", () => {
  const x = harness();
  const props = x.h.value().listProps;
  props.onLayout(layout(400));
  props.onContentSizeChange(300, 1000);
  props.onScroll(scroll(1000, 400, 300));
  props.onContentSizeChange(300, 200);
  props.onScroll(scroll(200, 400, 0));
  props.onContentSizeChange(300, 1100);
  props.onLayout(layout(250));
  props.onScroll(scroll(1100, 250, 700));
  assert.equal(x.offsets.at(-1), 850);
  assert.equal(x.h.flush().following, true);
});

test("user history reading stops corrections until sending resumes following", () => {
  const x = harness();
  let props = x.h.value().listProps;
  props.onLayout(layout(400));
  props.onContentSizeChange(300, 1000);
  props.onScrollBeginDrag();
  props.onScrollEndDrag(scroll(1000, 400, 100));
  assert.equal(x.h.flush().following, false);
  const previous = x.offsets.length;
  props = x.h.value().listProps;
  props.onContentSizeChange(300, 1200);
  props.onLayout(layout(250));
  props.onScroll(scroll(1200, 250, 200));
  assert.equal(x.offsets.length, previous);
  x.h.value().follow();
  assert.equal(x.offsets.at(-1), 950);
  assert.equal(x.h.flush().following, true);
});

test("stale previous-chat callbacks cannot change the new transcript", () => {
  const x = harness();
  const stale = x.h.value();
  stale.listProps.onLayout(layout(400));
  stale.listProps.onContentSizeChange(300, 1000);
  stale.pause();
  x.h.flush();
  const current = x.switchChat();
  const previous = x.offsets.length;
  stale.listProps.onLayout(layout(200));
  stale.listProps.onContentSizeChange(300, 3000);
  stale.listProps.onScrollBeginDrag();
  stale.pause();
  stale.follow();
  assert.equal(x.offsets.length, previous);
  current.listProps.onContentSizeChange(300, 600);
  assert.equal(x.offsets.length, previous);
  current.listProps.onLayout(layout(400));
  assert.equal(x.offsets.at(-1), 200);
  assert.equal(x.h.flush().following, true);
});

test("unmounted native callbacks cannot scroll a replacement list", () => {
  const x = harness();
  const stale = x.h.value();
  stale.listProps.onLayout(layout(400));
  stale.listProps.onContentSizeChange(300, 1000);
  x.h.unmount();
  const previous = x.offsets.length;
  stale.listProps.onContentSizeChange(300, 3000);
  stale.listProps.onLayout(layout(200));
  stale.follow();
  assert.equal(x.offsets.length, previous);
});

test("reopening a chat still rejects its previous list instance's callbacks", () => {
  const x = harness();
  const stale = x.h.value();
  x.switchChat();
  const current = x.switchChat("conversation-1");
  current.listProps.onContentSizeChange(300, 600);
  current.listProps.onLayout(layout(400));
  const previous = x.offsets.length;
  stale.listProps.onLayout(layout(200));
  stale.listProps.onContentSizeChange(300, 3000);
  stale.pause();
  stale.follow();
  assert.equal(x.offsets.length, previous);
  assert.equal(x.h.flush().following, true);
});

test("native fling metadata prevents content corrections between drag release and momentum begin", () => {
  for (const nativeRelease of [
    { velocity: { y: -0.5 } },
    { targetContentOffset: { y: 100 } },
  ]) {
    const x = harness();
    const props = x.h.value().listProps;
    props.onLayout(layout(400));
    props.onContentSizeChange(300, 1000);
    props.onScrollBeginDrag();
    props.onScrollEndDrag({
      nativeEvent: { ...scroll(1000, 400, 560).nativeEvent, ...nativeRelease },
    });
    const previous = x.offsets.length;
    props.onContentSizeChange(300, 1200);
    assert.equal(x.offsets.length, previous);
    assert.equal(x.h.flush().following, false);
    props.onMomentumScrollBegin();
    props.onMomentumScrollEnd(scroll(1200, 400, 100));
    assert.equal(x.h.flush().following, false);
  }
});

test("a live footer disappearing during a history drag does not pull the reader to saved rows", () => {
  const x = harness();
  const props = x.h.value().listProps;
  props.onLayout(layout(400));
  props.onContentSizeChange(300, 1200);
  props.onScrollBeginDrag();
  props.onScroll(scroll(1200, 400, 400));
  const previous = x.offsets.length;
  props.onContentSizeChange(300, 260);
  props.onScrollEndDrag(scroll(260, 400, 0));
  props.onContentSizeChange(300, 1000);
  assert.equal(x.offsets.length, previous);
  assert.equal(x.h.flush().following, false);
});

test("jump only appears away from latest, and resumes follow after explicit activation", () => {
  const x = harness();
  let p = x.h.value().listProps;
  p.onLayout(layout(400));
  p.onContentSizeChange(300, 1200);
  p.onScroll(scroll(1200, 400, 800));
  x.h.value().pause();
  assert.equal(x.h.flush().showJump, false);
  p.onScrollBeginDrag();
  p.onScrollEndDrag(scroll(1200, 400, 100));
  p.onScroll(scroll(1200, 400, 100));
  assert.equal(x.h.flush().showJump, true);
  x.h.value().follow();
  assert.equal(x.h.flush().showJump, false);
  assert.equal(x.offsets.at(-1), 800);
  x.h.unmount();
});
