#!/usr/bin/env node
/**
 * Increments build numbers in mobile/.env.prod (or .env.dev) before a release.
 *
 * Usage:
 *   node scripts/bump-version.js ios            # bumps PROD_IOS_BUILD_NUMBER
 *   node scripts/bump-version.js android        # bumps PROD_ANDROID_VERSION_CODE
 *   node scripts/bump-version.js both           # both, in PROD
 *   node scripts/bump-version.js ios dev        # bumps DEV_IOS_BUILD_NUMBER
 *
 * Reads the current value (or starts at 1 if unset), increments by 1, writes back.
 * If the relevant *_BUILD_NUMBER line doesn't exist in the .env file, appends it.
 */
const fs = require("fs");
const path = require("path");

const target = process.argv[2];
const profile = (process.argv[3] || "prod").toLowerCase();
if (!["ios", "android", "both"].includes(target)) {
  console.error("Usage: node scripts/bump-version.js <ios|android|both> [dev|prod]");
  process.exit(1);
}
if (!["dev", "prod"].includes(profile)) {
  console.error("Profile must be 'dev' or 'prod'");
  process.exit(1);
}

const envFile = path.join(__dirname, "..", `.env.${profile}`);
if (!fs.existsSync(envFile)) {
  console.error(`${envFile} does not exist. Create it from .env.example first.`);
  process.exit(1);
}

const PREFIX = profile.toUpperCase();
const targets = target === "both" ? ["ios", "android"] : [target];

let content = fs.readFileSync(envFile, "utf8");

for (const t of targets) {
  const key =
    t === "ios" ? `${PREFIX}_IOS_BUILD_NUMBER` : `${PREFIX}_ANDROID_VERSION_CODE`;
  const re = new RegExp(`^(${key}\\s*=\\s*)(\\d*)\\s*$`, "m");
  const match = content.match(re);
  let current = 0;
  if (match) {
    current = parseInt(match[2] || "0", 10);
  }
  const next = current + 1;
  if (match) {
    content = content.replace(re, `$1${next}`);
  } else {
    if (!content.endsWith("\n")) content += "\n";
    content += `${key}=${next}\n`;
  }
  console.log(`${key}: ${current || "(unset)"} → ${next}`);
}

fs.writeFileSync(envFile, content);
console.log(`\nWrote ${envFile}`);
