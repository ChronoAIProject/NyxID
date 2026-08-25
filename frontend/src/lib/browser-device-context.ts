import type { AuthDeviceRequestBody } from "@/schemas/auth-device";

interface UserAgentBrandVersion {
  readonly brand: string;
  readonly version: string;
}

interface HighEntropyValues {
  readonly architecture?: string;
  readonly bitness?: string;
  readonly model?: string;
  readonly platform?: string;
  readonly platformVersion?: string;
  readonly fullVersionList?: readonly UserAgentBrandVersion[];
  readonly brands?: readonly UserAgentBrandVersion[];
  readonly uaFullVersion?: string;
}

interface BrowserUserAgentData {
  readonly platform?: string;
  readonly mobile?: boolean;
  readonly getHighEntropyValues?: (
    hints: readonly string[],
  ) => Promise<HighEntropyValues>;
}

interface BrowserNavigatorContext {
  readonly userAgent: string;
  readonly language?: string;
  readonly hardwareConcurrency?: number;
  readonly deviceMemory?: number;
  readonly userAgentData?: BrowserUserAgentData;
}

export interface BrowserDeviceEnvironment {
  readonly navigator: BrowserNavigatorContext;
  readonly screen?: { readonly width: number; readonly height: number };
  readonly devicePixelRatio?: number;
  readonly timeZone?: string;
}

const HIGH_ENTROPY_TIMEOUT_MS = 300;

export async function collectBrowserDeviceContext(
  environment = currentBrowserEnvironment(),
  timeoutMs = HIGH_ENTROPY_TIMEOUT_MS,
): Promise<AuthDeviceRequestBody> {
  const navigatorContext = environment.navigator;
  const userAgent = cleanText(navigatorContext.userAgent, 512) ?? "browser";
  const fallback = parseUserAgent(userAgent);
  const highEntropy = await collectHighEntropyValues(
    navigatorContext.userAgentData,
    timeoutMs,
  );
  const app = highEntropyApp(highEntropy) ?? fallback.app;
  const platform = highEntropyPlatform(highEntropy) ?? fallback.platform;
  const model = cleanText(highEntropy?.model, 96);
  const formFactor = resolveFormFactor(
    userAgent,
    navigatorContext.userAgentData?.mobile,
  );

  return {
    client_label:
      cleanText(`${compactAppLabel(app)} on ${platform}`, 128) ?? "Web browser",
    client_user_agent: userAgent,
    client_app: cleanText(app, 96),
    client_platform: cleanText(platform, 96),
    client_model: model,
    client_form_factor: formFactor,
    client_timezone: cleanText(environment.timeZone, 64),
    client_locale: cleanText(navigatorContext.language, 35),
    client_screen_width: boundedInteger(environment.screen?.width, 32_768),
    client_screen_height: boundedInteger(environment.screen?.height, 32_768),
    client_device_pixel_ratio: boundedNumber(
      environment.devicePixelRatio,
      16,
    ),
    client_hardware_concurrency: boundedInteger(
      navigatorContext.hardwareConcurrency,
      1_024,
    ),
    client_device_memory: boundedNumber(navigatorContext.deviceMemory, 1_024),
  };
}

function currentBrowserEnvironment(): BrowserDeviceEnvironment {
  if (typeof navigator === "undefined") {
    return { navigator: { userAgent: "browser" } };
  }
  const navigatorContext = navigator as Navigator & {
    readonly deviceMemory?: number;
    readonly userAgentData?: BrowserUserAgentData;
  };
  let timeZone: string | undefined;
  try {
    timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    timeZone = undefined;
  }

  return {
    navigator: {
      userAgent: navigatorContext.userAgent,
      language: navigatorContext.language,
      hardwareConcurrency: navigatorContext.hardwareConcurrency,
      deviceMemory: navigatorContext.deviceMemory,
      userAgentData: navigatorContext.userAgentData,
    },
    screen:
      typeof window === "undefined"
        ? undefined
        : { width: window.screen.width, height: window.screen.height },
    devicePixelRatio:
      typeof window === "undefined" ? undefined : window.devicePixelRatio,
    timeZone,
  };
}

async function collectHighEntropyValues(
  userAgentData: BrowserUserAgentData | undefined,
  timeoutMs: number,
): Promise<HighEntropyValues | null> {
  if (!userAgentData?.getHighEntropyValues) return null;

  let timeout: ReturnType<typeof setTimeout> | undefined;
  const deadline = new Promise<null>((resolve) => {
    timeout = setTimeout(() => resolve(null), Math.max(0, timeoutMs));
  });
  let request: Promise<HighEntropyValues | null>;
  try {
    request = userAgentData
      .getHighEntropyValues([
        "architecture",
        "bitness",
        "model",
        "platform",
        "platformVersion",
        "fullVersionList",
        "uaFullVersion",
      ])
      .catch(() => null);
  } catch {
    if (timeout !== undefined) clearTimeout(timeout);
    return null;
  }
  const result = await Promise.race([request, deadline]);
  if (timeout !== undefined) clearTimeout(timeout);
  return result;
}

function highEntropyApp(values: HighEntropyValues | null): string | null {
  const list = values?.fullVersionList?.length
    ? values.fullVersionList
    : values?.brands;
  if (!list?.length) return null;
  const preferred = [
    ["Microsoft Edge", "Edge"],
    ["Google Chrome", "Chrome"],
    ["Opera", "Opera"],
    ["Chromium", "Chromium"],
  ] as const;
  for (const [brand, displayName] of preferred) {
    const match = list.find((item) => item.brand === brand);
    if (!match) continue;
    const version = cleanVersion(
      values?.fullVersionList?.length
        ? match?.version
        : values?.uaFullVersion ?? match?.version,
    );
    if (version) return `${displayName} ${version}`;
  }
  return null;
}

