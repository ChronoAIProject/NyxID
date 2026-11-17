import { test } from "@playwright/test";
import { accountBfcacheCases } from "./account-isolation.cases";

// Chromium's headless shell disables bfcache at the embedder level. Use the
// full browser and remove Playwright's bfcache flag only for these cases.
test.use({
  browserName: "chromium",
  channel: "chromium",
  launchOptions: { ignoreDefaultArgs: ["--disable-back-forward-cache"] },
});
accountBfcacheCases();
