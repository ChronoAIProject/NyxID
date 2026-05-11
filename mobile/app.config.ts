import dotenv from "dotenv";
import * as fs from "fs";
import * as path from "path";
import type { ExpoConfig, ConfigContext } from "expo/config";

type Profile = {
  apiBaseUrl: string;
  telemetryDsn: string;
  telemetryHost: string;
  shareAnalytics: string;
  allowedEmails: string;
};

function loadEnvFile(file: string): Record<string, string> {
  const p = path.join(__dirname, file);
  return fs.existsSync(p) ? dotenv.parse(fs.readFileSync(p)) : {};
}

function readProfile(merged: Record<string, string>, prefix: "DEV" | "PROD"): Profile {
  return {
    apiBaseUrl: merged[`${prefix}_API_BASE_URL`] ?? "",
    telemetryDsn: merged[`${prefix}_TELEMETRY_DSN`] ?? "",
    telemetryHost: merged[`${prefix}_TELEMETRY_HOST`] ?? "",
    shareAnalytics: merged[`${prefix}_SHARE_ANALYTICS`] ?? "false",
    allowedEmails: merged[`${prefix}_ALLOWED_EMAILS`] ?? "",
  };
}

const isEmpty = (p: Profile) => !p.apiBaseUrl;

function resolveProfile(appEnv: "dev" | "prod", merged: Record<string, string>): Profile {
  const dev = readProfile(merged, "DEV");
  const prod = readProfile(merged, "PROD");

  if (isEmpty(dev) && isEmpty(prod)) {
    throw new Error(
      "\n\n========================================\n" +
        "[app.config.ts] FATAL: both DEV_* and PROD_* env blocks are empty.\n" +
        "Copy mobile/.env.example to mobile/.env.dev and/or mobile/.env.prod\n" +
        "and set API_BASE_URL at minimum.\n" +
        "========================================\n",
    );
  }

  if (appEnv === "dev" && isEmpty(dev)) {
    console.warn("[app.config.ts] DEV profile empty — falling back to PROD config.");
    return prod;
  }
  if (appEnv === "prod" && isEmpty(prod)) {
    console.warn("[app.config.ts] PROD profile empty — falling back to DEV config.");
    return dev;
  }
  return appEnv === "dev" ? dev : prod;
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

  const resolved = resolveProfile(appEnv, merged);

  process.env.EXPO_PUBLIC_API_BASE_URL = resolved.apiBaseUrl;
  process.env.EXPO_PUBLIC_ALLOWED_EMAILS = resolved.allowedEmails;
  process.env.EXPO_PUBLIC_DEV_MODE = appEnv === "dev" ? "true" : "false";

  const apiHost = safeHost(resolved.apiBaseUrl);
  const associatedDomains = ["applinks:nyxid.onelink.me"];
  if (apiHost) associatedDomains.push(`applinks:${apiHost}`);

  return {
    ...config,
    name: "NyxID Mobile",
    slug: "nyxid-mobile",
    scheme: "nyxid",
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
      bundleIdentifier: "fun.chrono-ai.nyxid",
      associatedDomains,
      infoPlist: { ITSAppUsesNonExemptEncryption: false },
    },
    android: {
      package: "fun.chronoai.nyxid",
      googleServicesFile: "./google-services.json",
      blockedPermissions: [
        "android.permission.READ_EXTERNAL_STORAGE",
        "android.permission.WRITE_EXTERNAL_STORAGE",
      ],
      adaptiveIcon: {
        foregroundImage: "./assets/adaptive-icon.png",
        backgroundColor: "#10101A",
      },
      intentFilters: [
        {
          action: "VIEW",
          autoVerify: true,
          data: [
            {
              scheme: "https",
              host: "nyxid.onelink.me",
              pathPrefix: "/REzJ",
            },
          ],
          category: ["BROWSABLE", "DEFAULT"],
        },
        {
          action: "VIEW",
          data: [{ scheme: "nyxid" }],
          category: ["BROWSABLE", "DEFAULT"],
        },
      ],
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
      TELEMETRY_DSN: resolved.telemetryDsn,
      TELEMETRY_HOST: resolved.telemetryHost,
      NYXID_SHARE_ANALYTICS: resolved.shareAnalytics,
      eas: { projectId: "84ed5f10-0e59-46db-bf71-799e3ddd6ba9" },
    },
  };
};
