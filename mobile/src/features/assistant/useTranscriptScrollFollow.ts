import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject,
} from "react";
import {
  createTranscriptScrollFollower,
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
    loading = false,
    historyError = false,
  }: {
    reducedMotion?: boolean;
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
  const mounted = useRef(true);
  const available = () =>
    mounted.current && currentScope.current.generation === generation;
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
    if (!available() || initial.current.done) return;
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

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      cancelReveal();
      clearTimeout(revealDeadline.current);
    };
  }, []);
  useLayoutEffect(() => {
    cancelReveal();
    initial.current = { viewport: false, content: false, done: false };
    setRevealed(undefined);
    follower.reset();
    setFollowing(true);
    setShowJump(false);
    // Geometry and loading may never settle; keep errors and partial history reachable.
    revealDeadline.current = setTimeout(reveal, 800);
    return () => {
      cancelReveal();
      clearTimeout(revealDeadline.current);
    };
  }, [follower, scopeKey]);

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
      follower.pause();
      sync();
    },
    follow: () => {
      if (!available()) return;
      follower.follow();
      sync();
      scrollToFollowedEnd("jump");
    },
    listProps: {
      onLayout: (event: { nativeEvent: { layout: { height: number } } }) => {
        if (!available()) return;
        initial.current.viewport = event.nativeEvent.layout.height > 0;
        follower.layout(event.nativeEvent.layout.height);
        scrollToFollowedEnd("layout");
        settleOpening();
        sync();
      },
      onContentSizeChange: (_width: number, height: number) => {
        if (!available()) return;
        initial.current.content = true;
        follower.contentSize(height);
        scrollToFollowedEnd("grow");
        settleOpening();
        sync();
      },
      onScroll: (event: ScrollEvent) => {
        if (available()) {
          follower.scroll(sample(event));
          settleOpening();
          sync();
        }
      },
      onScrollBeginDrag: () => {
        if (!available()) return;
        follower.beginDrag();
        sync();
      },
      onScrollEndDrag: (event: ScrollEvent) => {
        if (!available()) return;
        follower.endDrag(sample(event));
        sync();
      },
      onMomentumScrollBegin: () => {
        if (!available()) return;
        follower.beginMomentum();
        sync();
      },
      onMomentumScrollEnd: (event: ScrollEvent) => {
        if (!available()) return;
        follower.endMomentum(sample(event));
        sync();
      },
      scrollEventThrottle: 32,
    },
  };
}
