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
  { reducedMotion = false }: { reducedMotion?: boolean } = {},
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
    list.current?.scrollToOffset({ offset, animated });
  };

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useLayoutEffect(() => {
    follower.reset();
    setFollowing(true);
    setShowJump(false);
  }, [follower, scopeKey]);

  return {
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
        follower.layout(event.nativeEvent.layout.height);
        scrollToFollowedEnd("layout");
        sync();
      },
      onContentSizeChange: (_width: number, height: number) => {
        if (!available()) return;
        follower.contentSize(height);
        scrollToFollowedEnd("grow");
        sync();
      },
      onScroll: (event: ScrollEvent) => {
        if (available()) {
          follower.scroll(sample(event));
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
