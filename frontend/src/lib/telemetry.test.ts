import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// PostHog mock. `vi.mock` is hoisted above imports, so the factory can't
// reference a top-level `const`. `vi.hoisted` lets us declare the spy
// object in the same hoisted scope so `./telemetry`'s `import posthog
// from 'posthog-js'` resolves to these stubs from the first call.
const mockPosthog = vi.hoisted(() => ({
  init: vi.fn(),
  capture: vi.fn(),
  captureException: vi.fn(),
  identify: vi.fn(),
  reset: vi.fn(),
  opt_out_capturing: vi.fn(),
  opt_in_capturing: vi.fn(),
  has_opted_out_capturing: vi.fn(() => false),
}));

vi.mock("posthog-js", () => ({
  default: mockPosthog,
}));

import {
  initTelemetry,
  isTelemetryActive,
  disableTelemetry,
  identify,
  reset as telemetryReset,
} from "./telemetry";

const validArgs = {
  dsn: "phc_test_dsn",
  host: "https://us.i.posthog.com",
  shareBack: false,
  consent: true,
};

beforeEach(() => {
  // Clear spy history. Module-level `inited` state is reset via the
  // `disableTelemetry()` call in afterEach of the prior test.
  vi.clearAllMocks();
});

afterEach(() => {
  // Return `telemetry.ts` module state to pre-init. Safe to call
  // unconditionally — it's a no-op when `inited === false`.
  disableTelemetry();
});

describe("isTelemetryActive", () => {
  it("returns false before any init", () => {
    expect(isTelemetryActive()).toBe(false);
  });

  it("returns true after successful init", () => {
    initTelemetry(validArgs);
    expect(isTelemetryActive()).toBe(true);
  });

  it("returns false after disableTelemetry", () => {
    initTelemetry(validArgs);
    disableTelemetry();
    expect(isTelemetryActive()).toBe(false);
  });

  it("returns false when navigator.doNotTrack is '1', even after successful init", () => {
    // The privacy policy promises we honor DNT. This must be true
    // across the whole surface, not just inside the PostHog SDK —
    // api-client.ts reads this flag to decide whether to send
    // `X-NyxID-Client: ui` headers, which drive backend-side
    // surface-tagged telemetry.
    const original = Object.getOwnPropertyDescriptor(
      Navigator.prototype,
      "doNotTrack",
    );
    Object.defineProperty(navigator, "doNotTrack", {
      value: "1",
      configurable: true,
    });
    try {
      initTelemetry(validArgs);
      expect(isTelemetryActive()).toBe(false);
    } finally {
      if (original) {
        Object.defineProperty(navigator, "doNotTrack", original);
      } else {
        // happy-dom may not have set a descriptor; best-effort reset.
        Object.defineProperty(navigator, "doNotTrack", {
          value: "unspecified",
          configurable: true,
        });
      }
    }
  });
});

describe("initTelemetry", () => {
  it("no-ops when consent is false", () => {
    initTelemetry({ ...validArgs, consent: false });
    expect(mockPosthog.init).not.toHaveBeenCalled();
    expect(isTelemetryActive()).toBe(false);
  });

  it("no-ops when dsn is empty", () => {
    initTelemetry({ ...validArgs, dsn: "" });
    expect(mockPosthog.init).not.toHaveBeenCalled();
    expect(isTelemetryActive()).toBe(false);
  });

  it("calls posthog.init exactly once when consent + dsn are present", () => {
    initTelemetry(validArgs);
    expect(mockPosthog.init).toHaveBeenCalledTimes(1);
    expect(mockPosthog.init).toHaveBeenCalledWith(
      "phc_test_dsn",
      expect.objectContaining({
        api_host: "https://us.i.posthog.com",
        mask_all_text: true,
        respect_dnt: true,
        ip: false,
      }),
    );
  });

  it("is idempotent — second call with same args is a no-op", () => {
    initTelemetry(validArgs);
    initTelemetry(validArgs);
    expect(mockPosthog.init).toHaveBeenCalledTimes(1);
  });

  it("defaults host to PostHog US when host is empty", () => {
    initTelemetry({ ...validArgs, host: "" });
    expect(mockPosthog.init).toHaveBeenCalledWith(
      "phc_test_dsn",
      expect.objectContaining({ api_host: "https://us.i.posthog.com" }),
    );
  });
});

