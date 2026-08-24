import { expect, test, type Page } from "@playwright/test";
import { openAssistant } from "./helpers";

/**
 * Wave-2 service-family journeys. Action-card wiring is a PM merge point, so
 * these specs mount the owned dialogs through the Vite dev graph and drive
 * them the way a user would: type, confirm, wait for evidence, finish.
 */

const SERVICE_ID = "11111111-1111-4111-8111-111111111111";
const EVIDENCE = {
  id: SERVICE_ID,
  api_key_id: "22222222-2222-4222-8222-222222222222",
  is_active: true,
  status: "active",
  connection_status: null,
  granted_scopes: null,
  last_authorized_at: null,
  node_id: null,
  rotation_predecessor_id: null,
  state_version: 1,
  updated_at: "2026-08-20T00:00:00Z",
};

async function stubServiceApis(
  page: Page,
  options: { deleteConflictOnce?: boolean } = {},
): Promise<unknown[]> {
  let deleteCalls = 0;
  let serviceStateVersion = EVIDENCE.state_version;
  let deleted = false;
  const requestBodies: unknown[] = [];
  await page.route("**/api/v1/assistant/actions/services/**", async (route) => {
    const url = route.request().url();
    const method = route.request().method();
    if (method !== "POST") {
      await route.continue();
      return;
    }
    requestBodies.push(route.request().postDataJSON());
    if (url.includes("/delete") && options.deleteConflictOnce && deleteCalls === 0) {
      deleteCalls += 1;
      await route.fulfill({
        status: 409,
        contentType: "application/json",
        body: JSON.stringify({
          error: "grant_cascade_confirmation_required",
          error_code: 11_500,
          message: "Grant cascade confirmation required",
          details: {
            provider_slug: "github",
            provider_name: "GitHub",
            revokes_grant: true,
            siblings: [
              { user_service_id: "sibling", name: "other", slug: "other" },
            ],
          },
        }),
      });
      return;
    }
    serviceStateVersion += 1;
    if (url.includes("/delete")) deleted = true;
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        resource: { userServiceId: SERVICE_ID },
        replayed: false,
      }),
    });
  });
  await page.route(`**/api/v1/keys/${SERVICE_ID}/authorization`, (route) => {
    if (deleted) {
      return route.fulfill({ status: 404, contentType: "application/json", body: "" });
    }
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ...EVIDENCE, state_version: serviceStateVersion }),
    });
  });
  return requestBodies;
}

async function mountDialog(
  page: Page,
  modulePath: string,
  exportName: string,
  props: Record<string, unknown>,
): Promise<void> {
  await page.evaluate(
    async ({ modulePath: path, exportName: name, props: dialogProps }) => {
      const reactMod = (await import("/node_modules/.vite/deps/react.js")) as {
        default?: typeof import("react");
      } & typeof import("react");
      const React = reactMod.default ?? reactMod;
      const reactDomMod = (await import(
        "/node_modules/.vite/deps/react-dom_client.js"
      )) as {
        default?: { createRoot?: typeof import("react-dom/client").createRoot };
        createRoot?: typeof import("react-dom/client").createRoot;
      };
      const createRoot =
        reactDomMod.createRoot ?? reactDomMod.default?.createRoot;
      if (typeof createRoot !== "function") {
        throw new Error(
          `createRoot missing: ${Object.keys(reactDomMod).join(",")}`,
        );
      }
      const mod = (await import(/* @vite-ignore */ path)) as Record<
        string,
        (props: Record<string, unknown>) => unknown
      >;
      const Component = mod[name];
      const host = document.createElement("div");
      host.id = "wave2-service-harness";
      document.body.appendChild(host);
      const completed = document.createElement("div");
      completed.setAttribute("data-testid", "wave2-complete");
      document.body.appendChild(completed);
      createRoot(host).render(
        React.createElement(Component, {
          ...dialogProps,
          open: true,
          onOpenChange: () => undefined,
          onComplete: (id: string) => {
            completed.textContent = id;
          },
        }),
      );
    },
    { modulePath, exportName, props },
  );
}

test("wave2_service_update_dialog_ui_contract", async ({ page }) => {
  // Falsifier: remove any semantic field or actionRequestId from the dialog POST;
  // the request-body assertion fails even though the stub still returns 200.
  await openAssistant(page);
  const requestBodies = await stubServiceApis(page);
  await mountDialog(
    page,
    "/src/components/assistant/assistant-service-update-dialog.tsx",
    "AssistantServiceUpdateDialog",
    {
      actionRequestId: "act-e2e-update",
      params: {
        userServiceId: SERVICE_ID,
        name: "Example",
        endpointUrl: "https://api.example.com",
      },
    },
  );

  await expect(
    page.getByRole("heading", { name: "Update connected service" }),
  ).toBeVisible();
  await page.getByRole("button", { name: "Update service" }).click();
  await expect.poll(() => requestBodies).toEqual([
    {
      actionRequestId: "act-e2e-update",
      userServiceId: SERVICE_ID,
      name: "Example",
      endpointUrl: "https://api.example.com",
    },
  ]);
  await expect(
    page.getByText("Authorization evidence verified."),
  ).toBeVisible();
  await page.getByRole("button", { name: "Report service" }).click();
  await expect(page.getByTestId("wave2-complete")).toHaveText(SERVICE_ID);
});

test("wave2_service_delete_dialog_ui_contract", async ({ page }) => {
  // Falsifier: omit cascadeGrant or cascadeSiblings on the confirmation retry;
  // the request-body assertion fails even though the stub still returns 200.
  await openAssistant(page);
  const requestBodies = await stubServiceApis(page, {
    deleteConflictOnce: true,
  });
  await mountDialog(
    page,
    "/src/components/assistant/assistant-service-delete-dialog.tsx",
    "AssistantServiceDeleteDialog",
    {
      actionRequestId: "act-e2e-delete",
      params: { userServiceId: SERVICE_ID },
    },
  );

  await expect(
    page.getByRole("heading", { name: "Delete connected service" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Delete service" }),
  ).toHaveCount(0);
  await page.getByRole("button", { name: "I understand, continue" }).click();
  await page.getByRole("button", { name: "Delete service" }).click();
  await expect(page.getByText(/revoke the shared grant/i)).toBeVisible();
  await page.getByRole("button", { name: "Delete and revoke grant" }).click();
  await expect.poll(() => requestBodies).toEqual([
    {
      actionRequestId: "act-e2e-delete",
      userServiceId: SERVICE_ID,
    },
    {
      actionRequestId: "act-e2e-delete",
      userServiceId: SERVICE_ID,
      cascadeGrant: true,
      cascadeSiblings: [
        { user_service_id: "sibling", name: "other", slug: "other" },
      ],
    },
  ]);
  await expect(page.getByTestId("wave2-complete")).toHaveText(SERVICE_ID);
});
