import { useCallback, useEffect, useLayoutEffect, useRef, type Ref, type RefObject } from "react";
import { ScrollView, TextInput, type ScrollViewProps, type View } from "react-native";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { spacing } from "../../theme/designTokens";
import {
  KeyboardViewport,
  type KeyboardViewportFrame,
} from "./KeyboardViewport";

type RevealView = Pick<View, "measureInWindow">;
export type KeyboardScrollViewHandle = {reveal: (view: RevealView | null) => void};

/** A measured viewport reserves only the keyboard overlap. Focus is revealed
 * separately, including when an alert returns focus without a new IME frame. */
export function KeyboardScrollView({
  scrollRef,
  revealController,
  ...props
}: ScrollViewProps & {
  scrollRef?: Ref<ScrollView>;
  revealController?: RefObject<KeyboardScrollViewHandle | null>;
}) {
  const scroller = useRef<ScrollView | null>(null);
  const focused =
    useRef<ReturnType<typeof TextInput.State.currentlyFocusedInput>>(null);
  const requested = useRef<RevealView | null>(null);
  const viewport = useRef<KeyboardViewportFrame | undefined>(undefined);
  const offset = useRef(0);
  const revision = useRef(0);
  const scheduled = useRef<number | undefined>(undefined);
  const revealPending = useRef(false);
  const mounted = useRef(false);
  const reducedMotion = useReducedMotion();
  const cancelReveal = useCallback(() => {
    revision.current++;
    revealPending.current = false;
    if (scheduled.current !== undefined)
      cancelAnimationFrame(scheduled.current);
    scheduled.current = undefined;
  }, []);
  const reveal = useCallback(() => {
    cancelReveal();
    const explicit = requested.current;
    const input = explicit ?? focused.current;
    const scroll = scroller.current;
    const frame = viewport.current;
    if (!input || !scroll || !frame || frame.height <= 0) return;
    const current = revision.current;
    revealPending.current = true;
    // Wait for overlap padding to reach native layout before asking it to scroll.
    scheduled.current = requestAnimationFrame(() => {
      scheduled.current = undefined;
      const measuredOffset = offset.current;
      input.measureInWindow((_x, y, _width, height) => {
        if (
          !mounted.current ||
          revision.current !== current ||
          scroller.current !== scroll ||
          (explicit ? requested.current !== input : (
            focused.current !== input || TextInput.State.currentlyFocusedInput() !== input
          )) ||
          offset.current !== measuredOffset ||
          !Number.isFinite(y) ||
          !Number.isFinite(height) ||
          height <= 0
        )
          return;
        revealPending.current = false;
        const top = frame.y + spacing.lg;
        const bottom = frame.y + frame.height - spacing.lg;
        const delta =
          y < top || height > bottom - top
            ? y - top
            : Math.max(0, y + height - bottom);
        if (Math.abs(delta) > 1)
          scroll.scrollTo({
            y: Math.max(0, measuredOffset + delta),
            animated: !reducedMotion,
          });
      });
    });
  }, [cancelReveal, reducedMotion]);
  useLayoutEffect(() => {
    if (!revealController) return;
    const handle: KeyboardScrollViewHandle = {reveal: (view) => {
      requested.current = view;
      reveal();
    }};
    revealController.current = handle;
    return () => {if (revealController.current === handle) revealController.current = null;};
  }, [reveal, revealController]);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      cancelReveal();
    };
  }, [cancelReveal]);
  const attach = useCallback(
    (node: ScrollView | null) => {
      scroller.current = node;
      if (typeof scrollRef === "function") {
        const cleanup = scrollRef(node);
        if (cleanup)
          return () => {
            scroller.current = null;
            cleanup();
          };
      } else if (scrollRef) scrollRef.current = node;
    },
    [scrollRef],
  );
  return (
    <KeyboardViewport
      onViewportChange={(frame) => {
        viewport.current = frame;
        reveal();
      }}
    >
      <ScrollView
        {...props}
        ref={attach}
        automaticallyAdjustKeyboardInsets={false}
        onFocus={(event) => {
          requested.current = null;
          focused.current = TextInput.State.currentlyFocusedInput();
          reveal();
          props.onFocus?.(event);
        }}
        onBlur={(event) => {
          requested.current = null;
          focused.current = null;
          cancelReveal();
          props.onBlur?.(event);
        }}
        onLayout={(event) => {
          reveal();
          props.onLayout?.(event);
        }}
        onContentSizeChange={(width, height) => {
          reveal();
          props.onContentSizeChange?.(width, height);
        }}
        onScroll={(event) => {
          const pending = revealPending.current;
          offset.current = event.nativeEvent.contentOffset.y;
          // Native resize can report an offset before the focus measurement
          // completes. Re-measure from that offset instead of losing the reveal.
          if (pending) reveal();
          else cancelReveal();
          props.onScroll?.(event);
        }}
        onScrollBeginDrag={(event) => {
          requested.current = null;
          cancelReveal();
          props.onScrollBeginDrag?.(event);
        }}
      />
    </KeyboardViewport>
  );
}
