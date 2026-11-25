import { setImmediate as nextTask } from "node:timers";
import { AppState, Platform } from "react-native";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { createPerformanceConsent } from "./consent";
import { PerformancePostHog } from "./posthogTransport";

// The Expo Jest preset preserves import(), which ordinarily requires Node's
// experimental VM modules flag. The shared harness compiles the production
// entry point to CommonJS and injects this exact SDK class at its lazy import.
function loadMetrics(
  nativeLaunch: {
    setLaunchConsent(binding: string | null): Promise<void>;
  } | null = null,
) {
  const harness = hookHarness({
    "expo-constants": {
      expoConfig: {
        extra: {
          APP_ENV: "dev",
          PERFORMANCE_LAUNCH_BINDING: "a".repeat(64),
          PERFORMANCE_POSTHOG_TOKEN: destination.token,
          PERFORMANCE_POSTHOG_HOST: destination.host,
        },
      },
    },
    "expo-application": {
      nativeApplicationVersion: "1.2.3",
      nativeBuildVersion: "42",
    },
    "react-native": { Platform },
    "@react-native-async-storage/async-storage": AsyncStorage,
    "./consent": { createPerformanceConsent },
    "./nativePerformance": { nativePerformance: nativeLaunch },
    "./posthogTransport": { PerformancePostHog },
  });
  return harness.load(
    require.resolve("./metrics"),
  ) as typeof import("./metrics");
}

// Application code and the pinned SDK are real. Only the network boundary and
// native device state are replaced. No request can leave the test process.
const destination = {
  host: "https://ingest.invalid",
  token: "phc_testfixture",
};
const settle = () => new Promise<void>((resolve) => nextTask(resolve));
const originalFetch = global.fetch;
const fetchMock = jest.fn<ReturnType<typeof fetch>, Parameters<typeof fetch>>();
const lifecycle = new Set<(state: "active" | "background") => void>();
function resume() {
  for (const listener of lifecycle) listener("background");
  for (const listener of lifecycle) listener("active");
}
let metrics: ReturnType<typeof loadMetrics>;

beforeEach(() => {
  jest.useFakeTimers();
  fetchMock.mockReset().mockResolvedValue({ status: 200 } as Response);
  global.fetch = fetchMock;
  lifecycle.clear();
  metrics = loadMetrics();
  jest
    .spyOn(AppState, "addEventListener")
    .mockImplementation((_event, listener) => {
      lifecycle.add(listener);
      return {
        remove() {
          lifecycle.delete(listener);
        },
      };
    });
});

afterEach(async () => {
  await metrics.setPerformanceConsent(false);
  global.fetch = originalFetch;
  jest.restoreAllMocks();
  jest.useRealTimers();
});

