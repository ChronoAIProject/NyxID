import { test as base, expect } from "@playwright/test";
import fs from "node:fs";

export const test = base.extend<{ productionPolicy: void }>({
  productionPolicy: [async ({ context, page }, use, testInfo) => {
    if (!process.env.NYXID_PRODUCTION_TEST_URL) { await use(); return; }
    const violations: unknown[] = [];
    await context.exposeBinding("__nyxidPolicyViolation", (_source, value) => { violations.push(value); });
    await context.addInitScript(() => {
      document.addEventListener("securitypolicyviolation", (event) => {
        const send = (window as unknown as { __nyxidPolicyViolation: (value: unknown) => Promise<void> }).__nyxidPolicyViolation;
        void send({ directive: event.effectiveDirective, blocked: event.blockedURI, document: location.pathname,
          source: event.sourceFile, line: event.lineNumber });
      });
    });
    const font = process.env.NYXID_TEST_FONT;
    if (!font) throw new Error("NYXID_TEST_FONT must name the recorded real font fixture");
    await context.route("https://fonts.googleapis.com/**", (route) => route.fulfill({
      contentType: "text/css", body: '@font-face{font-family:"Mona Sans";src:url(https://fonts.gstatic.com/nyxid-policy-fixture.ttf) format("truetype");font-weight:100 900;font-display:swap} @font-face{font-family:"JetBrains Mono";src:url(https://fonts.gstatic.com/nyxid-policy-fixture.ttf) format("truetype");font-weight:100 900;font-display:swap}',
    }));
    await context.route("https://fonts.gstatic.com/**", (route) => route.fulfill({
      contentType: "font/ttf", body: fs.readFileSync(font), headers: { "access-control-allow-origin": "*" },
    }));
    await use();
    const allowed = testInfo.annotations.filter((item) => item.type === "expected-csp").map((item) => JSON.parse(item.description!));
    // CSP events and exposeBinding delivery are asynchronous. Depending on page
    // keeps it alive until this auto-fixture has drained the browser task queue;
    // otherwise page teardown can discard a just-queued rejection event.
    await page.evaluate(() => new Promise<void>((resolve) => setTimeout(resolve, 0)));
    for (const item of allowed) {
      await expect.poll(() => violations.some((value) => {
        const violation = value as { directive: string; blocked: string };
        return item.directive === violation.directive && item.blocked === violation.blocked;
      }), { timeout: 5_000, message: `required CSP event ${JSON.stringify(item)}` }).toBe(true);
    }
    await testInfo.attach("production-csp-violations", { body: JSON.stringify(violations, null, 2), contentType: "application/json" });
    expect(violations.filter((value) => !allowed.some((item) => {
      const violation = value as { directive: string; blocked: string };
      return item.directive === violation.directive && item.blocked === violation.blocked;
    }))).toEqual([]);
  }, { auto: true }],
});
