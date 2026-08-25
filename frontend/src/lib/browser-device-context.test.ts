import { afterEach, describe, expect, it, vi } from "vitest";
import { collectBrowserDeviceContext } from "./browser-device-context";

const CHROME_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 " +
  "(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const SAFARI_UA =
  "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 " +
  "(KHTML, like Gecko) Version/18.2 Safari/605.1.15";

afterEach(() => {
  vi.useRealTimers();
});

describe("collectBrowserDeviceContext", () => {
  it("uses Chromium high-entropy values, ignores GREASE, composes arm64, and drops an empty model", async () => {
    const context = await collectBrowserDeviceContext({
      navigator: {
        userAgent: CHROME_UA,
        language: "en-SG",
        hardwareConcurrency: 12,
        deviceMemory: 16,
        userAgentData: {
          platform: "macOS",
          mobile: false,
          getHighEntropyValues: vi.fn().mockResolvedValue({
            architecture: "arm",
            bitness: "64",
            model: "",
            platform: "macOS",
            platformVersion: "26.5.2",
            uaFullVersion: "151.0.7922.174",
            brands: [
              { brand: "Not=A?Brand", version: "99" },
              { brand: "Google Chrome", version: "151" },
              { brand: "Chromium", version: "151" },
            ],
          }),
        },
      },
      screen: { width: 1512, height: 982 },
      devicePixelRatio: 2,
      timeZone: "Asia/Singapore",
    });

    expect(context).toEqual({
      client_label: "Chrome 151 on macOS 26.5.2 (arm64)",
      client_user_agent: CHROME_UA,
      client_app: "Chrome 151.0.7922.174",
      client_platform: "macOS 26.5.2 (arm64)",
      client_model: undefined,
      client_form_factor: "desktop",
      client_timezone: "Asia/Singapore",
      client_locale: "en-SG",
      client_screen_width: 1512,
      client_screen_height: 982,
      client_device_pixel_ratio: 2,
      client_hardware_concurrency: 12,
      client_device_memory: 16,
    });
  });

  it("falls back to UA parsing when UA-CH returns only a GREASE brand", async () => {
    const context = await collectBrowserDeviceContext({
      navigator: {
        userAgent: CHROME_UA,
        language: "en-US",
        userAgentData: {
          platform: "macOS",
          mobile: false,
          getHighEntropyValues: vi.fn().mockResolvedValue({
            architecture: "arm",
            bitness: "64",
            platform: "macOS",
            platformVersion: "26.5.2",
            uaFullVersion: "99.0.0.0",
            brands: [{ brand: "Not=A?Brand", version: "99" }],
          }),
        },
      },
      timeZone: "Asia/Singapore",
    });

    expect(context.client_app).toBe("Chrome 131.0.0.0");
    expect(context.client_label).toBe(
      "Chrome 131 on macOS 26.5.2 (arm64)",
    );
  });

  it("falls back to the user agent when high-entropy values are unsupported", async () => {
    const context = await collectBrowserDeviceContext({
      navigator: {
        userAgent: SAFARI_UA,
        language: "en-US",
        hardwareConcurrency: 8,
      },
      screen: { width: 1440, height: 900 },
      devicePixelRatio: 2,
      timeZone: "America/Los_Angeles",
    });

    expect(context.client_label).toBe("Safari 18 on macOS 10.15.7");
    expect(context.client_app).toBe("Safari 18.2");
    expect(context.client_platform).toBe("macOS 10.15.7");
    expect(context.client_form_factor).toBe("desktop");
  });

  it("falls back when high-entropy collection rejects", async () => {
    const context = await collectBrowserDeviceContext({
      navigator: {
        userAgent: CHROME_UA,
        language: "en-SG",
        userAgentData: {
          platform: "macOS",
          mobile: false,
          getHighEntropyValues: vi.fn().mockRejectedValue(new Error("denied")),
        },
      },
      timeZone: "Asia/Singapore",
    });

    expect(context.client_label).toBe("Chrome 131 on macOS 10.15.7");
  });

  it("falls back after the deadline instead of stalling QR creation", async () => {
    vi.useFakeTimers();
    const pending = collectBrowserDeviceContext(
      {
        navigator: {
          userAgent: CHROME_UA,
          language: "en-SG",
          userAgentData: {
            platform: "macOS",
            mobile: false,
            getHighEntropyValues: vi.fn(() => new Promise<never>(() => {})),
          },
        },
        timeZone: "Asia/Singapore",
      },
      300,
    );

    await vi.advanceTimersByTimeAsync(299);
    let settled = false;
    void pending.then(() => {
      settled = true;
    });
    await Promise.resolve();
    expect(settled).toBe(false);

    await vi.advanceTimersByTimeAsync(1);
    await expect(pending).resolves.toMatchObject({
      client_label: "Chrome 131 on macOS 10.15.7",
      client_form_factor: "desktop",
    });
  });
});
