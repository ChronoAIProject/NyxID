import { expect, test, type Page } from "@playwright/test";
import { installSetupFixture } from "./platform-setups-fixtures";
import {
  platformSetup,
  setupBinding,
} from "../src/test/platform-setup-fixture";

const moduleTitle = "Bring a Telegram draft into your account";
const reviewText =
  "I recognize these facts from my private Telegram chat and want this draft in my account.";
const adopt = (page: Page) =>
  page.getByRole("button", { name: "Adopt reviewed draft", exact: true });

async function singleModule(page: Page, draftCount: number) {
  // includeHidden also catches a duplicate concealed by layout/CSS.
  await expect(page.getByRole("main", { includeHidden: true })).toHaveCount(1);
  await expect(
    page.getByRole("heading", {
      name: "Your bot setups",
      exact: true,
      includeHidden: true,
    }),
  ).toHaveCount(1);
  await expect(
    page.getByRole("heading", {
      name: moduleTitle,
      exact: true,
      includeHidden: true,
    }),
  ).toHaveCount(1);
  await expect(
    page.getByRole("combobox", {
      name: "Linked chat",
      exact: true,
      includeHidden: true,
    }),
  ).toHaveCount(1);
  await expect(adopt(page)).toHaveCount(draftCount);
  await expect(page.getByRole("checkbox", { name: reviewText })).toHaveCount(
    draftCount,
  );
}

async function selectChat(page: Page) {
  await page
    .getByRole("combobox", { name: "Linked chat", exact: true })
    .selectOption(setupBinding.id);
}

for (const width of [1440, 390]) {
  test(`one page and import module across direct load, navigation and history at ${width}px`, async ({
    page,
  }, info) => {
    await page.setViewportSize({ width, height: 900 });
    const fixture = await installSetupFixture(page);
    fixture.drafts.push(
      platformSetup({
        id: "55555555-5555-4555-8555-555555555555",
        facts: { ...platformSetup().facts, name: "Second Telegram draft" },
      }),
    );
    await page.goto("/onboarding/setups");
    await singleModule(page, 0);
    await selectChat(page);
    await singleModule(page, 2);
    await expect(
      page.getByText("Telegram cafe · revision 3", { exact: true }),
    ).toHaveCount(1);
    await expect(
      page.getByText("Second Telegram draft · revision 3", { exact: true }),
    ).toHaveCount(1);
    await page
      .getByRole("heading", { name: moduleTitle })
      .scrollIntoViewIfNeeded();
    await page.screenshot({
      path: info.outputPath(`draft-module-${width}.png`),
    });

    for (let visit = 0; visit < 2; visit++) {
      await page
        .getByRole("link", { name: "Choose a starter", exact: true })
        .click();
      await expect(page).toHaveURL(/\/onboarding$/);
      await expect(
        page.getByRole("heading", { name: moduleTitle }),
      ).toHaveCount(0);
      await page
        .getByRole("link", { name: "Resume your saved setups", exact: true })
        .click();
      await singleModule(page, 0);
      await selectChat(page);
      await singleModule(page, 2);
      await page.goBack();
      await expect(
        page.getByRole("heading", {
          name: "Your own Telegram assistant",
          exact: true,
        }),
      ).toHaveCount(1);
      await page.goForward();
      await singleModule(page, 0);
      await selectChat(page);
      await singleModule(page, 2);
    }
    await page.reload();
    await singleModule(page, 0);
    await selectChat(page);
    await singleModule(page, 2);
    expect(fixture.writes).toEqual([]);
    expect(fixture.unexpected).toEqual([]);
    expect(fixture.pageErrors).toEqual([]);
  });
}

