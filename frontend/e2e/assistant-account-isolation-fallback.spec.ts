import { test } from "@playwright/test";
import { accountIsolationCases } from "./account-isolation.cases";

test.use({ browserName: "chromium" });
test.beforeEach(async ({ context }) => {
  await context.addInitScript(() => {
    Object.defineProperty(AbortSignal, "any", {
      configurable: true,
      value: undefined,
    });
  });
});
accountIsolationCases();