function compactAppLabel(app: string): string {
  return app.replace(/(\s[0-9]+)(?:\.[0-9]+)+$/, "$1");
}

function highEntropyPlatform(values: HighEntropyValues | null): string | null {
  if (!values) return null;
  const platform = canonicalPlatform(values.platform);
  if (!platform) return null;
  const version = cleanVersion(values.platformVersion);
  const architecture = canonicalArchitecture(values.architecture, values.bitness);
  const versioned = version ? `${platform} ${trimVersion(version)}` : platform;
  return architecture ? `${versioned} (${architecture})` : versioned;
}

function parseUserAgent(userAgent: string): { app: string; platform: string } {
  const app =
    versionAfter(userAgent, "EdgiOS/", "Edge") ??
    versionAfter(userAgent, "EdgA/", "Edge") ??
    versionAfter(userAgent, "Edg/", "Edge") ??
    versionAfter(userAgent, "CriOS/", "Chrome") ??
    versionAfter(userAgent, "Chrome/", "Chrome") ??
    versionAfter(userAgent, "FxiOS/", "Firefox") ??
    versionAfter(userAgent, "Firefox/", "Firefox") ??
    (userAgent.includes("Safari/")
      ? versionAfter(userAgent, "Version/", "Safari")
      : null) ??
    "Web browser";

  const macVersion = captureVersion(userAgent, /Mac OS X ([0-9_]+)/i);
  const iosVersion = captureVersion(userAgent, /(?:CPU (?:iPhone )?OS|iPhone OS) ([0-9_]+)/i);
  const androidVersion = captureVersion(userAgent, /Android ([0-9.]+)/i);
  const windowsVersion = captureVersion(userAgent, /Windows NT ([0-9.]+)/i);
  let platform = "Unknown platform";
  if (/iPhone|iPad|iPod/i.test(userAgent)) {
    platform = iosVersion ? `iOS ${trimVersion(iosVersion)}` : "iOS";
  } else if (/Android/i.test(userAgent)) {
    platform = androidVersion
      ? `Android ${trimVersion(androidVersion)}`
      : "Android";
  } else if (/Windows/i.test(userAgent)) {
    platform = windowsVersion
      ? `Windows ${trimVersion(windowsVersion)}`
      : "Windows";
  } else if (/Macintosh|Mac OS X/i.test(userAgent)) {
    platform = macVersion ? `macOS ${trimVersion(macVersion)}` : "macOS";
  } else if (/Linux|X11/i.test(userAgent)) {
    platform = "Linux";
  }
  return { app, platform };
}

function versionAfter(
  userAgent: string,
  marker: string,
  displayName: string,
): string | null {
  const start = userAgent.indexOf(marker);
  if (start < 0) return null;
  const version = cleanVersion(userAgent.slice(start + marker.length));
  return version ? `${displayName} ${version}` : null;
}

function captureVersion(userAgent: string, pattern: RegExp): string | null {
  const match = pattern.exec(userAgent);
  return cleanVersion(match?.[1]?.replaceAll("_", "."));
}

function cleanVersion(value: string | undefined): string | null {
  const version = value?.match(/^[0-9]+(?:\.[0-9]+){0,5}/)?.[0];
  return version ? version.slice(0, 32) : null;
}

function trimVersion(value: string): string {
  const segments = value.split(".");
  while (segments.length > 1 && segments.at(-1) === "0") segments.pop();
  return segments.join(".");
}

function canonicalPlatform(value: string | undefined): string | null {
  switch (value?.trim().toLowerCase()) {
    case "macos":
      return "macOS";
    case "windows":
      return "Windows";
    case "linux":
      return "Linux";
    case "android":
      return "Android";
    case "ios":
      return "iOS";
    case "chrome os":
      return "ChromeOS";
    default:
      return null;
  }
}

function canonicalArchitecture(
  architecture: string | undefined,
  bitness: string | undefined,
): string | null {
  const normalized = architecture?.trim().toLowerCase();
  if (normalized === "arm" && bitness === "64") return "arm64";
  if (normalized === "x86" && bitness === "64") return "x86_64";
  if (normalized === "x86" && bitness === "32") return "x86";
  return cleanText(normalized, 16) ?? null;
}

function resolveFormFactor(
  userAgent: string,
  userAgentMobile: boolean | undefined,
): "desktop" | "mobile" | "tablet" | "unknown" {
  if (/iPad|Tablet/i.test(userAgent)) return "tablet";
  if (userAgentMobile === true || /Mobile|iPhone|Android/i.test(userAgent)) {
    return "mobile";
  }
  if (userAgentMobile === false || /Windows|Macintosh|Linux|X11/i.test(userAgent)) {
    return "desktop";
  }
  return "unknown";
}

function cleanText(value: string | undefined, maxLength: number): string | undefined {
  if (value === undefined) return undefined;
  const cleaned = Array.from(value)
    .filter((character) => !/\p{Cc}/u.test(character))
    .slice(0, maxLength)
    .join("")
    .trim();
  return cleaned || undefined;
}

function boundedInteger(value: number | undefined, max: number): number | undefined {
  return value !== undefined && Number.isInteger(value) && value > 0 && value <= max
    ? value
    : undefined;
}

function boundedNumber(value: number | undefined, max: number): number | undefined {
  return value !== undefined && Number.isFinite(value) && value > 0 && value <= max
    ? value
    : undefined;
}
