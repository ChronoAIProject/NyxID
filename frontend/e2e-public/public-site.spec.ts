import fs from "node:fs";
const publication = JSON.parse(
  fs.readFileSync("dist/public-release.json", "utf8"),
);
import { test, expect } from "@playwright/test";
test("initial body remains readable with JavaScript disabled", async ({
  browser,
}) => {
  const context = await browser.newContext({ javaScriptEnabled: false });
  const page = await context.newPage();
  for (const path of [
    "/",
    "/zh",
    "/security",
    "/zh/solutions/help-desk",
    "/docs/ai/guides/agent-isolation",
  ]) {
    await page.goto(`http://127.0.0.1:4188${path}`);
    await expect(page.locator("h1")).toBeVisible();
    await expect(page.locator("main")).not.toBeEmpty();
  }
  await context.close();
});
test("hydrates, keeps shell during a delayed route chunk and supports history and locale URLs", async ({
  page,
}) => {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  await page.goto("/?code=INV&return_to=%2Fkeys&utm_source=test");
  expect(
    await page.evaluate(
      () => matchMedia("(prefers-reduced-motion: reduce)").matches,
    ),
  ).toBe(true);
  await expect(page.locator("nav")).toBeVisible();
  await page.waitForFunction(() =>
    document.querySelector('nav a[href^="/security"]'),
  );
  await expect(page.locator("html")).toHaveAttribute("data-public-ready", "v1");
  await expect(page.locator('nav a[href^="/register"]')).toHaveAttribute(
    "href",
    /code=INV/,
  );
  const nav = await page.locator("#nav").elementHandle();
  let release!: () => void;
  const held = new Promise<void>((resolve) => {
    release = resolve;
  });
  let delayed = false;
  await page.route(/\/assets\/SecurityPage-[^/]+\.js/, async (route) => {
    delayed = true;
    await held;
    await route.continue();
  });
  await page.locator('nav a[href^="/security"]').click({ noWaitAfter: true });
  await page.waitForTimeout(250);
  await expect(page.locator("#nav")).toBeVisible();
  await expect(page.locator("#foot")).toBeAttached();
  expect(await nav!.evaluate((el) => el.isConnected)).toBe(true);
  release();
  await expect(page).toHaveURL(/\/security\?/);
  await expect(page.locator("h1")).toContainText("passwords");
  expect(delayed).toBe(true);
  await page.getByRole("link", { name: "中文", exact: true }).click();
  await expect(page).toHaveURL(/\/zh\/security\?/);
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  expect(new URL(page.url()).searchParams.get("return_to")).toBe("/keys");
  await page.goBack();
  await expect(page).toHaveURL(/\/security\?/);
  await page.goForward();
  await expect(page).toHaveURL(/\/zh\/security\?/);
  expect(errors.filter((e) => !/401|Failed to load resource/.test(e))).toEqual(
    [],
  );
});
test("contact preselection and configured solution gate survive hydration", async ({
  page,
}) => {
  await page.goto("/zh/contact?topic=bundle&return_to=%2Fkeys");
  await expect(page.locator("#cTopic")).toHaveValue("bundle");
  await expect(
    page.locator("#cForm input,#cForm select,#cForm textarea"),
  ).toHaveCount(5);
  await expect(page.locator('[role="log"]')).toHaveCount(
    publication.visibility.contactChat ? 1 : 0,
  );
  await page.goto("/solutions/help-desk");
  if (publication.configuration.solutions["help-desk"]) {
    const link = page.locator('[data-solution-activation="help-desk"]');
    await expect(link).toHaveAttribute("href", /^\/login\?/);
    const destination = new URL((await link.getAttribute("href"))!, page.url());
    expect(destination.pathname).toBe("/login");
    expect(destination.searchParams.get("return_to")).toBe(
      publication.configuration.solutions["help-desk"],
    );
  } else {
    await expect(
      page.getByRole("button", { name: "Use this solution", exact: true }),
    ).toBeDisabled();
    await expect(
      page.getByText("Not available yet", { exact: true }),
    ).toBeVisible();
  }
});

