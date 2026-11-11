import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  loginInventory,
  requestedRead,
} from "@/lib/__fixtures__/login-inventory";
import { newApiKeyDefaults } from "@/schemas/api-keys";
import { newKeySelection } from "@/schemas/agent-key-login";
import { LoginKeyDraft } from "./login-key-draft";

afterEach(cleanup);

describe("login key draft owner authority", () => {
  it.each([
    [undefined, true],
    ["org", true],
    [undefined, false],
    ["org", false],
  ] as const)(
    "resets lifetime after changing owner from %s (nonexpiring: %s)",
    async (target_org_id, never_expires) => {
      const user = userEvent.setup();
      const inventory = loginInventory();
      inventory.options.orgs = [
        { id: "org", name: "Organization", owner_id: "org" },
      ];
      const onDraft = vi.fn();
      render(
        <LoginKeyDraft
          initial={{
            ...newApiKeyDefaults(),
            name: "Login key",
            scopes: ["proxy"],
            target_org_id,
            never_expires,
            expires_at: never_expires ? null : "2099-01-01",
            allow_all_services: true,
            allow_all_nodes: true,
          }}
          inventory={inventory}
          requested={requestedRead}
          initialRequested={requestedRead}
          actor="user"
          disabled={false}
          onDraft={onDraft}
          onApprove={vi.fn()}
          onRequestedChange={vi.fn()}
          onDeny={vi.fn()}
        />,
      );
      await user.click(screen.getByText("Customize", { exact: true }));
      expect(
        screen.getByRole("checkbox", { name: "No expiry" }),
      ).toHaveAttribute("aria-checked", String(never_expires));
      await user.click(screen.getByRole("combobox", { name: "Owner" }));
      await user.click(
        screen.getByRole("option", {
          name: target_org_id ? "Personal" : "Organization",
        }),
      );
      await waitFor(() =>
        expect(
          screen.getByRole("checkbox", { name: "No expiry" }),
        ).not.toBeChecked(),
      );
      const draft = onDraft.mock.lastCall![0];
      expect(draft).toMatchObject({
        never_expires: false,
        expires_at: newApiKeyDefaults().expires_at,
        allow_all_services: false,
        allow_all_nodes: false,
        allow_auto_connected_services: false,
        allowed_service_ids: [],
        allowed_node_ids: [],
        target_org_id: target_org_id ? undefined : "org",
      });
      const selection = newKeySelection(draft);
      expect(selection.success).toBe(true);
      if (selection.success) {
        expect(selection.data).not.toHaveProperty("never_expires");
        expect(new Date(selection.data.expires_at!).getTime()).toBeGreaterThan(
          Date.now(),
        );
      }
      // Returning to the original owner also requires a fresh lifetime choice.
      await user.click(screen.getByRole("checkbox", { name: "No expiry" }));
      await user.click(screen.getByRole("combobox", { name: "Owner" }));
      await user.click(
        screen.getByRole("option", {
          name: target_org_id ? "Organization" : "Personal",
        }),
      );
      expect(onDraft.mock.lastCall![0]).toMatchObject({
        never_expires: false,
        expires_at: newApiKeyDefaults().expires_at,
      });
    },
  );
});