describe("disableTelemetry", () => {
  it("no-ops when not inited", () => {
    disableTelemetry();
    expect(mockPosthog.opt_out_capturing).not.toHaveBeenCalled();
    expect(mockPosthog.reset).not.toHaveBeenCalled();
  });

  it("calls opt_out_capturing and reset when inited", () => {
    initTelemetry(validArgs);
    disableTelemetry();
    expect(mockPosthog.opt_out_capturing).toHaveBeenCalledTimes(1);
    expect(mockPosthog.reset).toHaveBeenCalledTimes(1);
  });

  it("flips isTelemetryActive to false", () => {
    initTelemetry(validArgs);
    expect(isTelemetryActive()).toBe(true);
    disableTelemetry();
    expect(isTelemetryActive()).toBe(false);
  });

  it("swallows vendor errors during teardown", () => {
    initTelemetry(validArgs);
    mockPosthog.opt_out_capturing.mockImplementationOnce(() => {
      throw new Error("SDK teardown failed");
    });
    // Must not throw — state reset is the source of truth for the app.
    expect(() => disableTelemetry()).not.toThrow();
    expect(isTelemetryActive()).toBe(false);
  });
});

describe("re-init after disable (consent withdrawal + re-opt-in flow)", () => {
  it("runs initTelemetry cleanly after disableTelemetry", () => {
    initTelemetry(validArgs);
    expect(mockPosthog.init).toHaveBeenCalledTimes(1);

    disableTelemetry();
    expect(isTelemetryActive()).toBe(false);

    initTelemetry(validArgs);
    expect(mockPosthog.init).toHaveBeenCalledTimes(2);
    expect(isTelemetryActive()).toBe(true);
  });

  it("clears the persistent PostHog opt-out flag on re-enable so events flow again", () => {
    // PostHog's `opt_out_capturing` writes a flag to localStorage that
    // survives `init()`. Without this explicit `opt_in_capturing`, a
    // user who toggles OFF in Settings and later toggles back ON would
    // silently get no events — `telemetryActive` says yes but PostHog's
    // own flag says no.
    mockPosthog.has_opted_out_capturing.mockReturnValue(true);
    initTelemetry(validArgs);
    expect(mockPosthog.opt_in_capturing).toHaveBeenCalledTimes(1);
  });

  it("does not fire opt_in_capturing on a fresh init (avoids noisy $opt_in events)", () => {
    mockPosthog.has_opted_out_capturing.mockReturnValue(false);
    initTelemetry(validArgs);
    expect(mockPosthog.opt_in_capturing).not.toHaveBeenCalled();
  });
});

describe("emit helpers (identify / reset / capture) are gated on init", () => {
  it("identify is a no-op before init", () => {
    identify("user-123");
    expect(mockPosthog.identify).not.toHaveBeenCalled();
  });

  it("identify calls posthog.identify after init", () => {
    initTelemetry(validArgs);
    identify("user-123");
    expect(mockPosthog.identify).toHaveBeenCalledWith("user-123");
  });

  it("identify is a no-op with empty user id", () => {
    initTelemetry(validArgs);
    identify("");
    expect(mockPosthog.identify).not.toHaveBeenCalled();
  });

  it("reset is a no-op before init", () => {
    telemetryReset();
    expect(mockPosthog.reset).not.toHaveBeenCalled();
  });

  it("reset calls posthog.reset after init", () => {
    initTelemetry(validArgs);
    telemetryReset();
    expect(mockPosthog.reset).toHaveBeenCalledTimes(1);
  });
});
