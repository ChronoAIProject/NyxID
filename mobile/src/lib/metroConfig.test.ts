import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

const require = createRequire(import.meta.url);
const configPath = require.resolve("../../metro.config.js");

test("Metro invalidates compiled environment values when either embedded domain set changes", () => {
  const fixture = {
    APP_ENV: "prod",
    PROD_API_BASE_URL: "https://old-api.example.test/api/v1",
    PROD_FRONTEND_URL: "https://old.example.test",
    PROD_LEGAL_BASE_URL: "https://old.example.test",
    PROD_NEW_API_BASE_URL: "",
    PROD_NEW_FRONTEND_URL: "",
    PROD_NEW_LEGAL_BASE_URL: "",
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
    const legacyOnly = cacheVersion();
    Object.assign(process.env, {
      PROD_NEW_API_BASE_URL: "https://api.example.test/api/v1",
      PROD_NEW_FRONTEND_URL: "https://example.test",
      PROD_NEW_LEGAL_BASE_URL: "https://example.test",
    });
    const both = cacheVersion();
    assert.notEqual(both, legacyOnly);

    process.env.PROD_NEW_API_BASE_URL = "https://replacement-api.example.test/api/v1";
    const replacedNew = cacheVersion();
    assert.notEqual(replacedNew, both);

    process.env.PROD_API_BASE_URL = "https://replacement-old-api.example.test/api/v1";
    assert.notEqual(cacheVersion(), replacedNew);
  } finally {
    delete require.cache[configPath];
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
});
