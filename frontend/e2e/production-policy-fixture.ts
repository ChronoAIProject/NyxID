import { test as base, expect } from "@playwright/test";
import fs from "node:fs";

export const test = base.extend<{ productionPolicy: void }>({
  productionPolicy: [async ({ context }, use, testInfo) => {
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
    await testInfo.attach("production-csp-violations", { body: JSON.stringify(violations, null, 2), contentType: "application/json" });
    const allowed = testInfo.annotations.filter((item) => item.type === "expected-csp").map((item) => JSON.parse(item.description!));
    expect(violations.filter((value) => !allowed.some((item) => {
      const violation = value as { directive: string; blocked: string };
      return item.directive === violation.directive && item.blocked === violation.blocked;
    }))).toEqual([]);
    for (const item of allowed) expect(violations).toContainEqual(expect.objectContaining(item));
  }, { auto: true }],
});
