export function transcriptEndOffset(
  contentHeight: number,
  viewportHeight: number,
): number {
  return Math.max(0, contentHeight - viewportHeight);
}

export type TranscriptScrollSample = {
  contentHeight: number;
  viewportHeight: number;
  offset: number;
  velocityY?: number;
  targetOffset?: number;
};

/**
 * Following is a user's intent, not the distance reported by every native
 * scroll event. Layout, keyboard and virtualized-row corrections also produce
 * scroll events whose offsets may precede the latest measured content size.
 */
export function createTranscriptScrollFollower(bottomThreshold = 80) {
  let following = true;
  let contentHeight = 0;
  let offset = 0;
  let viewportHeight = 0;
  let gesture: "idle" | "drag" | "momentum" = "idle";
  let userMomentumAllowed = false;
  let lastGestureSample: TranscriptScrollSample | undefined;
  let gestureContentHeight = 0;
  let gestureContentContracted = false;
  const observeGestureContent = (height: number) => {
    if (gesture === "idle") return;
    if (height < gestureContentHeight - 1) gestureContentContracted = true;
    gestureContentHeight = Math.max(gestureContentHeight, height);
  };
  const nearBottom = (sample: TranscriptScrollSample | undefined) =>
    !!sample &&
    transcriptEndOffset(sample.contentHeight, sample.viewportHeight) -
      sample.offset <=
      bottomThreshold;
  const finishGesture = (sample: TranscriptScrollSample | undefined) => {
    if (sample) observeGestureContent(sample.contentHeight);
    // A disappearing footer can clamp a history reader's native offset to
    // zero. That temporary fitted transcript is not a return-to-bottom gesture.
    following =
      !gestureContentContracted && nearBottom(sample ?? lastGestureSample);
    gesture = "idle";
    lastGestureSample = undefined;
  };

  return {
    isFollowing: () => following,
    isAwayFromEnd: () =>
      !following &&
      viewportHeight > 0 &&
      transcriptEndOffset(contentHeight, viewportHeight) - offset >
        bottomThreshold,
    reset() {
      following = true;
      contentHeight = 0;
      offset = 0;
      viewportHeight = 0;
      gesture = "idle";
      userMomentumAllowed = false;
      lastGestureSample = undefined;
    },
    follow() {
      following = true;
      gesture = "idle";
      userMomentumAllowed = false;
      lastGestureSample = undefined;
    },
    pause() {
      following = false;
      gesture = "idle";
      userMomentumAllowed = false;
      lastGestureSample = undefined;
    },
    layout(height: number) {
      viewportHeight = Math.max(0, height);
    },
    contentSize(height: number) {
      observeGestureContent(height);
      contentHeight = Math.max(0, height);
    },
    targetOffset(): number | null {
      return following && gesture === "idle" && viewportHeight > 0
        ? transcriptEndOffset(contentHeight, viewportHeight)
        : null;
    },
    scroll(sample: TranscriptScrollSample) {
      offset = sample.offset;
      // Passive events must neither pause following nor pull history readers
      // back to the end when content shrinks underneath their current offset.
      if (gesture !== "idle") {
        observeGestureContent(sample.contentHeight);
        lastGestureSample = sample;
      }
    },
    beginDrag() {
      gesture = "drag";
      following = false;
      userMomentumAllowed = true;
      lastGestureSample = undefined;
      gestureContentHeight = contentHeight;
      gestureContentContracted = false;
    },
    endDrag(sample?: TranscriptScrollSample) {
      if (gesture !== "drag") return;
      if (sample) observeGestureContent(sample.contentHeight);
      const release = sample ?? lastGestureSample;
      const momentumPending =
        !!release &&
        (release.targetOffset !== undefined
          ? Math.abs(release.targetOffset - release.offset) > 1
          : (release.velocityY ?? 0) !== 0);
      if (momentumPending) {
        // Native end-drag precedes momentum-begin. Keep corrections paused
        // across that gap so a content update cannot cancel the user's fling.
        gesture = "momentum";
        lastGestureSample = release;
      } else {
        finishGesture(release);
      }
    },
    beginMomentum() {
      if (!userMomentumAllowed) return;
      gesture = "momentum";
      following = false;
    },
    endMomentum(sample?: TranscriptScrollSample) {
      if (gesture !== "momentum") return;
      finishGesture(sample);
      userMomentumAllowed = false;
    },
  };
}
