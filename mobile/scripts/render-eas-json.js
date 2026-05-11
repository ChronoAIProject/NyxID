#!/usr/bin/env node
/**
 * Renders mobile/eas.json from env (.env.dev / .env.prod / .env.local / process.env).
 *
 * Build profiles are static. Submit profiles are only emitted when the env
 * actually contains the credentials EAS needs — partial submit blocks would
 * fail EAS validation at config-load time and break every build invocation.
 *
 *   iOS submit  → requires APPLE_ID, APPLE_TEAM_ID, APPLE_ASC_APP_ID,
 *                 ASC_API_KEY_ID, ASC_API_KEY_ISSUER_ID, and the .p8 path.
 *                 If any are missing, submit.{dev,prod}.ios is omitted.
 *   Android submit → only needs the service-account JSON path (which is fixed).
 *                    Always emitted; submit fails at run time if the file
 *                    doesn't exist on disk.
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

const IOS_REQUIRED = [
  "APPLE_ID",
  "APPLE_TEAM_ID",
  "APPLE_ASC_APP_ID",
  "ASC_API_KEY_ID",
  "ASC_API_KEY_ISSUER_ID",
];

const iosSubmit = IOS_REQUIRED.every(has)
  ? {
      appleId: env.APPLE_ID,
      ascAppId: env.APPLE_ASC_APP_ID,
      appleTeamId: env.APPLE_TEAM_ID,
      ascApiKeyPath: "./credentials/asc-api-key.p8",
      ascApiKeyId: env.ASC_API_KEY_ID,
      ascApiKeyIssuerId: env.ASC_API_KEY_ISSUER_ID,
    }
  : null;

const androidSubmit = (track, extra = {}) => ({
  serviceAccountKeyPath: "./credentials/play-service-account.json",
  track,
  ...extra,
});

const submitProfile = (androidTrack, androidExtra) => {
  const block = { android: androidSubmit(androidTrack, androidExtra) };
  if (iosSubmit) block.ios = iosSubmit;
  return block;
};

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
    dev: submitProfile("internal"),
    prod: submitProfile("production", { releaseStatus: "draft" }),
  },
};

fs.writeFileSync(OUTPUT, JSON.stringify(easJson, null, 2) + "\n");
console.log(`[render-eas-json] wrote ${path.relative(ROOT, OUTPUT)}`);
