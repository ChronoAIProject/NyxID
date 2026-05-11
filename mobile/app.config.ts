import dotenv from "dotenv";
import * as fs from "fs";
import * as path from "path";
import type { ExpoConfig, ConfigContext } from "expo/config";

type Profile = {
  apiBaseUrl: string;
  iosBundleId: string;
  androidPackage: string;
  universalLinkHost: string;
  universalLinkPathPrefix: string;
  allowedEmails: string;
  telemetryDsn: string;
  telemetryHost: string;
  shareAnalytics: string;
};

function loadEnvFile(file: string): Record<string, string> {
  const p = path.join(__dirname, file);
  return fs.existsSync(p) ? dotenv.parse(fs.readFileSync(p)) : {};
}

function readProfile(env: Record<string, string>, prefix: "DEV" | "PROD"): Profile {
  return {
    apiBaseUrl: env[`${prefix}_API_BASE_URL`] ?? "",
    iosBundleId: env[`${prefix}_IOS_BUNDLE_ID`] ?? "",
    androidPackage: env[`${prefix}_ANDROID_PACKAGE`] ?? "",
    universalLinkHost: env[`${prefix}_UNIVERSAL_LINK_HOST`] ?? "",
    universalLinkPathPrefix: env[`${prefix}_UNIVERSAL_LINK_PATH_PREFIX`] ?? "",
    allowedEmails: env[`${prefix}_ALLOWED_EMAILS`] ?? "",
    telemetryDsn: env[`${prefix}_TELEMETRY_DSN`] ?? "",
    telemetryHost: env[`${prefix}_TELEMETRY_HOST`] ?? "",
    shareAnalytics: env[`${prefix}_SHARE_ANALYTICS`] ?? "",
  };
}

function fatal(msg: string): never {
  throw new Error(
    "\n\n========================================\n" +
      "[app.config.ts] " +
      msg +
      "\n========================================\n",
  );
}

function resolveProfile(
  appEnv: "dev" | "prod",
  merged: Record<string, string>,
): Profile {
  const dev = readProfile(merged, "DEV");
  const prod = readProfile(merged, "PROD");

  if (!dev.apiBaseUrl && !prod.apiBaseUrl) {
    fatal(
      "FATAL: both DEV_API_BASE_URL and PROD_API_BASE_URL are empty.\n" +
        "Copy mobile/.env.example to mobile/.env.dev and/or mobile/.env.prod\n" +
        "and set API_BASE_URL at minimum.",
    );
  }

  const primary = appEnv === "dev" ? dev : prod;
  const fallback = appEnv === "dev" ? prod : dev;
  const fallbackName = appEnv === "dev" ? "PROD" : "DEV";

  if (!primary.apiBaseUrl) {
    console.warn(
      `[app.config.ts] ${appEnv.toUpperCase()}_API_BASE_URL empty — falling back to ${fallbackName}_* values for missing fields.`,
    );
  }

  const pick = (k: keyof Profile): string => primary[k] || fallback[k] || "";

  return {
    apiBaseUrl: pick("apiBaseUrl"),
    iosBundleId: pick("iosBundleId"),
    androidPackage: pick("androidPackage"),
    universalLinkHost: pick("universalLinkHost"),
    universalLinkPathPrefix: pick("universalLinkPathPrefix"),
    allowedEmails: pick("allowedEmails"),
    telemetryDsn: pick("telemetryDsn"),
    telemetryHost: pick("telemetryHost"),
    shareAnalytics: pick("shareAnalytics") || "false",
  };
}

function safeHost(url: string): string | null {
  try {
    return new URL(url).host;
  } catch {
    return null;
  }
}

export default ({ config }: ConfigContext): ExpoConfig => {
  const appEnv = (process.env.APP_ENV ?? "dev") as "dev" | "prod";
  const merged: Record<string, string> = {
    ...loadEnvFile(".env.dev"),
    ...loadEnvFile(".env.prod"),
    ...loadEnvFile(".env.local"),
    ...(process.env as Record<string, string>),
  };

  const r = resolveProfile(appEnv, merged);

  if (!r.iosBundleId) {
    fatal(
      "FATAL: no IOS_BUNDLE_ID set in either DEV or PROD profile.\n" +
        "Set DEV_IOS_BUNDLE_ID and/or PROD_IOS_BUNDLE_ID in mobile/.env.{dev,prod}.",
    );
  }
  if (!r.androidPackage) {
    fatal(
      "FATAL: no ANDROID_PACKAGE set in either DEV or PROD profile.\n" +
        "Set DEV_ANDROID_PACKAGE and/or PROD_ANDROID_PACKAGE in mobile/.env.{dev,prod}.",
    );
  }

  const easProjectId = merged.EAS_PROJECT_ID ?? "";
  if (!easProjectId) {
    fatal(
      "FATAL: EAS_PROJECT_ID is not set.\n" +
        "Run `eas init` to create a project (or copy the ID from your EAS dashboard)\n" +
        "and add EAS_PROJECT_ID=... to mobile/.env.local (or .env.{dev,prod}).",
    );
  }
  const appName = merged.APP_NAME || "NyxID Mobile";
  const appSlug = merged.APP_SLUG || "nyxid-mobile";
  const appScheme = merged.APP_SCHEME || "nyxid";

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
      data: [{ scheme: appScheme }],
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
    name: appName,
    slug: appSlug,
    scheme: appScheme,
    version: "1.0.1",
    orientation: "portrait",
    icon: "./assets/icon.png",
    userInterfaceStyle: "automatic",
    splash: {
      image: "./assets/splash.png",
      resizeMode: "contain",
      backgroundColor: "#10101A",
    },
    assetBundlePatterns: ["**/*"],
    ios: {
      supportsTablet: false,
      bundleIdentifier: r.iosBundleId,
      associatedDomains,
      infoPlist: { ITSAppUsesNonExemptEncryption: false },
    },
    android: {
      package: r.androidPackage,
      googleServicesFile: "./google-services.json",
      blockedPermissions: [
        "android.permission.READ_EXTERNAL_STORAGE",
        "android.permission.WRITE_EXTERNAL_STORAGE",
      ],
      adaptiveIcon: {
        foregroundImage: "./assets/adaptive-icon.png",
        backgroundColor: "#10101A",
      },
      intentFilters: androidIntentFilters,
    },
    plugins: [
      "expo-secure-store",
      [
        "expo-notifications",
        {
          enableBackgroundRemoteNotifications: true,
          icon: "./assets/notification-icon.png",
          color: "#8B5CF6",
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
      eas: { projectId: easProjectId },
    },
  };
};
