import type { KeyboardEvent } from "react-native";
import { visibleKeyboardFrame, type KeyboardFrame } from "./keyboardOverlap";

type FrameEvent = KeyboardEvent;
type Subscription = { remove(): void };
type Sources = {
  platform: string;
  keyboard: {
    metrics(): KeyboardFrame | undefined;
    isVisible(): boolean;
    addListener(
      name:
        | "keyboardWillChangeFrame"
        | "keyboardDidShow"
        | "keyboardDidChangeFrame"
        | "keyboardDidHide",
      listener: (event: FrameEvent) => void,
    ): Subscription;
  };
  appState: {
    currentState: string | null;
    addEventListener(
      name: "change",
      listener: (state: string) => void,
    ): Subscription;
  };
  dimensions: {
    addEventListener(name: "change", listener: () => void): Subscription;
  };
  windowHeight: () => number;
  androidContentBottom?: () => number | null;
  afterLayout: (callback: () => void) => number;
  cancelLayout: (id: number) => void;
  onFrame: (frame: KeyboardFrame | undefined, event?: FrameEvent) => void;
  /** The accepted frame is unchanged, but native hosts may have moved. */
  onSettled?: () => void;
};

/** Both composer limits and page insets follow one native event policy. Match
 * did frames to this show's will history; unseen did-only final frames are valid. */
export function observeKeyboardFrames({
  platform,
  keyboard,
  appState,
  dimensions,
  windowHeight,
  androidContentBottom,
  afterLayout,
  cancelLayout,
  onFrame,
  onSettled,
}: Sources) {
  let previousState = appState.currentState;
  let tracking = previousState !== "background";
  let backgrounded = !tracking;
  let authoritative = false;
  const willFrames: KeyboardFrame[] = [];
  let settled: number | undefined;
  let latest: KeyboardFrame | undefined;
  const publish = (frame: KeyboardFrame | undefined, event?: FrameEvent) => {
    latest = frame;
    onFrame(frame, event);
  };
  const visibleFrame = (frame: KeyboardFrame | undefined) =>
    visibleKeyboardFrame(frame, windowHeight(), frame && androidContentBottom?.());
  const sync = (keepAcceptedFrame = false) => {
    const accepted = keepAcceptedFrame && platform === "ios" && authoritative;
    const frame = tracking
      ? accepted
        ? latest
        : keyboard.isVisible()
          ? visibleFrame(keyboard.metrics())
          : undefined
      : undefined;
    authoritative = platform === "ios" && (accepted || !!frame);
    publish(frame);
  };
  const remeasure = () => {
    // RN's metrics cache also accepts did-show, even when that event is stale.
    // Re-measure the accepted iOS frame instead of reading that stale cache back.
    if (platform === "ios" && authoritative) {
      publish(
        latest?.height === 0
          ? latest
          : visibleKeyboardFrame(latest, windowHeight()),
      );
    } else sync();
  };
  const change = (event: FrameEvent, will: boolean) => {
    if (!tracking) return;
    if (!will) onSettled?.();
    const raw = event.endCoordinates;
    const height = windowHeight();
    if (
      !Number.isFinite(raw.height) ||
      !Number.isFinite(raw.screenY) ||
      raw.height > height ||
      raw.height < 0 ||
      raw.screenY < 0
    )
      return;
    const frame =
      raw.height === 0 && will ? raw : visibleFrame(raw);
    if (platform === "ios") {
      if (will && frame) willFrames.push(frame);
      else if (
        !will &&
        frame &&
        willFrames.some(
          (seen) =>
            seen.screenY === frame.screenY && seen.height === frame.height,
        )
      ) {
        return;
      }
      authoritative = true;
    }
    publish(frame, will ? event : undefined);
  };
  const will =
    platform === "ios"
      ? keyboard.addListener("keyboardWillChangeFrame", (event) =>
          change(event, true),
        )
      : undefined;
  const show = keyboard.addListener("keyboardDidShow", (event) =>
    change(event, false),
  );
  const changed = keyboard.addListener("keyboardDidChangeFrame", (event) =>
    change(event, false),
  );
  const hide = keyboard.addListener("keyboardDidHide", () => {
    if (!tracking) return;
    authoritative = platform === "ios";
    willFrames.length = 0;
    publish(undefined);
  });
  const app = appState.addEventListener("change", (state) => {
    const previous = previousState;
    previousState = state;
    tracking = state !== "background";
    if (!tracking) {
      if (settled !== undefined) {
        cancelLayout(settled);
        settled = undefined;
      }
      backgrounded = true;
      authoritative = false;
      willFrames.length = 0;
      publish(undefined);
      return;
    }
    // System overlays leave iOS inactive with its keyboard still visible.
    // Re-hosted frames during background -> inactive belong to the foreground
    // and must survive a stale RN metrics cache on the following active event.
    if (state !== "active" || previous === "active") return;
    if (backgrounded) sync(true);
    else remeasure();
    backgrounded = false;
    if (settled !== undefined) cancelLayout(settled);
    settled = afterLayout(() => {
      settled = undefined;
      if (tracking) remeasure();
    });
  });
  const resize = dimensions.addEventListener("change", () => {
    if (tracking) remeasure();
  });
  sync();
  return () => {
    tracking = false;
    if (settled !== undefined) cancelLayout(settled);
    will?.remove();
    show.remove();
    changed.remove();
    hide.remove();
    app.remove();
    resize.remove();
  };
}
