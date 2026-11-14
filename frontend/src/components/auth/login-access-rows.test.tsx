import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  loginInventory,
  requestedRead,
} from "@/lib/__fixtures__/login-inventory";
import { compareKey } from "@/lib/login-permissions";
import { agentKeyOptionsSchema } from "@/schemas/agent-key-login";
import { AUTO_EXPAND_ROW_LIMIT, LoginAccessRows } from "./login-access-rows";

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

  it("names an organization connection the approver lost, marks it unavailable and never shows its raw ID", () => {
    const inventory = loginInventory();
    const key = inventory.options.keys[0]!;
    const lost = { ...inventory.connections[0]!, id: "lost-svc", label: "GitHub", owner_id: "org-id" };
    // The key still lists the connection, but the approver's usable services no longer do.
    key.effective_services = [...inventory.connections, lost];
    key.allowed_service_ids = ["svc", "lost-svc"];
    key.allowed_services = [
      ...key.allowed_services,
      { id: "lost-svc", name: "github", owner_id: "org-id", owner_name: "Engineering" },
    ];
    const { rerender } = render(
      <LoginAccessRows
        apiKey={key}
        comparison={compareKey(key, requestedRead, inventory)}
        requested={requestedRead}
        inventory={inventory}
        disabled={false}
      />,
    );
    const summary = screen.getByLabelText("GitHub access").querySelector("summary")!;
    expect(summary).toHaveTextContent("GitHub personal (Personal)");
    expect(summary).toHaveTextContent("GitHub (Organization · Engineering) No longer available");
    // A neutral pill, not a warning colour, marks the lost connection.
    expect(within(summary).getByText("No longer available")).not.toHaveClass("text-warning");

    // Without any name source the label stays generic rather than exposing an ID.
    key.allowed_services = key.allowed_services.map((s) =>
      s.id === "lost-svc" ? { ...s, owner_name: null } : s,
    );
    rerender(
      <LoginAccessRows
        apiKey={{ ...key }}
        comparison={compareKey(key, requestedRead, inventory)}
        requested={requestedRead}
        inventory={inventory}
        disabled={false}
      />,
    );
    expect(summary).toHaveTextContent("GitHub (Organization) No longer available");
    expect(summary).not.toHaveTextContent("org-id");
  });

  it("opens only rows needing a choice in long lists and collapses or expands every row", async () => {
    const user = userEvent.setup();
    const inventory = loginInventory();
    const key = inventory.options.keys[0]!;
    const extra = ["notion", "figma", "linear", "stripe", "sentry", "asana"].map((slug) => ({
      ...inventory.connections[0]!,
      id: slug,
      label: slug,
      slug,
      catalog_service_slug: slug,
    }));
    inventory.connections.push(...extra);
    inventory.options.services.push(...extra.map((c) => ({ id: c.id, name: c.slug, owner_id: "user" })));
    key.effective_services = inventory.connections;
    key.allowed_service_ids = inventory.connections.map((c) => c.id);
    // A requested service with no connection must stay open as "Choose account".
    const requested = { ...requestedRead, service_permissions: [...requestedRead.service_permissions, "slack::chat:write"] };
    const { container } = render(
      <LoginAccessRows
        apiKey={key}
        comparison={compareKey(key, requested, inventory)}
        requested={requested}
        inventory={inventory}
        disabled={false}
      />,
    );
    const rows = () => [...container.querySelectorAll("details")];
    expect(rows().length).toBeGreaterThan(AUTO_EXPAND_ROW_LIMIT);
    const openRows = () => rows().filter((r) => r.open).map((r) => r.getAttribute("aria-label"));
    expect(openRows()).toEqual(["slack access"]);
    expect(screen.getByLabelText("slack access").querySelector("summary")).toHaveTextContent("Choose account");

    await user.click(screen.getByRole("button", { name: "Collapse all" }));
    expect(openRows()).toEqual([]);
    await user.click(screen.getByRole("button", { name: "Expand all" }));
    expect(openRows()).toHaveLength(rows().length);
    // An individual toggle still works after a bulk change.
    await user.click(screen.getByLabelText("notion access").querySelector("summary")!);
    expect(screen.getByLabelText("notion access")).not.toHaveAttribute("open");
    expect(screen.getByRole("button", { name: "Collapse all" })).toBeVisible();
  });
});
