/**
 * Shared env loader + DEV↔PROD profile resolver.
 *
 * Consumed by app.config.ts and by every script under mobile/scripts/.
 * Loads .env.dev / .env.prod / .env.local from mobile/, merged with
 * process.env (later sources win). Then applies per-field bidirectional
 * fallback: active profile's value first, other profile's value as fallback.
 *
 * Fatal errors are thrown (not console.error'd) so callers control exit.
 */
const fs = require("fs");
const path = require("path");

const MOBILE_ROOT = path.join(__dirname, "..", "..");

function parseEnvFile(file) {
  const p = path.join(MOBILE_ROOT, file);
  if (!fs.existsSync(p)) return {};
  const dotenv = require("dotenv");
  return dotenv.parse(fs.readFileSync(p));
}

function loadEnv() {
  return {
    ...parseEnvFile(".env.dev"),
    ...parseEnvFile(".env.prod"),
    ...parseEnvFile(".env.local"),
    ...process.env,
  };
}

function fatal(message) {
  throw new Error(
    "\n\n========================================\n" +
      message +
      "\n========================================\n",
  );
}

function readProfile(env, prefix) {
  return {
    apiBaseUrl: env[`${prefix}_API_BASE_URL`] || "",
    iosBundleId: env[`${prefix}_IOS_BUNDLE_ID`] || "",
    androidPackage: env[`${prefix}_ANDROID_PACKAGE`] || "",
    appleAscAppId: env[`${prefix}_APPLE_ASC_APP_ID`] || "",
    iosBuildNumber: env[`${prefix}_IOS_BUILD_NUMBER`] || "",
    androidVersionCode: env[`${prefix}_ANDROID_VERSION_CODE`] || "",
    universalLinkHost: env[`${prefix}_UNIVERSAL_LINK_HOST`] || "",
    universalLinkPathPrefix: env[`${prefix}_UNIVERSAL_LINK_PATH_PREFIX`] || "",
    legalBaseUrl: env[`${prefix}_LEGAL_BASE_URL`] || "",
    allowedEmails: env[`${prefix}_ALLOWED_EMAILS`] || "",
    telemetryDsn: env[`${prefix}_TELEMETRY_DSN`] || "",
    telemetryHost: env[`${prefix}_TELEMETRY_HOST`] || "",
    shareAnalytics: env[`${prefix}_SHARE_ANALYTICS`] || "",
  };
}

function resolveProfile(appEnv, env) {
  if (appEnv !== "dev" && appEnv !== "prod") {
    fatal(`APP_ENV must be "dev" or "prod"; got "${appEnv}"`);
  }

  const dev = readProfile(env, "DEV");
  const prod = readProfile(env, "PROD");

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
      `[load-env] ${appEnv.toUpperCase()}_API_BASE_URL empty — falling back to ${fallbackName}_* values for missing fields.`,
    );
  }

  const pick = (k) => primary[k] || fallback[k] || "";

  const resolved = {
    apiBaseUrl: pick("apiBaseUrl"),
    iosBundleId: pick("iosBundleId"),
    androidPackage: pick("androidPackage"),
    appleAscAppId: pick("appleAscAppId"),
    iosBuildNumber: pick("iosBuildNumber") || "1",
    androidVersionCode: pick("androidVersionCode") || "1",
    universalLinkHost: pick("universalLinkHost"),
    universalLinkPathPrefix: pick("universalLinkPathPrefix"),
    legalBaseUrl: pick("legalBaseUrl"),
    allowedEmails: pick("allowedEmails"),
    telemetryDsn: pick("telemetryDsn"),
    telemetryHost: pick("telemetryHost"),
    shareAnalytics: pick("shareAnalytics") || "false",
  };

  if (!resolved.iosBundleId) {
    fatal(
      "FATAL: no IOS_BUNDLE_ID set in either DEV or PROD profile.\n" +
        "Set DEV_IOS_BUNDLE_ID and/or PROD_IOS_BUNDLE_ID in mobile/.env.{dev,prod}.",
    );
  }
  if (!resolved.androidPackage) {
    fatal(
      "FATAL: no ANDROID_PACKAGE set in either DEV or PROD profile.\n" +
        "Set DEV_ANDROID_PACKAGE and/or PROD_ANDROID_PACKAGE in mobile/.env.{dev,prod}.",
    );
  }

  return resolved;
}

function appIdentity(env) {
  return {
    name: env.APP_NAME || "NyxID Mobile",
    slug: env.APP_SLUG || "nyxid-mobile",
    scheme: env.APP_SCHEME || "nyxid",
    version: env.APP_VERSION || "1.0.1",
  };
}

module.exports = {
  loadEnv,
  resolveProfile,
  appIdentity,
  fatal,
  MOBILE_ROOT,
};
