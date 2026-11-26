import { requireOptionalNativeModule } from "expo";
import { Platform } from "react-native";

/** Apple Watch pairing state reported by `modules/nyx-watch` (WatchConnectivity). */
export type WatchLinkStatus = {
  supported: boolean;
  activated: boolean;
  paired: boolean;
  installed: boolean;
  reachable: boolean;
  /** The six-digit setup code the watch shows while signed out ("" otherwise). */
  watchCode?: string;
};

export type WatchDelivery = "delivered" | "queued";

export type WatchLink = {
  getStatus(): Promise<WatchLinkStatus>;
  /** Sends a small string-only message to the NyxID watch app. */
  send(message: Record<string, string>): Promise<WatchDelivery>;
  /** Replaces a queued (undelivered) message with a blank one. */
  clearPending(): Promise<void>;
  addListener(
    event: "onStatusChange",
    listener: (status: WatchLinkStatus) => void,
  ): { remove(): void };
};

/**
 * The native link, or `null` on Android and on iOS builds that predate the
 * watch app (the settings row stays hidden then).
 */
export function nativeWatchLink(): WatchLink | null {
  if (Platform.OS !== "ios") return null;
  return requireOptionalNativeModule<WatchLink>("NyxWatch");
}
