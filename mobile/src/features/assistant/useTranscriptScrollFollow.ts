import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject,
} from "react";
import {
  createTranscriptScrollFollower,
  transcriptEndOffset,
  type TranscriptScrollSample,
} from "./assistantTranscriptScroll";

type ScrollableTranscript = {
  scrollToOffset: (options: { offset: number; animated: boolean }) => void;
};
type ScrollEvent = {
  nativeEvent: {
    contentOffset: { y: number };
    contentSize: { height: number };
    layoutMeasurement: { height: number };
    velocity?: { y: number };
    targetContentOffset?: { y: number };
  };
};
const sample = ({ nativeEvent }: ScrollEvent): TranscriptScrollSample => ({
  contentHeight: nativeEvent.contentSize.height,
  viewportHeight: nativeEvent.layoutMeasurement.height,
  offset: nativeEvent.contentOffset.y,
  velocityY: nativeEvent.velocity?.y,
  targetOffset: nativeEvent.targetContentOffset?.y,
});

/** A reply that grows by more than this many screens at once is followed instantly. */
const SMOOTH_FOLLOW_SCREENS = 1.25;

/**
 * Native geometry can change while the reader's follow/reading intent stays put.
 * Corrections glide: jumping to latest animates, and a reply growing by up to
 * about a screen is followed smoothly; larger jumps (opening a long chat) and
 * layout changes stay instant, as does everything under Reduce Motion.
 */