// Registered only in the explicit single-solution CI fixture, never skipped to
// manufacture default-off evidence. This proves wiring, not a live solution task.
if (publication.configuration.solutions["help-desk"]) {
  test("single accepted help-desk fixture preserves invitations through all entry routes", async ({
    page,
  }) => {
    const target = publication.configuration.solutions["help-desk"];
    const query = "code=NYX-ABCDEF12&return_to=https%3A%2F%2Fevil.test";
    for (const entry of ["cta", "fragment", "start"]) {
      if (entry === "cta") {
        await page.goto(`/solutions/help-desk?${query}`);
        await expect(page.locator("html")).toHaveAttribute(
          "data-public-ready",
          "v1",
        );
        await page.locator('[data-solution-activation="help-desk"]').click();
      } else {
        await page.goto(
          entry === "start"
            ? `/start?t=help-desk&${query}`
            : `/?${query}#/start?t=help-desk`,
        );
      }
      await expect(page).toHaveURL((url) => url.pathname === "/login");
      let params = new URL(page.url()).searchParams;
      expect(params.get("code")).toBe("NYX-ABCDEF12");
      expect(params.get("return_to")).toBe(target);
      await page.getByRole("button", { name: "Sign up", exact: true }).click();
      await expect(page).toHaveURL((url) => url.pathname === "/register");
      await expect(page.getByPlaceholder("NYX-XXXXXXXX")).toHaveValue(
        "NYX-ABCDEF12",
      );
      params = new URL(page.url()).searchParams;
      expect(params.get("code")).toBe("NYX-ABCDEF12");
      expect(params.get("return_to")).toBe(target);
    }
  });
}

test("records a bounded lab resource sample", async ({ page }) => {
  await page.addInitScript(() => {
    const stats = { lcp: null as number | null, cls: 0 };
    (window as unknown as { publicLab: typeof stats }).publicLab = stats;
    if (
      PerformanceObserver.supportedEntryTypes.includes(
        "largest-contentful-paint",
      )
    ) {
      new PerformanceObserver((list) => {
        stats.lcp = list.getEntries().at(-1)?.startTime ?? null;
      }).observe({ type: "largest-contentful-paint", buffered: true });
    }
    if (PerformanceObserver.supportedEntryTypes.includes("layout-shift")) {
      new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) {
          const shift = entry as PerformanceEntry & {
            hadRecentInput: boolean;
            value: number;
          };
          if (!shift.hadRecentInput) stats.cls += shift.value;
        }
      }).observe({ type: "layout-shift", buffered: true });
    }
  });
  await page.goto("/security");
  await expect(page.locator("html")).toHaveAttribute("data-public-ready", "v1");
  const sample = await page.evaluate(() => {
    const resources = performance.getEntriesByType(
      "resource",
    ) as PerformanceResourceTiming[];
    return {
      kind: "lab",
      path: "/security",
      reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches,
      fieldCwvStatus: "not-measured",
      resources: resources.length,
      transferBytes: resources.reduce((sum, r) => sum + r.transferSize, 0),
      decodedBytes: resources.reduce((sum, r) => sum + r.decodedBodySize, 0),
      ...(
        window as unknown as { publicLab: { lcp: number | null; cls: number } }
      ).publicLab,
      limitations:
        "Single CI browser at takeover; cross-origin transfer sizes may be unavailable. Not field CWV or an LCP/CLS pass assertion.",
    };
  });
  const { writeFile } = await import("node:fs/promises");
  await writeFile("dist/public-lab.json", JSON.stringify(sample, null, 2));
});

test("retains authenticated home redirects and Docs controls", async ({
  page,
  context,
}) => {
  await context.addCookies([
    {
      name: "fixture-auth",
      value: "1",
      url: "http://127.0.0.1:4188",
      httpOnly: true,
    },
  ]);
  for (const path of ["/", "/zh"]) {
    await page.goto(path);
    await expect(page).toHaveURL(/\/dashboard$/);
  }
  await page.goto("/docs/ai/guides/agent-isolation");
  await expect(
    page.getByRole("link", { name: "Dashboard", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Switch to (light|dark) mode/ }),
  ).toBeVisible();
  await expect(page.locator('aside a[href^="#"]')).not.toHaveCount(0);
});