test("one reviewed adoption request stays disabled while pending and moves the draft once", async ({
  page,
}) => {
  const fixture = await installSetupFixture(page);
  let release!: () => void;
  fixture.adoptionGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await page.goto("/onboarding/setups?template=telegram-assistant");
  await selectChat(page);
  await singleModule(page, 1);
  await expect(
    page.getByRole("button", { name: "Save draft", exact: true }),
  ).toBeDisabled();
  await expect(adopt(page)).toBeDisabled();
  await page.getByRole("checkbox", { name: reviewText }).check();
  try {
    await adopt(page).dblclick();
    await expect(adopt(page)).toBeDisabled();
    await expect.poll(() => fixture.writes.length).toBe(1);
    expect(fixture.writes).toEqual([
      {
        path: `/onboarding/setups/${platformSetup().id}/adopt`,
        method: "POST",
        body: {
          revision: 3,
          binding_id: setupBinding.id,
          binding_generation: 7,
        },
      },
    ]);
  } finally {
    release();
  }
  await expect(
    page.getByText("No unowned drafts remain for this chat.", { exact: true }),
  ).toBeVisible();
  await singleModule(page, 0);
  await expect(
    page.getByRole("heading", { name: "Telegram cafe", exact: true }),
  ).toHaveCount(1);
  await page.reload();
  await selectChat(page);
  await singleModule(page, 0);
  expect(fixture.writes).toHaveLength(1);
  expect(fixture.unexpected).toEqual([]);
  expect(fixture.pageErrors).toEqual([]);
});

test("revoking the selected linked chat clears its draft review", async ({
  page,
}) => {
  const fixture = await installSetupFixture(page);
  await page.goto("/onboarding/setups");
  await selectChat(page);
  await page.getByRole("checkbox", { name: reviewText }).check();
  await expect(adopt(page)).toBeEnabled();
  await page
    .getByRole("button", { name: "Review revocation", exact: true })
    .click();
  await page
    .getByRole("button", { name: "Revoke binding", exact: true })
    .click();
  await expect(
    page.getByText(
      "No active onboarding bindings. Use /help in the manager’s private chat to start onboarding.",
      { exact: true },
    ),
  ).toBeVisible();
  await expect(
    page.getByRole("combobox", { name: "Linked chat", exact: true }),
  ).toHaveValue("");
  await singleModule(page, 0);
  expect(fixture.writes).toEqual([
    {
      path: `/onboarding/bindings/${setupBinding.id}/revoke`,
      method: "POST",
      body: { generation: setupBinding.generation },
    },
  ]);
  expect(fixture.unexpected).toEqual([]);
  expect(fixture.pageErrors).toEqual([]);
});

for (const key of ["Enter", "Space"] as const) {
  test(`rapid ${key} activation adopts the reviewed draft once`, async ({
    page,
  }) => {
    const fixture = await installSetupFixture(page);
    let release!: () => void;
    fixture.adoptionGate = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.goto("/onboarding/setups");
    await selectChat(page);
    await page.getByRole("checkbox", { name: reviewText }).check();
    try {
      await adopt(page).focus();
      await page.keyboard.press(key);
      await page.keyboard.press(key);
      await expect.poll(() => fixture.writes.length).toBe(1);
      await expect(adopt(page)).toBeDisabled();
    } finally {
      release();
    }
    await expect(
      page.getByText("No unowned drafts remain for this chat.", {
        exact: true,
      }),
    ).toBeVisible();
    await singleModule(page, 0);
    expect(fixture.writes).toHaveLength(1);
    expect(fixture.unexpected).toEqual([]);
    expect(fixture.pageErrors).toEqual([]);
  });
}

for (const failure of ["invalidDraft", "draftError", "adoptError"] as const) {
  test(`${failure} preserves one module and the existing error boundary`, async ({
    page,
  }) => {
    const fixture = await installSetupFixture(page);
    fixture[failure] = true;
    await page.goto("/onboarding/setups");
    await selectChat(page);
    if (failure === "adoptError") {
      await singleModule(page, 1);
      await page.getByRole("checkbox", { name: reviewText }).check();
      await adopt(page).dblclick();
      await expect(
        page.getByText("Draft changed. Review it again.", { exact: true }),
      ).toBeVisible();
      await singleModule(page, 1);
      expect(fixture.writes).toHaveLength(1);
      expect(fixture.saved).toHaveLength(0);
      fixture.adoptError = false;
      await adopt(page).click();
      await expect(
        page.getByText("No unowned drafts remain for this chat.", {
          exact: true,
        }),
      ).toBeVisible();
      await singleModule(page, 0);
      expect(fixture.writes).toHaveLength(2);
      expect(fixture.writes[1]).toEqual(fixture.writes[0]);
    } else {
      await expect(
        page.getByText(
          "Could not load drafts for this chat. Refresh your binding before trying again.",
          { exact: true },
        ),
      ).toBeVisible();
      await singleModule(page, 0);
      expect(fixture.writes).toEqual([]);
    }
    expect(fixture.unexpected).toEqual([]);
    expect(fixture.pageErrors).toEqual([]);
  });
}
