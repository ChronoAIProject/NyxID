import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  loginInventory,
  requestedRead,
} from "@/lib/__fixtures__/login-inventory";
import { compareKey } from "@/lib/login-permissions";
import { agentKeyOptionsSchema } from "@/schemas/agent-key-login";
import { LoginAccessRows } from "./login-access-rows";

afterEach(cleanup);

describe("login access ownership and expansion", () => {
  it.each(["new", "existing"])(
    "distinguishes same-name connections in %s Key review, including non-admin organizations",
    async (kind) => {
      const user = userEvent.setup();
      const inventory = loginInventory();
      const connection = inventory.connections[0]!;
      connection.label = "GitHub";
      inventory.options.services.push({
        id: "org-svc",
        name: "GitHub personal",
        owner_id: "org",
        owner_name: "Engineering",
      });
      inventory.connections.push({
        ...connection,
        id: "org-svc",
        owner_id: "org",
      });
      // Members can grant shared services without being organization Key admins.
      inventory.options = agentKeyOptionsSchema.parse(inventory.options);
      expect(inventory.options.orgs).toEqual([]);
      const key = inventory.options.keys[0]!;
      key.effective_services = inventory.connections;
      key.allowed_service_ids = inventory.connections.map((c) => c.id);
      const onToggle = vi.fn();
      const { container } = render(
        <LoginAccessRows
          apiKey={key}
          comparison={compareKey(key, requestedRead, inventory)}
          requested={requestedRead}
          inventory={inventory}
          disabled={false}
          selection={
            kind === "new"
              ? {
                  inventory,
                  selectedIds: ["svc"],
                  impliedIds: [],
                  allowAll: false,
                  onToggle,
                }
              : undefined
          }
        />,
      );
      for (const row of container.querySelectorAll("details")) {
        expect(row).toHaveAttribute("open");
      }
      const row = screen.getByLabelText("GitHub access");
      const summary = row.querySelector("summary")!;
      expect(summary).toHaveTextContent("GitHub (Personal)");
      expect(summary).toHaveTextContent("GitHub (Organization · Engineering)");
      if (kind === "new") {
        expect(
          within(row).getByRole("checkbox", {
            name: "Grant connection GitHub (Personal)",
          }),
        ).toBeChecked();
        const organization = within(row).getByRole("checkbox", {
          name: "Grant connection GitHub (Organization · Engineering)",
        });
        expect(organization).not.toBeChecked();
        await user.click(organization);
        expect(onToggle).toHaveBeenCalledExactlyOnceWith("org-svc");
      }
      await user.click(summary);
      expect(row).not.toHaveAttribute("open");
      expect(summary).toBeVisible();
      expect(summary).toHaveTextContent("Organization · Engineering");
    },
  );
});
