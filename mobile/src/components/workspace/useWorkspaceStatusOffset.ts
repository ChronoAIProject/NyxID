import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  AppState,
  NativeEventEmitter,
  NativeModules,
  Platform,
} from "react-native";
import {
  subscribeSystemStatusActivity,
  systemStatusActivityActive,
} from "../../lib/systemStatusActivity";
import { QUICK_RAIL_MAX_STATUS_OFFSET } from "./workspaceLayout";

type StatusFrame = { frame: { y: number; height: number } };

/** Follow impending system movement, rather than waiting for the safe area. */
export function useWorkspaceStatusOffset(
  enabled: boolean,
  width: number,
  height: number,
  topInset: number,
) {
  const key = `${width}x${height}`;
  const [capture, setCapture] = useState(systemStatusActivityActive);
  const [status, setStatus] = useState<{ key: string; bottom: number }>();
  const startupKey = useRef(key);
  const baseline = useRef({
    key,
    inset: topInset > 0 ? topInset : undefined,
    bottom: undefined as number | undefined,
  });
  if (baseline.current.key !== key)
    baseline.current = {
      key,
      inset: topInset > 0 ? topInset : undefined,
      bottom: undefined,
    };
  // Ignore the provider's unmeasured/hidden zero inset. Its first real value
  // establishes the idle baseline instead of appearing to be an expansion.
  if (topInset > 0)
    baseline.current.inset = Math.min(baseline.current.inset ?? topInset, topInset);
  if (status?.key === key && status.bottom > 0)
    baseline.current.bottom = Math.min(
      baseline.current.bottom ?? status.bottom,
      status.bottom,
    );

  useLayoutEffect(() => {
    if (!enabled) return;
    const update = () => setCapture(systemStatusActivityActive());
    const unsubscribe = subscribeSystemStatusActivity(update);
    update();
    return unsubscribe;
  }, [enabled]);

  useEffect(() => {
    if (!enabled || Platform.OS !== "ios") return;
    const manager = NativeModules.StatusBarManager;
    if (!manager) return;
    let alive = true;
    let revision = 0;
    const initial = manager.getConstants?.().HEIGHT;
    // The module's constant describes app startup; after rotation use the
    // new posture's safe area until its first live frame read arrives.
    baseline.current.bottom ??=
      Number.isFinite(initial) && initial > 0 && startupKey.current === key
        ? initial
        : baseline.current.inset;
    const receive = (bottom: number) => {
      if (!alive || !Number.isFinite(bottom) || bottom < 0) return;
      setStatus({ key, bottom });
    };
    const emitter = new NativeEventEmitter(manager);
    const changed = ({ frame }: StatusFrame) => {
      ++revision;
      receive(frame.y + frame.height);
    };
    const read = () => {
      const requested = ++revision;
      manager.getHeight?.(({ height: current }: { height: number }) => {
        // A delayed initial/foreground read must not undo a newer will-change.
        if (requested === revision) receive(current);
      });
    };
    const will = emitter.addListener("statusBarFrameWillChange", changed);
    // UIKit's did-change payload contains the OLD frame. Read the current
    // frame on this event so finishing an expansion cannot undo the shift.
    const did = emitter.addListener("statusBarFrameDidChange", read);
    const foreground = AppState.addEventListener("change", (state) => {
      if (state === "active") read();
    });
    read();
    return () => {
      alive = false;
      will.remove();
      did.remove();
      foreground.remove();
    };
  }, [enabled, key]);

  if (!enabled) return 0;
  // Actual capture start/stop events cover privacy indicators that don't
  // resize the status frame. No route-open guesses, polling or UI timers.
  // Live frame/inset growth also covers system activity outside NyxID.
  return Math.min(
    QUICK_RAIL_MAX_STATUS_OFFSET,
    Math.max(
      capture ? QUICK_RAIL_MAX_STATUS_OFFSET : 0,
      topInset - (baseline.current.inset ?? topInset),
      status?.key === key
        ? status.bottom - (baseline.current.bottom ?? status.bottom)
        : 0,
    ),
  );
}
