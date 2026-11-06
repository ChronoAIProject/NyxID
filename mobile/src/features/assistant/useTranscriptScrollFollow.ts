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
  }: { reducedMotion?: boolean; focused?: boolean } = {},
) {
  const follower = useRef(createTranscriptScrollFollower()).current;
  const [following, setFollowing] = useState(true);
  const [showJump, setShowJump] = useState(false);
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
    list.current?.scrollToOffset({ offset, animated });
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
    };
  }, []);
  useLayoutEffect(() => {
    follower.reset();
    geometry.current = { contentHeight: 0, viewportHeight: 0, offset: 0 };
    savedPosition.current = undefined;
    cancelRestore();
    setFollowing(true);
    setShowJump(false);
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
    }
    return cancelRestore;
  }, [focused, generation]);

  return {
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
        follower.layout(height);
        if (savedPosition.current) scheduleRestore();
        else scrollToFollowedEnd("layout");
        sync();
      },
      onContentSizeChange: (_width: number, height: number) => {
        if (!currentCycle()) return;
        if (!focusCycle.current.focused) {
          if (height > 0) geometry.current.contentHeight = height;
          return;
        }
        geometry.current.contentHeight = height;
        follower.contentSize(height);
        if (savedPosition.current) scheduleRestore();
        else scrollToFollowedEnd("grow");
        sync();
      },
      onScroll: (event: ScrollEvent) => {
        if (available()) {
          // A native event racing the return frame must not replace the saved
          // visible offset before it has been restored.
          if (savedPosition.current) return;
          geometry.current.offset = event.nativeEvent.contentOffset.y;
          follower.scroll(sample(event));
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
