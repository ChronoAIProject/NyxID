import type { ExpoConfig, ConfigContext } from "expo/config";

// eslint-disable-next-line @typescript-eslint/no-require-imports
const { loadEnv, resolveProfile, appIdentity } = require("./scripts/lib/load-env");

function safeHost(url: string): string | null {
  try {
    return new URL(url).host;
  } catch {
    return null;
  }
}

export default ({ config }: ConfigContext): ExpoConfig => {
  const appEnv = (process.env.APP_ENV ?? "dev") as "dev" | "prod";
  const env = loadEnv();
  const r = resolveProfile(appEnv, env);
  const ident = appIdentity(env);

  process.env.EXPO_PUBLIC_API_BASE_URL = r.apiBaseUrl;
  process.env.EXPO_PUBLIC_ALLOWED_EMAILS = r.allowedEmails;
  process.env.EXPO_PUBLIC_DEV_MODE = appEnv === "dev" ? "true" : "false";

  const apiHost = safeHost(r.apiBaseUrl);
  const associatedDomains: string[] = [];
  if (apiHost) associatedDomains.push(`applinks:${apiHost}`);
  if (r.universalLinkHost) associatedDomains.push(`applinks:${r.universalLinkHost}`);

  const androidIntentFilters: NonNullable<ExpoConfig["android"]>["intentFilters"] = [
    {
      action: "VIEW",
      data: [{ scheme: ident.scheme }],
      category: ["BROWSABLE", "DEFAULT"],
    },
  ];
  if (r.universalLinkHost) {
    const data: { scheme: string; host: string; pathPrefix?: string } = {
      scheme: "https",
      host: r.universalLinkHost,
    };
    if (r.universalLinkPathPrefix) data.pathPrefix = r.universalLinkPathPrefix;
    androidIntentFilters.unshift({
      action: "VIEW",
      autoVerify: true,
      data: [data],
      category: ["BROWSABLE", "DEFAULT"],
    });
  }

  return {
    ...config,
    name: ident.name,
    slug: ident.slug,
    scheme: ident.scheme,
    version: ident.version,
    orientation: "portrait",
    icon: "./assets/icon.png",
    userInterfaceStyle: "automatic",
    assetBundlePatterns: ["**/*"],
    ios: {
      supportsTablet: false,
      bundleIdentifier: r.iosBundleId,
      buildNumber: r.iosBuildNumber,
      associatedDomains,
      infoPlist: { ITSAppUsesNonExemptEncryption: false },
    },
    android: {
      package: r.androidPackage,
      versionCode: Number(r.androidVersionCode),
      googleServicesFile: "./google-services.json",
      blockedPermissions: [
        "android.permission.READ_EXTERNAL_STORAGE",
        "android.permission.WRITE_EXTERNAL_STORAGE",
      ],
      adaptiveIcon: {
        foregroundImage: "./assets/adaptive-icon.png",
        backgroundColor: "#07060e",
      },
      intentFilters: androidIntentFilters,
    },
    plugins: [
      "expo-secure-store",
      [
        "expo-splash-screen",
        {
          image: "./assets/adaptive-icon.png",
          backgroundColor: "#07060e",
          imageWidth: 220,
          resizeMode: "contain",
        },
      ],
      [
        "expo-notifications",
        {
          enableBackgroundRemoteNotifications: true,
          icon: "./assets/notification-icon.png",
          color: "#9775fa",
          androidMode: "default",
          defaultChannel: "approvals",
        },
      ],
      "expo-font",
      "expo-web-browser",
    ],
    extra: {
      APP_ENV: appEnv,
      TELEMETRY_DSN: r.telemetryDsn,
      TELEMETRY_HOST: r.telemetryHost,
      NYXID_SHARE_ANALYTICS: r.shareAnalytics,
      LEGAL_BASE_URL: r.legalBaseUrl,
    },
  };
};
