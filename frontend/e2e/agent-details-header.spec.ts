import { expect, test } from "@playwright/test";

test("agent details collapses to the production header on scroll without a divider", async ({
  page,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/assistant?mock=1&nyxbot=1&avatars=1");
  await expect(
    page.getByRole("button", { name: "Create new agent", exact: true }),
  ).toBeVisible();
  await page
    .getByRole("button", { name: "researcher, specialist, Idle", exact: true })
    .hover();
  await page
    .getByRole("button", { name: "Options for researcher", exact: true })
    .click();
  await page
    .getByRole("menuitem", { name: "Agent details", exact: true })
    .click();
  const sheet = page.getByRole("dialog", {
    name: "Agent details",
    exact: true,
  });
  const header = sheet.locator("[data-agent-header]");
  const avatar = header.locator(".agent-header-avatar");
  const scroll = sheet.locator('[data-panel="overview"] [data-panel-scroll]');
  await expect(header).toHaveAttribute("data-compact", "false");
  await expect.poll(async () => (await avatar.boundingBox())?.width).toBe(112);
  const expanded = (await header.boundingBox())!.height;
  expect(
    await header.evaluate((e) => getComputedStyle(e).borderBottomWidth),
  ).toBe("0px");
  await scroll.evaluate((e) => {
    e.scrollTop = 400;
  });
  await expect(header).toHaveAttribute("data-compact", "true");
  await expect.poll(async () => (await avatar.boundingBox())?.width).toBe(32);
  expect((await header.boundingBox())!.height).toBeLessThan(expanded - 65);
  await expect(
    header.getByText(/Specialist · Idle · 2 memories/),
  ).toBeVisible();
  await expect(
    header.getByText("Created by NyxBot", { exact: true }),
  ).toBeHidden();
  expect(await scroll.evaluate((e) => e.scrollTop)).toBe(400);
  for (const width of [390, 320]) {
    await page.setViewportSize({ width, height: 844 });
    await expect(avatar).toBeInViewport();
    await expect(
      sheet.getByRole("button", { name: "Close agent details", exact: true }),
    ).toBeInViewport();
    expect(
      await sheet.evaluate((e) => e.scrollWidth - e.clientWidth),
    ).toBeLessThanOrEqual(1);
  }
  await scroll.evaluate((e) => {
    e.scrollTop = 0;
  });
  await expect(header).toHaveAttribute("data-compact", "false");
  await expect.poll(async () => (await avatar.boundingBox())?.width).toBe(112);
  await expect(
    header.getByText("Created by NyxBot", { exact: true }),
  ).toBeVisible();
  expect(errors).toEqual([]);
});