export function useTranscriptScrollFollow(
  list: RefObject<ScrollableTranscript | null>,
  scopeKey: string | undefined,
  {
    reducedMotion = false,
    focused = true,
    loading = false,
    historyError = false,
  }: {
    reducedMotion?: boolean;
    focused?: boolean;
    loading?: boolean;
    historyError?: boolean;
  } = {},
) {
  const follower = useRef(createTranscriptScrollFollower()).current;
  const [following, setFollowing] = useState(true);
  const [showJump, setShowJump] = useState(false);
  const [revealed, setRevealed] = useState<number>();
  const initial = useRef({ viewport: false, content: false, done: false });
  const revealFrame = useRef<
    ReturnType<typeof requestAnimationFrame> | undefined
  >(undefined);
  const revealDeadline = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const currentLoading = useRef(loading);
  currentLoading.current = loading;
  const currentScope = useRef({ key: scopeKey, generation: 0 });
  if (currentScope.current.key !== scopeKey) {
    currentScope.current = {
      key: scopeKey,
      generation: currentScope.current.generation + 1,
    };
  }
  const generation = currentScope.current.generation;
  const focusCycle = useRef({ focused, generation: 0 });
  if (focusCycle.current.focused !== focused) {
    focusCycle.current = {
      focused,
      generation: focusCycle.current.generation + 1,
    };
  }
  const focusGeneration = focusCycle.current.generation;
  const mounted = useRef(true);
  const currentCycle = () =>
    mounted.current &&
    currentScope.current.generation === generation &&
    focusCycle.current.generation === focusGeneration;
  const available = () => currentCycle() && focusCycle.current.focused;
  // Revealing only flips state, so it may land while hidden; it must not be
  // lost to a focus change, or the opened chat would stay concealed.
  const inScope = () =>
    mounted.current && currentScope.current.generation === generation;
  const geometry = useRef({ contentHeight: 0, viewportHeight: 0, offset: 0 });
  const savedPosition = useRef<
    { generation: number; offset: number; following: boolean } | undefined
  >(undefined);
  const restoreFrame = useRef<number | undefined>(undefined);
  const sync = () => {
    setFollowing(follower.isFollowing());
    setShowJump(follower.isAwayFromEnd());
  };
  const scrollToFollowedEnd = (kind: "jump" | "grow" | "layout") => {
    if (!available()) return;
    const offset = follower.targetOffset();
    if (offset === null) return;
    const distance = follower.distanceToEnd();
    const animated =
      !reducedMotion &&
      (kind === "jump" ||
        (kind === "grow" &&
          distance > 2 &&
          distance <= follower.viewport() * SMOOTH_FOLLOW_SCREENS));
    list.current?.scrollToOffset({
      offset,
      animated: initial.current.done && animated,
    });
  };
  const cancelReveal = () => {
    if (revealFrame.current !== undefined)
      cancelAnimationFrame(revealFrame.current);
    revealFrame.current = undefined;
  };
  const reveal = () => {
    if (!inScope() || initial.current.done) return;
    cancelReveal();
    clearTimeout(revealDeadline.current);
    revealDeadline.current = undefined;
    initial.current.done = true;
    setRevealed(generation);
  };
  const settleOpening = () => {
    if (
      !available() ||
      initial.current.done ||
      currentLoading.current ||
      !initial.current.viewport ||
      !initial.current.content
    )
      return;
    cancelReveal();
    // A native scroll acknowledgement proves the virtualized tail is in place.
    if (follower.distanceToEnd() > 2) return;
    revealFrame.current = requestAnimationFrame(() => {
      if (!available()) return;
      scrollToFollowedEnd("layout");
      revealFrame.current = requestAnimationFrame(() => {
        revealFrame.current = undefined;
        if (!available() || currentLoading.current) return;
        reveal();
      });
    });
  };
  const cancelRestore = () => {
    if (restoreFrame.current !== undefined)
      cancelAnimationFrame(restoreFrame.current);
    restoreFrame.current = undefined;
  };
  const scheduleRestore = () => {
    if (
      !available() ||
      !savedPosition.current ||
      restoreFrame.current !== undefined
    )
      return;
    restoreFrame.current = requestAnimationFrame(() => {
      restoreFrame.current = undefined;
      const saved = savedPosition.current;
      if (!available() || !saved || saved.generation !== generation) return;
      const measured = geometry.current;
      if (measured.viewportHeight <= 0) return;
      follower.layout(measured.viewportHeight);
      follower.contentSize(measured.contentHeight);
      const end = transcriptEndOffset(
        measured.contentHeight,
        measured.viewportHeight,
      );
      const offset = saved.following
        ? end
        : Math.max(0, Math.min(saved.offset, end));
      savedPosition.current = undefined;
      if (saved.following) follower.follow();
      else follower.pause();
      measured.offset = offset;
      follower.scroll({ ...measured, offset });
      list.current?.scrollToOffset({ offset, animated: false });
      sync();
    });
  };

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      cancelRestore();
      cancelReveal();
      clearTimeout(revealDeadline.current);
    };
  }, []);
  useLayoutEffect(() => {
    cancelReveal();
    initial.current = { viewport: false, content: false, done: false };
    setRevealed(undefined);
    follower.reset();
    geometry.current = { contentHeight: 0, viewportHeight: 0, offset: 0 };
    savedPosition.current = undefined;
    cancelRestore();
    setFollowing(true);
    setShowJump(false);
    // Geometry and loading may never settle; keep errors and partial history reachable.
    revealDeadline.current = setTimeout(reveal, 800);
    return () => {
      cancelReveal();
      clearTimeout(revealDeadline.current);
    };
  }, [follower, scopeKey]);
  useLayoutEffect(() => {
    if (!focused) {
      // Native navigation can animate a retained offscreen UIScrollView to
      // zero. Keep the last visible position and the reader's follow intent.
      savedPosition.current =
        geometry.current.viewportHeight > 0
          ? {
              generation,
              offset: geometry.current.offset,
              following: follower.isFollowing(),
            }
          : undefined;
    } else {
      if (savedPosition.current) scheduleRestore();
      else {
        // A screen first laid out while hidden has no visible position to
        // restore. Its current dimensions can still open it at the latest row.
        follower.layout(geometry.current.viewportHeight);
        follower.contentSize(geometry.current.contentHeight);
        scrollToFollowedEnd("layout");
      }
      // Geometry measured while hidden still counts towards the opening reveal.
      if (geometry.current.viewportHeight > 0) initial.current.viewport = true;
      if (geometry.current.contentHeight > 0) initial.current.content = true;
      settleOpening();
    }
    return cancelRestore;
  }, [focused, generation]);

  useEffect(() => {
    if (historyError) reveal();
    else if (!loading) settleOpening();
  }, [loading, historyError, scopeKey]);

  return {
    ready: revealed === generation,
    following,
    showJump,
    pause: () => {
      if (!available()) return;
      savedPosition.current = undefined;
      cancelRestore();
      follower.pause();
      sync();
    },
    follow: () => {
      if (!available()) return;
      savedPosition.current = undefined;
      cancelRestore();
      follower.follow();
      sync();
      scrollToFollowedEnd("jump");
    },
    listProps: {
      onLayout: (event: { nativeEvent: { layout: { height: number } } }) => {
        if (!currentCycle()) return;
        const height = event.nativeEvent.layout.height;
        if (!focusCycle.current.focused) {
          // Keyboard dismissal can resize a retained hidden screen without a
          // second layout on return. Ignore native collapsed zero dimensions.
          if (height > 0) geometry.current.viewportHeight = height;
          return;
        }
        geometry.current.viewportHeight = height;
        initial.current.viewport = height > 0;
        follower.layout(height);
        if (savedPosition.current) scheduleRestore();
        else scrollToFollowedEnd("layout");
        settleOpening();
        sync();
      },
      onContentSizeChange: (_width: number, height: number) => {
        if (!currentCycle()) return;
        if (!focusCycle.current.focused) {
          if (height > 0) geometry.current.contentHeight = height;
          return;
        }
        geometry.current.contentHeight = height;
        initial.current.content = true;
        follower.contentSize(height);
        if (savedPosition.current) scheduleRestore();
        else scrollToFollowedEnd("grow");
        settleOpening();
        sync();
      },
      onScroll: (event: ScrollEvent) => {
        if (available()) {
          // A native event racing the return frame must not replace the saved
          // visible offset before it has been restored.
          if (savedPosition.current) return;
          geometry.current.offset = event.nativeEvent.contentOffset.y;
          follower.scroll(sample(event));
          settleOpening();
          sync();
        }
      },
      onScrollBeginDrag: () => {
        if (!available()) return;
        savedPosition.current = undefined;
        cancelRestore();
        follower.beginDrag();
        sync();
      },
      onScrollEndDrag: (event: ScrollEvent) => {
        if (!available() || savedPosition.current) return;
        geometry.current.offset = event.nativeEvent.contentOffset.y;
        follower.endDrag(sample(event));
        sync();
      },
      onMomentumScrollBegin: () => {
        if (!available() || savedPosition.current) return;
        follower.beginMomentum();
        sync();
      },
      onMomentumScrollEnd: (event: ScrollEvent) => {
        if (!available() || savedPosition.current) return;
        geometry.current.offset = event.nativeEvent.contentOffset.y;
        follower.endMomentum(sample(event));
        sync();
      },
      scrollEventThrottle: 32,
    },
  };
}
