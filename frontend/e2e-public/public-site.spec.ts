import fs from "node:fs";
const publication = JSON.parse(
  fs.readFileSync("dist/public-release.json", "utf8"),
);
import { test, expect } from "@playwright/test";
const testOrigin = process.env.PUBLIC_TEST_ORIGIN || "http://127.0.0.1:4188";
test("initial body remains readable with JavaScript disabled", async ({
  browser,
}) => {
  const context = await browser.newContext({ javaScriptEnabled: false });
  const page = await context.newPage();
  for (const path of [
    "/",
    "/zh",
    "/security",
    "/integrations",
    "/zh/integrations",
    "/zh/solutions/help-desk",
    "/docs/ai/guides/agent-isolation",
  ]) {
    await page.goto(`${testOrigin}${path}`);
    await expect(page.locator("h1")).toBeVisible();
    await expect(page.locator("main")).not.toBeEmpty();
    if (path === "/" || path === "/zh") {
      await expect(page.locator(".nyx-home #top h1")).toBeVisible();
      for (const id of ["pile", "agent", "apps", "jobs", "end"])
        await expect(page.locator(`.nyx-home section#${id}`)).toHaveCount(1);
      await expect(page.locator("#nav .public-language")).toHaveCount(1);
    }
    if (path.endsWith("/integrations")) {
      await expect(page.locator(".stc-map")).toBeVisible();
      await expect(page.locator(".stc-map-tile")).toHaveCount(13);
      await expect(page.locator(".stc-map-tile small")).toHaveText(path.startsWith("/zh") ? "即将上线" : "Coming soon");
    }
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
  await expect(page.locator("#nav")).toBeVisible();
  await page.waitForFunction(() =>
    document.querySelector('#nav .public-more a[href^="/security"]'),
  );
  await expect(page.locator("html")).toHaveAttribute("data-public-ready", "v1");
  await expect(page.locator('#nav a[href^="/register"]')).toHaveAttribute(
    "href",
    /code=INV/,
  );
  await page.evaluate(() => window.scrollTo(0, 1200));
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
  await page.locator("#nav .public-more summary").click();
  await page.locator('#nav .public-more a[href^="/security"]').click({ noWaitAfter: true });
  await page.waitForTimeout(250);
  await expect(page.locator("#nav")).toBeVisible();
  await expect(page.locator("#foot")).toBeAttached();
  expect(await nav!.evaluate((el) => el.isConnected)).toBe(true);
  release();
  await expect(page).toHaveURL(/\/security\?/);
  await expect(page.locator("h1")).toContainText("credentials");
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  expect(delayed).toBe(true);
  await page.locator("#foot").getByRole("link", { name: "中文", exact: true }).click();
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
test("header language switching keeps the route and queries, and new pages start at the top", async ({ page }) => {
  await page.goto("/integrations?code=INV&tag=one&tag=two&return_to=%2Fkeys");
  await expect(page.locator("html")).toHaveAttribute("data-public-ready", "v1");
  await page.evaluate(() => window.scrollTo(0, 1100));
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(1100);
  await page.locator('#nav .public-nav-links a[href^="/solutions"]').click();
  await expect(page.locator("h1")).toHaveText("Jobs you can hand to an assistant");
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  await page.goBack();
  await expect(page.locator("h1")).toHaveText("Find the tools your agent can use");
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(1100);

  const language = page.locator("#nav .public-language");
  await language.locator("summary").click();
  await page.keyboard.press("Escape");
  await expect(language).not.toHaveAttribute("open", "");
  await expect(language.locator("summary")).toBeFocused();
  await language.locator("summary").click();
  await language.getByRole("link", { name: "中文", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  expect(new URL(page.url()).pathname).toBe("/zh/integrations");
  expect(new URL(page.url()).searchParams.getAll("tag")).toEqual(["one", "two"]);
  expect(new URL(page.url()).searchParams.get("code")).toBe("INV");
  expect(new URL(page.url()).searchParams.get("return_to")).toBe("/keys");
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  await expect(language).not.toHaveAttribute("open", "");

  // A real section link still lands at its target on the homepage.
  await page.goto("/#apps");
  await expect(page.locator("#apps")).toBeInViewport();
});
test("Chinese home keeps the approved homepage design and localized interactions", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/?code=INV&tag=one&tag=two");
  await expect(page.locator("html")).toHaveAttribute("data-public-ready", "v1");
  const sections = await page.locator(".nyx-home main > section").evaluateAll(nodes => nodes.map(node => node.id));
  await page.locator(".public-language summary").click();
  await page.locator(".public-language").getByRole("link", { name: "中文", exact: true }).click();
  await expect(page.locator("html")).toHaveAttribute("lang", "zh-CN");
  await expect(page.locator(".nyx-home h1")).toContainText("能办事的 AI 助手");
  expect(await page.locator(".nyx-home main > section").evaluateAll(nodes => nodes.map(node => node.id))).toEqual(sections);
  await expect(page.locator("#nav")).toHaveCount(1);
  await expect(page.locator("#foot")).toHaveCount(1);
  await expect(page.locator(".nyx-home canvas[data-nyx]")).toHaveCount(1);
  await expect(page.locator(".unicorn-embed")).toHaveCount(0);
  await expect(page.locator(".public-cta")).toHaveAttribute("href", /code=INV/);
  await page.locator(".tool-catalog summary").click();
  await page.getByRole("searchbox", { name: "搜索工具", exact: true }).fill("WhatsApp");
  await expect(page.locator(".catalog-tile:visible")).toHaveCount(1);
  await expect(page.locator(".catalog-tile:visible")).toContainText("即将上线");
  await expect(page.locator(".tool-catalog [role=status]")).toHaveText("找到 1 个工具");
  await page.locator(".public-language summary").click();
  await page.locator(".public-language").getByRole("link", { name: "English", exact: true }).click();
  await expect(page.locator(".nyx-home h1")).toContainText("The AI assistant");
  await expect.poll(() => page.evaluate(() => window.scrollY)).toBe(0);
  expect(new URL(page.url()).searchParams.getAll("tag")).toEqual(["one", "two"]);
  expect(errors).toEqual([]);
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
    await expect(page.locator(".st-availability")).toContainText("Not available yet");
    await expect(page.getByRole("link", { name: "Talk it through", exact: true })).toBeVisible();
    await page.getByText("Setup availability", { exact: true }).click();
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
      url: testOrigin,
      httpOnly: true,
    },
  ]);
  for (const path of ["/", "/zh"]) {
    await page.goto(path);
    await expect(page).toHaveURL(/\/assistant$/);
  }
  await page.goto("/docs/ai/guides/agent-isolation");
  await expect(
    page.getByRole("link", { name: "Dashboard", exact: true }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: /Switch to (day|night) mode/ }),
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
      await target.goto(`${testOrigin}/contact`, {
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
      request.url().startsWith(testOrigin) &&
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
  const menu = page.locator("#nav .public-more");
  for (const [disclosure, selector] of [
    [".public-more", 'a[href^="/security"]'],
    [".public-more", 'a[href^="/solutions"]'],
    [".public-language", 'a[lang="zh-CN"]'],
  ]) {
    const activeMenu = page.locator(`#nav ${disclosure}`);
    await activeMenu.locator("summary").click();
    await expect(activeMenu).toHaveAttribute("open", "");
    const link = activeMenu.locator(selector);
    expect(Array.from(new URL((await link.getAttribute("href"))!, page.url()).searchParams)).toEqual(entries);
    await link.click();
    await expect(menu).not.toHaveAttribute("open", "");
    await expect(page.locator("#nav .public-language")).not.toHaveAttribute("open", "");
    expect(Array.from(new URL(page.url()).searchParams)).toEqual(entries);
    expect(await page.evaluate(() => (window as unknown as { identity: string }).identity)).toBe("same-document");
  }
  for (const direction of ["goBack", "goForward"] as const) {
    await menu.locator("summary").click();
    await page[direction]();
    await expect(menu).not.toHaveAttribute("open", "");
    expect(Array.from(new URL(page.url()).searchParams)).toEqual(entries);
  }
  await menu.locator("summary").click();
  await menu.locator('a[href*="/security"]').click({ modifiers: ["Control"] });
  await expect(menu).toHaveAttribute("open", "");
  const login = menu.locator('a[href^="/login"]');
  expect(Array.from(new URL((await login.getAttribute("href"))!, page.url()).searchParams)).toEqual(entries);
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
  await expect(page.locator("h1")).toContainText("credentials");
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
      new URL(request.url()).origin === testOrigin
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
      await page.locator("#nav .public-more summary").click();
      await expect(page.locator('nav [data-nav="blog"]')).toHaveCount(
        publication.visibility.blogMenu ? 1 : 0,
      );
      await expect(page.locator(`footer a[href="${path}"]`)).toHaveCount(1);
    }
  }
});
