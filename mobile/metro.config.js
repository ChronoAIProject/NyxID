const { createHash } = require("node:crypto");
const { getDefaultConfig } = require("expo/metro-config");
const { loadEnv, resolveProfile, appIdentity } = require("./scripts/lib/load-env");

const env = loadEnv();
const appEnv = process.env.APP_ENV ?? "dev";
const profile = resolveProfile(appEnv, env);
const config = getDefaultConfig(__dirname);

// Metro caches production env substitutions. Include the resolved build profile
// so changing domains cannot reuse JavaScript compiled for the previous backend.
const profileHash = createHash("sha256")
  .update(JSON.stringify({ appEnv, profile, identity: appIdentity(env) }))
  .digest("hex");
config.cacheVersion = `${config.cacheVersion}:${profileHash}`;

module.exports = config;
