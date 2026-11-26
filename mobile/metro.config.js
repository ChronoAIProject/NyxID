const path = require("node:path");
const { readFileSync } = require("node:fs");
const { createHash } = require("node:crypto");
const { getDefaultConfig } = require("expo/metro-config");
const { loadEnv, resolveProfile, appIdentity } = require("./scripts/lib/load-env");
const { lucideEntry } = require("./scripts/babel-lucide-icons");

const env = loadEnv();
const appEnv = process.env.APP_ENV ?? "dev";
const profile = resolveProfile(appEnv, env);
const config = getDefaultConfig(__dirname);
config.watchFolders = [...config.watchFolders, path.resolve(__dirname, "../shared")];

// Invalidate compiled domains and icon imports when their build inputs change.
const transformHash = createHash("sha256")
  .update(JSON.stringify({ appEnv, profile, identity: appIdentity(env) }))
  .update(readFileSync(require.resolve("./babel.config.js")))
  .update(readFileSync(require.resolve("./scripts/babel-lucide-icons")))
  .update(readFileSync(lucideEntry))
  .digest("hex");
config.cacheVersion = `${config.cacheVersion}:${transformHash}`;

module.exports = config;
