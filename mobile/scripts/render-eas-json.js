#!/usr/bin/env node
/**
 * Renders mobile/eas.json from env (.env.dev / .env.prod / .env.local / process.env).
 *
 * Build profiles are static; submit profiles are only emitted when the env actually
 * contains the credentials EAS needs. Partial submit blocks would fail EAS validation
 * at config-load time and break every build invocation.
 *
 *   iOS submit  → requires APPLE_ID, APPLE_TEAM_ID, ASC_API_KEY_ID, ASC_API_KEY_ISSUER_ID,
 *                 + DEV_APPLE_ASC_APP_ID or PROD_APPLE_ASC_APP_ID for the active profile
 *                 (falls back across profiles, same pattern as app.config.ts).
 *                 If any are missing, that profile's submit.ios block is omitted.
 *   Android submit → only needs the service-account JSON path (which is fixed).
 *                    Both dev and prod target Play's "Internal testing" track —
 *                    TestFlight equivalent, no Play review needed, immediate
 *                    availability to up to 100 testers. Operator promotes to
 *                    production via Play Console web UI.
 *
 * No-op when EAS_BUILD=true (EAS cloud worker), where .env.* files don't exist.
 */
const fs = require("fs");
const path = require("path");

if (process.env.EAS_BUILD === "true") {
  console.log("[render-eas-json] Skipped (running on EAS Build cloud).");
  process.exit(0);
}

const ROOT = path.join(__dirname, "..");
const OUTPUT = path.join(ROOT, "eas.json");

function parseEnvFile(file) {
  const p = path.join(ROOT, file);
  if (!fs.existsSync(p)) return {};
  try {
    const dotenv = require("dotenv");
    return dotenv.parse(fs.readFileSync(p));
  } catch (e) {
    if (e && e.code === "MODULE_NOT_FOUND") {
      console.error("[render-eas-json] `dotenv` is not installed. Run `pnpm install` in mobile/.");
      process.exit(1);
    }
    throw e;
  }
}

const env = {
  ...parseEnvFile(".env.dev"),
  ...parseEnvFile(".env.prod"),
  ...parseEnvFile(".env.local"),
  ...process.env,
};

const has = (k) => typeof env[k] === "string" && env[k].trim() !== "";

const APPLE_ACCT_REQUIRED = [
  "APPLE_ID",
  "APPLE_TEAM_ID",
  "ASC_API_KEY_ID",
  "ASC_API_KEY_ISSUER_ID",
];

function ascAppIdFor(profile) {
  const primary = profile === "dev" ? "DEV_APPLE_ASC_APP_ID" : "PROD_APPLE_ASC_APP_ID";
  const fallback = profile === "dev" ? "PROD_APPLE_ASC_APP_ID" : "DEV_APPLE_ASC_APP_ID";
  return env[primary] || env[fallback] || "";
}

function iosSubmit(profile) {
  const ascAppId = ascAppIdFor(profile);
  if (!APPLE_ACCT_REQUIRED.every(has) || !ascAppId) return null;
  return {
    appleId: env.APPLE_ID,
    ascAppId,
    appleTeamId: env.APPLE_TEAM_ID,
    ascApiKeyPath: "./credentials/asc-api-key.p8",
    ascApiKeyId: env.ASC_API_KEY_ID,
    ascApiKeyIssuerId: env.ASC_API_KEY_ISSUER_ID,
  };
}

function submitProfile(profile) {
  const block = {
    android: {
      serviceAccountKeyPath: "./credentials/play-service-account.json",
      track: "internal",
    },
  };
  const ios = iosSubmit(profile);
  if (ios) block.ios = ios;
  return block;
}

const easJson = {
  cli: {
    version: ">= 16.0.0",
    appVersionSource: "remote",
  },
  build: {
    dev: {
      distribution: "internal",
      autoIncrement: true,
      env: { APP_ENV: "dev" },
      ios: { simulator: false },
      android: { buildType: "apk" },
    },
    prod: {
      distribution: "store",
      autoIncrement: true,
      env: { APP_ENV: "prod" },
      android: { buildType: "app-bundle" },
    },
  },
  submit: {
    dev: submitProfile("dev"),
    prod: submitProfile("prod"),
  },
};

fs.writeFileSync(OUTPUT, JSON.stringify(easJson, null, 2) + "\n");
console.log(`[render-eas-json] wrote ${path.relative(ROOT, OUTPUT)}`);
