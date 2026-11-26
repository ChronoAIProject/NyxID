import assert from "node:assert/strict";
import test from "node:test";
import {
  nativeWebFallback,
  deduplicatedWebFallback,
} from "./nativeWebFallback";
const hosts = {
  apiBaseUrls: ["https://old-api.example/api/v1", "https://api.example/api/v1"],
  frontendUrls: ["https://old-web.example", "https://web.example"],
  universalLinkHost: "nyxid.onelink.me",
};
test("fallback trusts only embedded HTTPS first-party origins and uses the launch-selected frontend", () => {
  for (const host of ["old-api.example", "api.example"])
    assert.equal(
      nativeWebFallback(
        `https://${host}/developer?tab=apps`,
        hosts,
        "https://web.example",
      ),
      "https://web.example/developer?tab=apps",
    );
  assert.equal(
    nativeWebFallback(
      "https://old-web.example/developer?tab=apps",
      hosts,
      "https://web.example",
    ),
    "https://old-web.example/developer?tab=apps",
  );
  for (const url of [
    "https://evil.example/developer",
    "https://api.example.evil.example/developer",
    "https://user@api.example/developer",
    "http://api.example/developer",
    "nyxid://developer",
    "https://nyxid.onelink.me/REzJ",
  ])
    assert.equal(nativeWebFallback(url, hosts, "https://web.example"), null);
});
test("repeated OS delivery opens a fallback once", async () => {
  const opened: string[] = [];
  const open = deduplicatedWebFallback(async (url) => {
    opened.push(url);
  });
  open("https://web.example/developer");
  open("https://web.example/developer");
  await Promise.resolve();
  assert.deepEqual(opened, ["https://web.example/developer"]);
});
