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

/** Native geometry can change while the reader's follow/reading intent stays put. */
export function useTranscriptScrollFollow(
  list: RefObject<ScrollableTranscript | null>,
  scopeKey: string | undefined,
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
  const scrollToFollowedEnd = () => {
    if (!available()) return;
    const offset = follower.targetOffset();
    if (offset !== null)
      list.current?.scrollToOffset({ offset, animated: false });
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
      scrollToFollowedEnd();
    },
    listProps: {
      onLayout: (event: { nativeEvent: { layout: { height: number } } }) => {
        if (!available()) return;
        follower.layout(event.nativeEvent.layout.height);
        scrollToFollowedEnd();
        sync();
      },
      onContentSizeChange: (_width: number, height: number) => {
        if (!available()) return;
        follower.contentSize(height);
        scrollToFollowedEnd();
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
