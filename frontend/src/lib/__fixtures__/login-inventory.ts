import type { LoginInventory } from "../login-permissions";
export function loginInventory(): LoginInventory {
  const services = [{ id: "svc", name: "GitHub personal", owner_id: "user" }];
  const inventory: LoginInventory = {
    options: {
      keys: [
        {
          id: "key",
          name: "Reader",
          key_prefix: "nyxid_ag_test",
          owner_type: "personal",
          owner_id: "user",
          owner_name: "Human",
          scopes: "read proxy",
          allow_all_services: false,
          allow_all_nodes: false,
          allowed_service_ids: ["svc"],
          allowed_node_ids: [],
          allowed_services: services,
          allowed_nodes: [],
          expires_at: null,
          rate_limit_per_second: null,
          rate_limit_burst: null,
          platform: "codex",
          created_now: false,
        },
      ],
      services,
      nodes: [],
      orgs: [],
    },
    connections: [
      {
        id: "svc",
        label: "GitHub personal",
        slug: "my-github",
        catalog_service_slug: "github",
        is_active: true,
        status: "active",
        expires_at: null,
        node_id: null,
        granted_scopes: ["repo:read"],
      },
    ],
    catalog: [
      {
        slug: "github",
        name: "GitHub",
        scope_catalog: [
          {
            scope: "repo:read",
            label: "Read repositories",
            description: "Read repository data",
          },
          {
            scope: "repo:write",
            label: "Write repositories",
            description: "Modify repository data",
          },
        ],
      },
    ],
  };
  inventory.connections[0]!.permission_snapshot = "c".repeat(64);
  inventory.options.connections = inventory.connections;
  inventory.options.keys[0]!.effective_services = inventory.connections;
  inventory.options.keys[0]!.permission_snapshot = "k".repeat(64);
  return inventory;
}
export const requestedRead = {
  permissions: ["read", "proxy"],
  services: [],
  service_permissions: ["github::repo:read"],
};
