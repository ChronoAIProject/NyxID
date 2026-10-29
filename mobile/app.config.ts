import type { ExpoConfig, ConfigContext } from "expo/config";

// eslint-disable-next-line @typescript-eslint/no-require-imports
const { loadEnv, resolveProfile, appIdentity } = require("./scripts/lib/load-env");
// eslint-disable-next-line @typescript-eslint/no-require-imports
const { buildNativeLinks } = require("./scripts/lib/native-links");

export default ({ config }: ConfigContext): ExpoConfig => {
  const appEnv = (process.env.APP_ENV ?? "dev") as "dev" | "prod";
  const env = loadEnv();
  const r = resolveProfile(appEnv, env, { warnOnFrontendUrlFallback: true });
  const ident = appIdentity(env);

  // Keep public env inputs covered by the profile/identity cache key in metro.config.js.
  process.env.EXPO_PUBLIC_API_BASE_URL = r.apiBaseUrl;
  process.env.EXPO_PUBLIC_FRONTEND_URL = r.frontendUrl;
  process.env.EXPO_PUBLIC_LEGACY_FRONTEND_URL = r.legacyFrontendUrl;
  process.env.EXPO_PUBLIC_UNIVERSAL_LINK_HOST = r.universalLinkHost;
  process.env.EXPO_PUBLIC_APP_SCHEME = ident.scheme;
  process.env.EXPO_PUBLIC_ALLOWED_EMAILS = r.allowedEmails;
  process.env.EXPO_PUBLIC_DEV_MODE = appEnv === "dev" ? "true" : "false";

  const { associatedDomains, androidIntentFilters } = buildNativeLinks({
    apiBaseUrl: r.apiBaseUrl,
    universalLinkHost: r.universalLinkHost,
    universalLinkPathPrefix: r.universalLinkPathPrefix,
    scheme: ident.scheme,
  });

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
      infoPlist: {
        ITSAppUsesNonExemptEncryption: false,
        UISupportedInterfaceOrientations: [
          "UIInterfaceOrientationPortrait",
          "UIInterfaceOrientationLandscapeLeft",
          "UIInterfaceOrientationLandscapeRight",
        ],
        NSCameraUsageDescription:
          "Allow NyxID to scan login approval QR codes and take photos to attach in chats.",
        NSMicrophoneUsageDescription:
          "Allow NyxID to use your microphone for dictation and voice conversations.",
      },
    },
    android: {
      package: r.androidPackage,
      versionCode: Number(r.androidVersionCode),
      googleServicesFile: "./google-services.json",
      permissions: [
        "android.permission.CAMERA",
        "android.permission.RECORD_AUDIO",
      ],
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
      ["expo-screen-orientation", { initialOrientation: "PORTRAIT_UP" }],
      [
        "expo-image-picker",
        {
          photosPermission: false,
          cameraPermission: "Allow NyxID to scan login approval QR codes and take photos to attach in chats.",
          microphonePermission:
            "Allow NyxID to use your microphone for dictation and voice conversations.",
        },
      ],
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
      "expo-sharing",
      [
        "expo-camera",
        {
          cameraPermission: "Allow NyxID to scan login approval QR codes and take photos to attach in chats.",
          microphonePermission:
            "Allow NyxID to use your microphone for dictation and voice conversations.",
          recordAudioAndroid: true,
        },
      ],
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
