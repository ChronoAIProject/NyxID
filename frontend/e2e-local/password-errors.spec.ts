/** Real backend regression. Run with the isolated backend/frontend described in
 * ~/s2-2622-evidence/checks-phase2.md; no network mocks or production account. */
import { test, expect } from "@playwright/test";

const origin = process.env.NYXID_LOCAL_AUTH_ORIGIN ?? "http://127.0.0.1:3122";
const copy = {
  en: "Invalid email or password",
  "zh-CN": "电子邮件或密码不正确",
  "zh-TW": "電子郵件或密碼不正確",
};
for (const language of ["en", "zh-CN", "zh-TW"] as const) {
  for (const surface of ["login", "device", "agent-key"] as const) {
    test(`${surface} exposes the backend password refusal as an accessible ${language} alert`, async ({
      page,
      request,
    }, testInfo) => {
      // Dedicated loopback proxy fixture: each test represents a separate client.
      // Keep the real approval rate limiter enabled. Backend trusts only our loopback proxy.
      const headers = {
        "X-Forwarded-For": `192.0.2.${(testInfo.testId.split("").reduce((n, c) => n + c.charCodeAt(0), 0) % 200) + 1}`,
      };
      await page.setExtraHTTPHeaders(headers);
      await page.addInitScript(
        (language) => localStorage.setItem("nyxid.language", language),
        language,
      );
      let url = "/login";
      if (surface !== "login") {
        const response = await request.post(
          `${origin}/api/v1/auth/${surface === "device" ? "device/v2" : "agent-key"}/request`,
          {
            headers,
            data: { client_label: "2622 local password regression" },
          },
        );
        expect(response.ok()).toBe(true);
        const body = await response.json();
        url = `/login/${surface}?user_code=${encodeURIComponent(body.user_code)}`;
      }
      await page.goto(origin + url);
      await page.locator('input[type="email"]').fill("s2-2622@example.test");
      await page
        .locator('input[type="password"]')
        .fill("DeliberatelyWrong-2622!");
      const refusal = page.waitForResponse(
        (response) =>
          response.request().method() === "POST" &&
          (response.url().endsWith("/auth/login") ||
            /\/auth\/approval\/[^/]+\/password$/.test(response.url())),
      );
      await page
        .locator('form:has(input[type="password"]) button[type="submit"]')
        .click();
      const response = await refusal;
      expect(response.status()).toBe(401);
      const body = await response.json();
      expect(body.error_code).toBe(2000);
      expect(body.message).toBe(
        "Authentication failed: Invalid email or password",
      );
      if (process.env.NYXID_LOCAL_AUTH_EVIDENCE) {
        await page.screenshot({
          path: `${process.env.NYXID_LOCAL_AUTH_EVIDENCE}/${surface}-${language}.png`,
          fullPage: true,
        });
      }
      const alert = page.getByRole("alert").filter({ hasText: copy[language] });
      await expect(alert).toBeVisible();
      await expect(alert).toHaveText(copy[language]);
      await expect(page).toHaveURL(
        new RegExp(surface === "login" ? "/login$" : `/login/${surface}\\?`),
      );
    });
  }
}
