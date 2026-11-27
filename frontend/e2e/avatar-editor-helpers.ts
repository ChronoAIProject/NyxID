import { expect, type Page } from "@playwright/test";

/** Open the real assistant drawer against the ordinary HTTP fixture boundary. */
export async function openAvatarEditor(page: Page) {
  for (const [path, json] of [
    ["**/api/v1/assistant/nyxagent/cma-sessions?**", { sessions: [] }],
    ["**/api/v1/assistant/nyxagent/machines?**", []],
    ["**/api/v1/triggers", { triggers: [] }],
    [
      "**/api/v1/service-preferences",
      { groups: [], version: 0, updated_at: null },
    ],
  ] as const) {
    await page.route(path, (route) => route.fulfill({ json }));
  }
  await page.goto("/assistant?mock=1&nyxbot=1&avatars=1");
  await expect(
    page.getByRole("button", { name: "Create new agent", exact: true }),
  ).toBeVisible();
  const thread = await page.evaluate(() => {
    const state = JSON.parse(
      sessionStorage.getItem("nyxagent-http-fixture-v4")!,
    );
    const nyx = state.agents.find(
      (agent: { kind: string }) => agent.kind === "nyxbot",
    );
    return state.rows.find(
      ([, row]: [string, { agentId: string }]) => row.agentId === nyx.id,
    )[0] as string;
  });
  await page.goto(
    `/assistant?mock=1&nyxbot=1&avatars=1&c=${thread}&details=overview`,
  );
  await page.getByRole("button", { name: "Edit avatar", exact: true }).click();
  const editor = page.getByRole("form", { name: "Avatar", exact: true });
  await expect(editor).toBeVisible();
  return {
    editor,
    avatar: page.locator(".agent-header-avatar [data-avatar-renderer]"),
  };
}
