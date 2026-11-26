import { nativePerformance } from "./nativePerformance";
import type { LaunchTiming } from "./launchRuntime";
import Constants from "expo-constants";
import { nativeBuildVersion, nativeApplicationVersion } from "expo-application";
import { Platform } from "react-native";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { URL as WhatwgURL } from "whatwg-url-minimum";
import { createPerformanceConsent } from "./consent";
import type { PerformancePostHog } from "./posthogTransport";
import type { FrameScenario, FrameTiming } from "./nativePerformance";
import type { ChatTurnTimings } from "../../features/assistant/assistantTurnPerformance";

import type { JsTaskTiming } from "./jsTaskWindow";

type PerformanceEvents = {
  "mobile.perf_launch": LaunchTiming;
  "mobile.perf_js_tasks": JsTaskTiming;
  "mobile.perf_chat_turn": ChatTurnTimings;
  "mobile.perf_frames": FrameTiming & { scenario: FrameScenario };
};

const extra = Constants.expoConfig?.extra;
const token = extra?.PERFORMANCE_POSTHOG_TOKEN;
const host = extra?.PERFORMANCE_POSTHOG_HOST;

function canonicalPerformanceOrigin(value: unknown): string | null {
  if (typeof value !== "string") return null;
  // Require the raw HTTPS origin form before URL parsing can discard or rewrite input.
  const match =
    /^https:\/\/([a-zA-Z0-9._-]+|\[[a-fA-F0-9:.]+\])(?::[0-9]+)?\/?$/.exec(value);
  const rawHost = match?.[1];
  if (!rawHost || match?.[0] !== value) return null;
  try {
    const url = new WhatwgURL(value);
    // For unbracketed hosts, reject transformations such as packed IPv4 aliases.
    if (!rawHost.startsWith("[") && url.hostname !== rawHost.toLowerCase())
      return null;
    return url.origin;
  } catch {
    return null;
  }
}

function readDestination() {
  if (
    (Platform.OS !== "android" && Platform.OS !== "ios") ||
    typeof token !== "string" ||
    !/^phc_[A-Za-z0-9_-]+$/.test(token)
  )
    return null;
  const origin = canonicalPerformanceOrigin(host);
  return origin ? { token, host: origin } : null;
}
const destination = readDestination();
const preferenceKey = "nyxid.performance-sharing.v1";
const destinationGrant =
  destination && JSON.stringify([destination.host, destination.token]);
async function readSavedGrant() {
  const saved = await AsyncStorage.getItem(preferenceKey);
  if (!destination || saved === null) return null;
  try {
    const record: unknown = JSON.parse(saved);
    if (
      Array.isArray(record) &&
      record.length === 2 &&
      record[1] === destination.token &&
      canonicalPerformanceOrigin(record[0]) === destination.host
    )
      return destinationGrant;
  } catch {
    // Invalid saved records carry no grant.
  }
  return null;
}
const launchBinding = extra?.PERFORMANCE_LAUNCH_BINDING;
let mirrorWrites = Promise.resolve();
let preferenceChanged = false;
function mirrorLaunchGrant(enabled: boolean) {
  const binding =
    destination &&
    typeof launchBinding === "string" &&
    /^[0-9a-f]{64}$/.test(launchBinding)
      ? launchBinding
      : null;
  const write = mirrorWrites.then(() =>
    nativePerformance?.setLaunchConsent?.(enabled ? binding : null),
  );
  mirrorWrites = write.catch(() => {});
  return write;
}
const consent = createPerformanceConsent(
  destinationGrant,
  readSavedGrant,
  async (value) => {
    if (value === null) {
      // Attempt both removals even if one durable store fails. The UI can retry.
      await Promise.all([
        AsyncStorage.removeItem(preferenceKey),
        mirrorLaunchGrant(false),
      ]);
    } else {
      await AsyncStorage.setItem(preferenceKey, value);
      try {
        await mirrorLaunchGrant(true);
      } catch (error) {
        await AsyncStorage.removeItem(preferenceKey);
        throw error;
      }
    }
  },
  () => extra?.APP_ENV === "dev" || Math.random() < 0.1,
);
export const isPerformanceConfigured = () => destination !== null;
export const getPerformanceConsentEpoch = consent.current;
export async function hydratePerformanceConsent() {
  await consent.hydrate();
  // The saved JS choice stays available for withdrawal while the native mirror
  // reconciles. Native setters do not arm a launch already in progress.
  if (!preferenceChanged)
    void mirrorLaunchGrant(consent.current() !== null).catch(() => {});
}
export const getPerformanceColdGrant = consent.coldGrant;
export function setPerformanceConsent(enabled: boolean) {
  preferenceChanged = true;
  return consent.set(enabled);
}
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
