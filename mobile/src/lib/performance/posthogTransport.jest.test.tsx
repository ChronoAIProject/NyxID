import { setImmediate as nextTask } from "node:timers";
import { AppState, Platform } from "react-native";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import { createPerformanceConsent } from "./consent";
import { PerformancePostHog } from "./posthogTransport";

const { URL: NativeURL } = jest.requireActual<{ URL: typeof URL }>(
  "react-native/Libraries/Blob/URL",
);

// The Expo Jest preset preserves import(), which ordinarily requires Node's
// experimental VM modules flag. The shared harness compiles the production
// entry point to CommonJS and injects this exact SDK class at its lazy import.
function loadMetrics(
  nativeLaunch: {
    setLaunchConsent(binding: string | null): Promise<void>;
  } | null = null,
  host = destination.host,
) {
  const harness = hookHarness({
    "expo-constants": {
      expoConfig: {
        extra: {
          APP_ENV: "dev",
          PERFORMANCE_LAUNCH_BINDING: "a".repeat(64),
          PERFORMANCE_POSTHOG_TOKEN: destination.token,
          PERFORMANCE_POSTHOG_HOST: host,
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

test.each(["https://us.i.posthog.com", destination.host])(
  "a legacy trailing slash for %s hydrates the stored grant and sends to its origin",
  async (origin) => {
    const legacyHost = origin + "/";
    expect(new NativeURL(legacyHost).origin).toBe(origin);
    await AsyncStorage.setItem(
      "nyxid.performance-sharing.v1",
      JSON.stringify([origin, destination.token]),
    );
    const restarted = loadMetrics(null, legacyHost);
    expect(restarted.isPerformanceConfigured()).toBe(true);
    await restarted.hydratePerformanceConsent();
    const epoch = restarted.beginPerformanceSample();
    expect(epoch).not.toBeNull();
    expect(await restarted.getPerformanceColdGrant()).toBe(epoch);

    restarted.submitPerformanceSample(epoch, "mobile.perf_chat_turn", {
      outcome: "completed",
      duration_ms: 200,
    });
    await settle();
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(fetchMock.mock.calls[0]?.[0]).toBe(origin + "/batch/");
  },
);

test.each([
  ["https://INGEST.invalid:443/", destination.host],
  ["https://INGEST.invalid:08443/", destination.host + ":8443"],
  ["https://192.0.2.1:443/", "https://192.0.2.1"],
  ["https://[2001:DB8::1]:443/", "https://[2001:db8::1]"],
  ["https://[0:0:0:0:0:0:0:1]/", "https://[::1]"],
])("a stored RN origin from %s matches the configured origin %s", async (legacyHost, origin) => {
  const preferenceKey = "nyxid.performance-sharing.v1";
  const legacyGrant = JSON.stringify([new NativeURL(legacyHost).origin, destination.token]);
  await AsyncStorage.setItem(preferenceKey, legacyGrant);
  let finishMirror!: () => void;
  const pendingMirror = new Promise<void>((resolve) => {
    finishMirror = resolve;
  });
  const mirror = {
    setLaunchConsent: jest.fn((binding: string | null) =>
      binding ? pendingMirror : Promise.resolve(),
    ),
  };
  const restarted = loadMetrics(mirror, origin);
  await restarted.hydratePerformanceConsent();
  const epoch = restarted.beginPerformanceSample();
  expect(epoch).not.toBeNull();
  expect(await restarted.getPerformanceColdGrant()).toBe(epoch);
  expect(await AsyncStorage.getItem(preferenceKey)).toBe(legacyGrant);
  expect(mirror.setLaunchConsent).toHaveBeenCalledWith("a".repeat(64));

  restarted.submitPerformanceSample(epoch, "mobile.perf_chat_turn", {
    outcome: "completed",
    duration_ms: 200,
  });
  await settle();
  expect(fetchMock.mock.calls.map(([url]) => url)).toEqual([origin + "/batch/"]);

  restarted.submitPerformanceSample(epoch, "mobile.perf_chat_turn", {
    outcome: "completed",
    duration_ms: 200,
  });
  const withdrawn = restarted.setPerformanceConsent(false);
  expect(restarted.beginPerformanceSample()).toBeNull();
  finishMirror();
  await withdrawn;
  await settle();
  expect(mirror.setLaunchConsent.mock.calls).toEqual([["a".repeat(64)], [null]]);
  expect(await AsyncStorage.getItem(preferenceKey)).toBeNull();
  expect(fetchMock).toHaveBeenCalledTimes(1);
  await restarted.setPerformanceConsent(true);
  expect(await AsyncStorage.getItem(preferenceKey)).toBe(JSON.stringify([origin, destination.token]));
  expect(restarted.beginPerformanceSample()).not.toBe(epoch);
  expect(await restarted.getPerformanceColdGrant()).toBeNull();
});

const invalidOrigins = [
  "http://ingest.invalid",
  "https://",
  "https://user:password@ingest.invalid",
  "https://@ingest.invalid",
  " https://ingest.invalid",
  "https://ingest.invalid\n",
  "https://IN\tGEST.invalid",
  "https://ingest.invalid?",
  "https://ingest.invalid#",
  "https://ingest.invalid/private",
  "https://ingest%2einvalid",
  "https://ingest.invalid\\",
  "https://ingest.invalid:",
  "https://ingest.invalid:65536",
  "https://ingest.invalid:443x",
  "https://0x7f000001",
  "https://[:::]",
];

test.each(invalidOrigins)("invalid HTTPS origin %j cannot configure collection or hydrate consent", async (host) => {
  await AsyncStorage.setItem(
    "nyxid.performance-sharing.v1",
    JSON.stringify([host, destination.token]),
  );
  const configured = loadMetrics();
  await configured.hydratePerformanceConsent();
  expect(await configured.getPerformanceColdGrant()).toBeNull();
  expect(configured.beginPerformanceSample()).toBeNull();
  const injected = loadMetrics(null, host);
  expect(injected.isPerformanceConfigured()).toBe(false);
  await injected.setPerformanceConsent(true);
  expect(injected.beginPerformanceSample()).toBeNull();
  expect(fetchMock).not.toHaveBeenCalled();
});

test.each([
  ["a different host", ["https://another.invalid", destination.token]],
  ["a different port", [destination.host + ":8443", destination.token]],
  ["a changed token", [destination.host, destination.token + "x"]],
  ["a changed token case", [destination.host, destination.token.toUpperCase()]],
  ["an extra field", [destination.host, destination.token, true]],
])("a saved grant with %s stays off", async (_reason, value) => {
  await AsyncStorage.setItem("nyxid.performance-sharing.v1", JSON.stringify(value));
  const restarted = loadMetrics();
  await restarted.hydratePerformanceConsent();
  expect(await restarted.getPerformanceColdGrant()).toBeNull();
  expect(restarted.beginPerformanceSample()).toBeNull();
  expect(fetchMock).not.toHaveBeenCalled();
});

test("React Native's empty HTTPS origin disables collection before the SDK can use its default", async () => {
  expect(new NativeURL("https://").origin).toBe("");
  await AsyncStorage.setItem(
    "nyxid.performance-sharing.v1",
    JSON.stringify(["", destination.token]),
  );
  const malformed = loadMetrics(null, "https://");
  expect(malformed.isPerformanceConfigured()).toBe(false);
  await malformed.hydratePerformanceConsent();
  expect(await malformed.getPerformanceColdGrant()).toBeNull();
  await malformed.setPerformanceConsent(true);
  const epoch = malformed.beginPerformanceSample();
  expect(epoch).toBeNull();
  malformed.submitPerformanceSample(epoch, "mobile.perf_chat_turn", {
    outcome: "completed",
    duration_ms: 200,
  });
  await settle();
  expect(fetchMock).not.toHaveBeenCalled();
});

test("a saved JS grant hydrates through a native mirror failure and can be withdrawn", async () => {
  await metrics.setPerformanceConsent(true);
  const failedWrite = new Error("native disk write failed");
  const bridge = {
    setLaunchConsent: jest.fn(async (_binding: string | null) => {})
      .mockRejectedValueOnce(failedWrite)
      .mockRejectedValueOnce(failedWrite),
  };
  const restarted = loadMetrics(bridge);
  await expect(restarted.hydratePerformanceConsent()).resolves.toBeUndefined();
  const original = restarted.beginPerformanceSample();
  expect(original).not.toBeNull();
  expect(bridge.setLaunchConsent).toHaveBeenCalledWith("a".repeat(64));

  await expect(restarted.setPerformanceConsent(false)).rejects.toBe(failedWrite);
  expect(bridge.setLaunchConsent).toHaveBeenLastCalledWith(null);
  expect(restarted.getPerformanceConsentEpoch()).toBeNull();
  await expect(loadMetrics().getPerformanceColdGrant()).resolves.toBeNull();

  await restarted.setPerformanceConsent(true);
  expect(restarted.beginPerformanceSample()).not.toBeNull();
  expect(restarted.beginPerformanceSample()).not.toBe(original);
  expect(await restarted.getPerformanceColdGrant()).toBeNull();
});
