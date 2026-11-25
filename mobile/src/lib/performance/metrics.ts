import Constants from "expo-constants";
import { nativeBuildVersion, nativeApplicationVersion } from "expo-application";
import { Platform } from "react-native";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { createPerformanceConsent } from "./consent";
import type { PerformancePostHog } from "./posthogTransport";
import type { FrameScenario, FrameTiming } from "./nativePerformance";
import type { ChatTurnTimings } from "../../features/assistant/assistantTurnPerformance";

import type { JsTaskTiming } from "./jsTaskWindow";

type PerformanceEvents = {
  "mobile.perf_js_tasks": JsTaskTiming;
  "mobile.perf_chat_turn": ChatTurnTimings;
  "mobile.perf_frames": FrameTiming & { scenario: FrameScenario };
};

const extra = Constants.expoConfig?.extra;
const token = extra?.PERFORMANCE_POSTHOG_TOKEN;
const host = extra?.PERFORMANCE_POSTHOG_HOST;
function readDestination() {
  if (
    (Platform.OS !== "android" && Platform.OS !== "ios") ||
    typeof token !== "string" ||
    !/^phc_[A-Za-z0-9_-]+$/.test(token) ||
    typeof host !== "string"
  )
    return null;
  try {
    const url = new URL(host);
    if (
      url.protocol === "https:" &&
      !url.username &&
      !url.password &&
      url.pathname === "/" &&
      !url.search &&
      !url.hash
    ) {
      return { token, host: url.origin };
    }
  } catch {
    /* An invalid configuration disables collection. */
  }
  return null;
}
const destination = readDestination();
const preferenceKey = "nyxid.performance-sharing.v1";
const consent = createPerformanceConsent(
  destination && JSON.stringify([destination.host, destination.token]),
  () => AsyncStorage.getItem(preferenceKey),
  (value) =>
    value === null
      ? AsyncStorage.removeItem(preferenceKey)
      : AsyncStorage.setItem(preferenceKey, value),
  () => extra?.APP_ENV === "dev" || Math.random() < 0.1,
);
export const isPerformanceConfigured = () => destination !== null;
export const getPerformanceConsentEpoch = consent.current;
export const hydratePerformanceConsent = consent.hydrate;
export const setPerformanceConsent = consent.set;
export const subscribePerformanceConsent = consent.subscribe;
export const beginPerformanceSample = consent.begin;
let client: PerformancePostHog | undefined;
let pending = 0;
export function submitPerformanceSample<K extends keyof PerformanceEvents>(
  epoch: number | null,
  event: K,
  properties: PerformanceEvents[K],
): void {
  if (
    !destination ||
    epoch === null ||
    epoch !== consent.current() ||
    pending >= 8
  )
    return;
  const target = destination;
  pending++;
  void import("./posthogTransport")
    .then(async ({ PerformancePostHog }) => {
      if (epoch !== consent.current()) return;
      client ??= new PerformancePostHog(target);
      await client.ready();
      if (epoch !== consent.current()) return;
      const platform = Platform.OS;
      await client.send(event, {
        ...properties,
        platform,
        client_version: nativeApplicationVersion ?? "unknown",
        build_number: nativeBuildVersion ?? "unknown",
        environment: extra?.APP_ENV === "dev" ? "dev" : "prod",
        surface: "mobile",
      });
    })
    .catch(() => {})
    .finally(() => {
      pending--;
    });
}
