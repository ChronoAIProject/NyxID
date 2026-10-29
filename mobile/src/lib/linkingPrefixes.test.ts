import assert from "node:assert/strict";
import { createRequire } from "node:module";
import path from "node:path";
import test from "node:test";

import { buildLinkingPrefixes } from "./linkingPrefixes";

const require = createRequire(import.meta.url);
const navigationRoot = path.dirname(require.resolve("@react-navigation/native/package.json"));
// Exercise the installed navigation parser; this internal path may need updating with React Navigation.
const { extractPathFromURL } = require(path.join(navigationRoot, "lib/module/extractPathFromURL.js")) as {
  extractPathFromURL: (prefixes: string[], url: string) => string | undefined;
};

const newDomains = {
  appScheme: "nyxid",
  apiBaseUrl: "https://api.nyxid.ai/api/v1",
  frontendUrl: "https://nyxid.ai",
  universalLinkHost: "nyxid.ai",
  allowHttp: false,
};

test("routes selected frontend, backend and custom-scheme device-login links", () => {
  const prefixes = buildLinkingPrefixes(newDomains);
  for (const origin of ["https://nyxid.ai/", "https://api.nyxid.ai/", "nyxid://"]) {
    assert.equal(
      extractPathFromURL(prefixes, `${origin}login/device?user_code=ABCD-EFGH`),
      "login/device?user_code=ABCD-EFGH",
    );
  }
});

test("does not route old, unrelated, suffix-confused or HTTP production hosts", () => {
  const prefixes = buildLinkingPrefixes(newDomains);
  for (const origin of [
    "https://nyx.chrono-ai.fun/",
    "https://nyx-api.chrono-ai.fun/",
    "https://evil.example/",
    "https://nyxid.ai.evil.example/",
    "https://nyxid.ai@evil.example/",
    "http://nyxid.ai/",
  ]) {
    assert.equal(extractPathFromURL(prefixes, `${origin}login/device?user_code=ABCD-EFGH`), undefined);
  }
});

test("routes an explicit link host and development HTTP origins", () => {
  const prefixes = buildLinkingPrefixes({
    ...newDomains,
    appScheme: "nyxid-dev",
    apiBaseUrl: "http://localhost:3001/api/v1",
    frontendUrl: "http://localhost:3000",
    universalLinkHost: "dev-links.example.com",
    allowHttp: true,
  });
  for (const origin of ["nyxid-dev://", "http://localhost:3000/", "http://localhost:3001/", "https://dev-links.example.com/"]) {
    assert.equal(extractPathFromURL(prefixes, `${origin}login/agent-key`), "login/agent-key");
  }
});

test("ignores empty or invalid optional URL inputs", () => {
  const prefixes = buildLinkingPrefixes({ ...newDomains, apiBaseUrl: "invalid", frontendUrl: "", universalLinkHost: "" });
  assert.equal(extractPathFromURL(prefixes, "nyxid://login/device"), "login/device");
  assert.equal(extractPathFromURL(prefixes, "https://nyxid.ai/login/device"), undefined);
});