test("F01 contact remains inert without entry JavaScript and never submits personal data to the origin", async ({
  browser,
  page,
}) => {
  for (const mode of ["disabled", "failed", "delayed"] as const) {
    const context = await browser.newContext({
      javaScriptEnabled: mode !== "disabled",
      reducedMotion: "reduce",
    });
    const target = await context.newPage();
    let release = () => {};
    if (mode !== "disabled") {
      const held = new Promise<void>((resolve) => {
        release = resolve;
      });
      await target.route(/\/assets\/index-[^/]+\.js/, async (route) => {
        if (mode === "delayed") await held;
        await route.abort();
      });
    }
    const submissions: string[] = [];
    target.on("request", (request) => {
      if (
        request.url().includes("PERSONAL_SENTINEL") ||
        request.postData()?.includes("PERSONAL_SENTINEL")
      )
        submissions.push(request.url());
    });
    try {
      await target.goto("http://127.0.0.1:4188/contact", {
        waitUntil: "commit",
      });
      await expect(target.locator("#cForm fieldset")).toHaveAttribute(
        "disabled",
        "",
      );
      await expect(
        target.locator('#cForm button[type="submit"]'),
      ).toBeDisabled();
      await target.locator("#cName").evaluate((input: HTMLInputElement) => {
        input.value = "PERSONAL_SENTINEL";
      });
      for (const control of await target
        .locator("#cForm input,#cForm select,#cForm textarea")
        .all())
        await expect(control).toBeDisabled();
      await expect(target.locator("#cForm")).toHaveAttribute(
        "action",
        "mailto:contact@chrono-ai.fun",
      );
      await target.keyboard.press("Enter");
      await expect(
        target.locator('a[href="mailto:contact@chrono-ai.fun"]').first(),
      ).toBeVisible();
      expect(submissions).toEqual([]);
      expect(new URL(target.url()).search).toBe("");
    } finally {
      release();
      await context.close();
    }
  }
  await page.goto("/contact");
  await expect(page.locator('#cForm button[type="submit"]')).toBeEnabled();
  await page.locator("#cName").fill("PERSONAL_SENTINEL");
  await page.locator("#cEmail").fill("personal@example.test");
  await page.locator("#cTopic").selectOption("bundle");
  await page.locator("#cMsg").fill("A message to test the email handoff.");
  const leaked: string[] = [];
  page.on("request", (request) => {
    if (
      request.url().startsWith("http://127.0.0.1:4188") &&
      (request.url().includes("PERSONAL_SENTINEL") ||
        request.postData()?.includes("PERSONAL_SENTINEL"))
    )
      leaked.push(request.url());
  });
  await page.locator("#cName").press("Enter");
  await expect(page.getByRole("status")).toContainText("Email app requested");
  expect(leaked).toEqual([]);
  expect(new URL(page.url()).search).toBe("");
});