test("consented measurements produce only direct single-event batches with the expected properties", async () => {
  expect(metrics.beginPerformanceSample()).toBeNull();
  await metrics.setPerformanceConsent(true);
  const epoch = metrics.beginPerformanceSample();
  expect(epoch).not.toBeNull();
  const jsTasks = {
    observation_source: "rn_longtask",
    duration_ms: 10000,
    threshold_ms: 50,
    task_count: 2,
    blocked_ms: 100,
    max_task_ms: 125,
  } as const;
  const frames = {
    measurement_source: "ios_display_link",
    scenario: "chat_drag",
    duration_ms: 1000,
    sample_count: 58,
    interval_p50_ms: 16.7,
    interval_p95_ms: 33.4,
    interval_max_ms: 50.1,
    over_budget_count: 2,
    observed_fps: 58,
    peak_rss_bytes: 210_000_000,
    peak_rss_source: "ios_getrusage_maxrss",
  } as const;
  const chat = {
    outcome: "completed",
    duration_ms: 750,
    headers_ms: 102,
    first_text_ms: 230,
    first_text_commit_ms: 248,
  } as const;
  const launch = {
    startup_kind: "cold",
    milestone: "first_frame",
    target: "application",
    duration_ms: 391,
    origin_source: "ios_did_finish_launching",
    measurement_source: "ios_window_commit_then_display_link_proxy",
  } as const;
  metrics.submitPerformanceSample(epoch, "mobile.perf_launch", launch);
  metrics.submitPerformanceSample(epoch, "mobile.perf_js_tasks", jsTasks);
  metrics.submitPerformanceSample(epoch, "mobile.perf_frames", frames);
  metrics.submitPerformanceSample(epoch, "mobile.perf_chat_turn", chat);
  await settle();
  resume();
  await jest.advanceTimersByTimeAsync(60_000);
  const events = [
    ["mobile.perf_launch", launch],
    ["mobile.perf_js_tasks", jsTasks],
    ["mobile.perf_frames", frames],
    ["mobile.perf_chat_turn", chat],
  ] as const;
  expect(fetchMock).toHaveBeenCalledTimes(events.length);
  for (const [index, [url, options]] of fetchMock.mock.calls.entries()) {
    expect(url).toBe(destination.host + "/batch/");
    expect(options?.method).toBe("POST");
    const body = JSON.parse(String(options?.body));
    expect(body.api_key).toBe(destination.token);
    expect(body.batch).toHaveLength(1);
    const [name, fields] = events[index]!;
    expect(body.batch[0]).toEqual({
      type: "capture",
      event: name,
      distinct_id: expect.any(String),
      library: "posthog-react-native",
      library_version: expect.any(String),
      timestamp: expect.any(String),
      uuid: expect.any(String),
      properties: {
        ...fields,
        surface: "mobile",
        environment: "dev",
        platform: Platform.OS,
        client_version: "1.2.3",
        build_number: "42",
        $geoip_disable: true,
        $process_person_profile: false,
      },
    });
  }
});

test.each(["offline", "timeout"] as const)(
  "a send during %s is dropped after one fetch even on foreground and flush",
  async (mode) => {
    const client = new PerformancePostHog(destination);
    await client.ready();
    expect(fetchMock).not.toHaveBeenCalled();
    if (mode === "offline") {
      fetchMock.mockRejectedValue(new TypeError("offline"));
    } else {
      fetchMock.mockImplementation(
        (_url, options) =>
          new Promise((_resolve, reject) => {
            options!.signal!.addEventListener("abort", () =>
              reject(new Error("timed out")),
            );
          }),
      );
    }
    const sent = client.send("mobile.perf_chat_turn", {
      outcome: "timeout",
      duration_ms: 45000,
    });
    await settle();
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const signal = fetchMock.mock.calls[0]![1]!.signal!;
    if (mode === "timeout") {
      expect(signal.aborted).toBe(false);
      await jest.advanceTimersByTimeAsync(5000);
      expect(signal.aborted).toBe(true);
    }
    await sent;
    fetchMock.mockResolvedValue({ status: 200 } as Response);
    resume();
    await client.flush();
    await jest.advanceTimersByTimeAsync(60_000);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  },
);

test("revoking consent before dispatch drops pending work even if consent is granted again", async () => {
  await metrics.setPerformanceConsent(true);
  const old = metrics.beginPerformanceSample();
  expect(old).not.toBeNull();
  const chat = { outcome: "no_text", duration_ms: 200 } as const;
  metrics.submitPerformanceSample(old, "mobile.perf_chat_turn", chat);
  await metrics.setPerformanceConsent(false);
  await metrics.setPerformanceConsent(true);
  metrics.submitPerformanceSample(old, "mobile.perf_chat_turn", chat);
  await settle();
  expect(fetchMock).not.toHaveBeenCalled();
});

test("the device mirrors only the current project's grant and never reauthorizes the current cold launch on opt-in", async () => {
  const writes: Array<string | null> = [];
  const bridge = {
    setLaunchConsent: async (binding: string | null) => {
      writes.push(binding);
    },
  };
  const first = loadMetrics(bridge);
  await first.hydratePerformanceConsent();
  expect(writes).toEqual([null]);
  await first.setPerformanceConsent(true);
  expect(await first.getPerformanceColdGrant()).toBeNull();
  const restarted = loadMetrics(bridge);
  await restarted.hydratePerformanceConsent();
  expect(await restarted.getPerformanceColdGrant()).not.toBeNull();
  await restarted.setPerformanceConsent(false);
  expect(writes).toEqual([null, "a".repeat(64), "a".repeat(64), null]);
  expect(await restarted.getPerformanceColdGrant()).toBeNull();
});
