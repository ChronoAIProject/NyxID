import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const configPath = require.resolve("../../metro.config.js");

test("Metro invalidates compiled environment values when the selected production domains change", () => {
  const fixture = {
    APP_ENV: "prod",
    PROD_USE_NEW_DOMAINS: "false",
    PROD_API_BASE_URL: "https://old-api.example.test/api/v1",
    PROD_FRONTEND_URL: "https://old.example.test",
    PROD_LEGAL_BASE_URL: "https://old.example.test",
    PROD_NEW_API_BASE_URL: "https://api.example.test/api/v1",
    PROD_NEW_FRONTEND_URL: "https://example.test",
    PROD_NEW_LEGAL_BASE_URL: "https://example.test",
    PROD_IOS_BUNDLE_ID: "test.example.app",
    PROD_ANDROID_PACKAGE: "test.example.app",
  };
  const previous = Object.fromEntries(Object.keys(fixture).map((key) => [key, process.env[key]]));
  const cacheVersion = () => {
    delete require.cache[configPath];
    return (require(configPath) as { cacheVersion: string }).cacheVersion;
  };

  try {
    Object.assign(process.env, fixture);
    const legacy = cacheVersion();
    process.env.PROD_USE_NEW_DOMAINS = "true";
    const migrated = cacheVersion();
    assert.notEqual(migrated, legacy);

    process.env.PROD_NEW_API_BASE_URL = "https://replacement-api.example.test/api/v1";
    assert.notEqual(cacheVersion(), migrated);

    process.env.PROD_USE_NEW_DOMAINS = "false";
    assert.equal(cacheVersion(), legacy);
    process.env.PROD_API_BASE_URL = "https://replacement-old-api.example.test/api/v1";
    assert.notEqual(cacheVersion(), legacy);
  } finally {
    delete require.cache[configPath];
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
});