test("F02 and F04 preserve ordered query strings, document identity and mobile menu behavior", async ({
  page,
}) => {
  await page.setViewportSize({ width: 700, height: 900 });
  const query =
    "?code=123&flag=true&nil=null&json=%7B%22a%22%3A1%7D&tag=first&x=middle&tag=second&encoded=a%2Bb%20c&return_to=%2Fkeys%3Ftab%3Dactive";
  const entries = Array.from(new URLSearchParams(query));
  await page.goto("/" + query);
  await expect(page.locator("html")).toHaveAttribute("data-public-ready", "v1");
  await page.evaluate(() => {
    (window as unknown as { identity: string }).identity = "same-document";
  });
  for (const selector of [
    'nav a[data-nav="security"]',
    'nav a[data-nav="solutions"]',
    'nav a[data-lang="zh"]',
  ]) {
    await page.locator("#burger").click();
    await expect(page.locator("#burger")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    const link = page.locator(selector);
    expect(
      Array.from(
        new URL((await link.getAttribute("href"))!, page.url()).searchParams,
      ),
    ).toEqual(entries);
    await link.click();
    await expect(page.locator("#burger")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(Array.from(new URL(page.url()).searchParams)).toEqual(entries);
    expect(
      await page.evaluate(
        () => (window as unknown as { identity: string }).identity,
      ),
    ).toBe("same-document");
  }
  for (const direction of ["goBack", "goForward"] as const) {
    await page.locator("#burger").click();
    await page[direction]();
    await expect(page.locator("#burger")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(Array.from(new URL(page.url()).searchParams)).toEqual(entries);
  }
  await page.locator("#burger").click();
  await page
    .locator('nav a[data-nav="security"]')
    .click({ modifiers: ["Control"] });
  await expect(page.locator("#burger")).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  const login = page.locator('nav a[href^="/login"]').first();
  expect(
    Array.from(
      new URL((await login.getAttribute("href"))!, page.url()).searchParams,
    ),
  ).toEqual(entries);
});

test("F03 retries a failed Docs fetch and recovers a failed route import by explicit reload", async ({
  page,
}) => {
  let recovered = false,
    reads = 0;
  await page.route("**/docs/ai/guides/agent-isolation.md", async (route) => {
    reads++;
    if (!recovered)
      await route.fulfill({ status: 503, body: "Temporarily unavailable" });
    else await route.continue();
  });
  await page.goto("/docs/ai/getting-started/what-is-ai-access");
  await expect(page.locator("html")).toHaveAttribute("data-public-ready", "v1");
  await page
    .locator('a[href="/docs/ai/guides/agent-isolation"]')
    .first()
    .click();
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toBeVisible();
  await expect(page.locator("header")).toBeVisible();
  const before = reads;
  recovered = true;
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(page.locator("h1")).toHaveText(
    "Isolate agents with scoped keys",
  );
  expect(reads).toBe(before + 1);
  let imports = 0,
    allow = false;
  await page.route(/\/assets\/SecurityPage-[^/]+\.js/, async (route) => {
    imports++;
    if (allow) await route.continue();
    else await route.abort();
  });
  await page.goto("/security");
  await expect(
    page.getByRole("button", { name: "Retry", exact: true }),
  ).toBeVisible();
  const rejected = imports;
  await page.evaluate(() => {
    (window as unknown as { retryDocument: string }).retryDocument =
      "failed-import-document";
  });
  allow = true;
  const reloaded = page.waitForEvent("domcontentloaded");
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await reloaded;
  await expect(page.locator("html")).toHaveAttribute("data-public-ready", "v1");
  await expect(page.locator("h1")).toContainText("passwords");
  expect(
    await page.evaluate(
      () => (window as unknown as { retryDocument?: string }).retryDocument,
    ),
  ).toBeUndefined();
  expect(imports).toBeGreaterThan(rejected);
});

test("published visibility matches the release and makes no feature POST requests on view", async ({
  page,
}) => {
  const writes: string[] = [];
  page.on("request", (request) => {
    if (
      request.method() === "POST" &&
      new URL(request.url()).origin === "http://127.0.0.1:4188"
    )
      writes.push(request.url());
  });
  for (const path of [
    "/blog",
    "/zh/blog",
    "/contact",
    "/zh/contact",
    "/security",
  ]) {
    await page.goto(path);
    await expect(page.locator("html")).toHaveAttribute(
      "data-public-ready",
      "v1",
    );
    expect(
      JSON.parse(
        (await page
          .locator("[data-website-visibility]")
          .getAttribute("data-website-visibility"))!,
      ),
    ).toEqual(publication.visibility);
    for (const [name, enabled] of [
      [
        "newsletter",
        publication.visibility.newsletter && path.endsWith("blog"),
      ],
      [
        "contact-chat",
        publication.visibility.contactChat && path.endsWith("contact"),
      ],
      ["footer-status", publication.visibility.footerStatus],
    ] as const)
      await expect(
        page.locator(`[data-website-feature="${name}"]`),
      ).toHaveCount(enabled ? 1 : 0);
    await expect(page.locator('nav [data-nav="blog"]')).toHaveCount(
      publication.visibility.blogMenu ? 1 : 0,
    );
    if (path.includes("contact")) {
      await expect(page.locator("#cForm")).toBeVisible();
      await expect(
        page.locator('a[href="mailto:contact@chrono-ai.fun"]').first(),
      ).toBeVisible();
    }
  }
  expect(writes).toEqual([]);
});

test("Blog menu visibility applies to mobile and desktop without changing direct access", async ({
  page,
}) => {
  for (const width of [700, 1280]) {
    await page.setViewportSize({ width, height: 900 });
    for (const path of ["/blog", "/zh/blog"]) {
      await page.goto(path);
      await expect(page.locator("html")).toHaveAttribute(
        "data-public-ready",
        "v1",
      );
      await expect(page.locator("h1")).toBeVisible();
      if (width === 700) await page.locator("#burger").click();
      await expect(page.locator('nav [data-nav="blog"]')).toHaveCount(
        publication.visibility.blogMenu ? 1 : 0,
      );
      await expect(page.locator(`footer a[href="${path}"]`)).toHaveCount(1);
    }
  }
});
