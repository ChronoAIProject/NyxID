import { describe, expect, it } from "vitest";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";
import {
  compareKey,
  exactConnectionDefaults,
  permissionOptions,
  draftSummary,
  effectiveLoginConnections,
  loginConnectionLabel,
  preferredOwnerKey,
} from "./login-permissions";
import { loginInventory, requestedRead } from "./__fixtures__/login-inventory";

describe("device login permission comparisons", () => {
  it.each(["user", "org"])(
    "selects only the requested owner's CMA and distinguishes identical names: %s",
    (owner) => {
      const inventory = loginInventory();
      inventory.options.personal_owner_id = "user";
      inventory.options.orgs = [
        { id: "org", owner_id: "org", name: "ChronoAI" },
      ];
      inventory.options.services = ["user", "org"].map((id) => ({
        id: `cma-${id}`,
        name: "cma",
        owner_id: id,
      }));
      inventory.connections = ["user", "org"].map((id) => ({
        ...inventory.connections[0]!,
        id: `cma-${id}`,
        owner_id: id,
        label: "CMA",
        slug: "cma",
        catalog_service_slug: "cma",
        granted_scopes: null,
      }));
      inventory.catalog = [{ slug: "cma", name: "CMA", scope_catalog: null }];
      const original = inventory.options.keys[0]!;
      inventory.options.keys = ["user", "org"].map((id) => ({
        ...original,
        id: `key-${id}`,
        owner_type: id === "user" ? "personal" : "org",
        owner_id: id,
        scopes: "proxy",
        allowed_service_ids: [`cma-${id}`],
        allowed_services: [
          inventory.options.services.find((s) => s.owner_id === id)!,
        ],
        effective_services: [
          inventory.connections.find((c) => c.owner_id === id)!,
        ],
      }));
      const requested = {
        permissions: ["proxy"],
        services: ["cma"],
        service_permissions: [],
      };
      expect(exactConnectionDefaults(requested, inventory, owner)).toEqual([
        `cma-${owner}`,
      ]);
      expect(preferredOwnerKey(inventory, requested, owner)).toBe(
        `key-${owner}`,
      );
      expect(loginConnectionLabel(inventory.connections[0]!, inventory)).toBe(
        "Personal · CMA",
      );
      expect(loginConnectionLabel(inventory.connections[1]!, inventory)).toBe(
        "ChronoAI · CMA",
      );
      inventory.options.keys.find(
        (k) => k.owner_id === owner,
      )!.allow_all_services = true;
      expect(preferredOwnerKey(inventory, requested, owner)).toBeUndefined();
      inventory.connections.push({
        ...inventory.connections.find((c) => c.owner_id === owner)!,
        id: "duplicate",
      });
      inventory.options.services.push({
        id: "duplicate",
        name: "cma",
        owner_id: owner,
      });
      expect(exactConnectionDefaults(requested, inventory, owner)).toEqual([]);
    },
  );
  it("includes inferred platform rows and durable future access without crossing owners", () => {
    const inventory = loginInventory();
    inventory.options.personal_owner_id = "user";
    inventory.options.services.push(
      {
        id: "platform",
        name: "Platform",
        owner_id: "user",
        auto_connected: true,
      },
      {
        id: "org-platform",
        name: "Org platform",
        owner_id: "org",
        auto_connected: true,
      },
    );
    inventory.connections.push(
      ...["platform", "org-platform"].map((id) => ({
        ...inventory.connections[0]!,
        id,
        catalog_service_slug: "platform",
        credential_binding: "platform",
        credential_missing: false,
        granted_scopes: null,
      })),
    );
    inventory.catalog.push({ slug: "platform", name: "Platform" });
    const data: CreateApiKeyFormData = {
      name: "Draft",
      scopes: ["read", "proxy"],
      allowed_service_ids: ["svc"],
      allow_auto_connected_services: true,
    };
    const draft = draftSummary(data, inventory, "wrong-fallback");
    expect(draft.owner_id).toBe("user");
    expect(
      effectiveLoginConnections(draft, inventory).map((c) => c.id),
    ).toEqual(["svc", "platform"]);
    const comparison = compareKey(draft, requestedRead, inventory);
    expect(comparison).toMatchObject({ matches: true, exact: false });
    expect(comparison.extras.map((e) => e.label)).toContain(
      "All current and future auto-connected platform services",
    );
    expect(comparison.extras.map((e) => e.label)).toContain(
      "Provider access not reported",
    );
    const org = draftSummary(
      { ...data, target_org_id: "org", allowed_service_ids: [] },
      inventory,
      "user",
    );
    expect(effectiveLoginConnections(org, inventory).map((c) => c.id)).toEqual([
      "org-platform",
    ]);
    expect(compareKey(org, requestedRead, inventory).matches).toBe(false);
  });
  it("matches usable platform services with no user credential but cannot invent provider scopes", () => {
    const inventory = loginInventory();
    const key = inventory.options.keys[0]!;
    key.allowed_service_ids = [];
    key.allow_auto_connected_services = true;
    const connection = key.effective_services![0]!;
    connection.credential_binding = "platform";
    connection.auto_connected = true;
    connection.credential_missing = false;
    connection.granted_scopes = null;
    const requested = {
      ...requestedRead,
      services: ["github"],
      service_permissions: [],
    };
    expect(compareKey(key, requested, inventory)).toMatchObject({
      matches: true,
      exact: false,
    });
    expect(compareKey(key, requestedRead, inventory).matches).toBe(false);
    connection.credential_missing = true;
    expect(compareKey(key, requested, inventory).matches).toBe(false);
  });
  it("uses bound overrides instead of default access and permits unchanged unavailable extras", () => {
    const inventory = loginInventory(),
      key = inventory.options.keys[0]!;
    key.effective_services = [
      {
        ...inventory.connections[0]!,
        granted_scopes: ["repo:read", "repo:write"],
      },
    ];
    expect(
      compareKey(key, requestedRead, inventory).extras.map((e) => e.label),
    ).toContain("Write repositories");
    key.effective_services[0]!.credential_missing = true;
    expect(compareKey(key, requestedRead, inventory).matches).toBe(false);
    key.effective_services = [
      inventory.connections[0]!,
      {
        ...inventory.connections[0]!,
        id: "inactive",
        is_active: false,
        granted_scopes: null,
      },
    ];
    key.allowed_service_ids.push("inactive", "missing");
    const comparison = compareKey(key, requestedRead, inventory);
    expect(comparison).toMatchObject({ matches: true, exact: false });
    expect(comparison.extras.map((e) => e.label)).toEqual(
      expect.arrayContaining([
        "Connection unavailable",
        "Service access not reported",
      ]),
    );
  });
  it("requires usable proxy permission and normalizes its legacy spelling", () => {
    const inventory = loginInventory(),
      key = inventory.options.keys[0]!;
    expect(compareKey(key, requestedRead, inventory).exact).toBe(true);
    expect(
      compareKey({ ...key, scopes: "read" }, requestedRead, inventory).matches,
    ).toBe(false);
    expect(
      compareKey({ ...key, scopes: "read proxy:*" }, requestedRead, inventory)
        .exact,
    ).toBe(true);
  });
  it("requires every requested scope and service and excludes disabled/missing/expired connections", () => {
    const inventory = loginInventory(),
      key = inventory.options.keys[0]!;
    expect(
      compareKey(
        key,
        {
          ...requestedRead,
          service_permissions: ["github::repo:read", "github::repo:write"],
        },
        inventory,
      ).matches,
    ).toBe(false);
    expect(
      compareKey(key, { ...requestedRead, services: ["unknown"] }, inventory)
        .matches,
    ).toBe(false);
    for (const change of [
      { is_active: false },
      { credential_missing: true },
      { status: "revoked" },
      { expires_at: "2000-01-01T00:00:00Z" },
    ])
      expect(
        compareKey(
          {
            ...key,
            effective_services: [{ ...inventory.connections[0]!, ...change }],
          },
          requestedRead,
          inventory,
        ).matches,
      ).toBe(false);
  });
  it("counts identity, provider extras, additional services, nodes and future grants", () => {
    const inventory = loginInventory(),
      key = inventory.options.keys[0]!;
    inventory.connections[0]!.granted_scopes!.push("email", "repo:write");
    const comparison = compareKey(
      {
        ...key,
        scopes: "read proxy profile",
        allowed_node_ids: ["node"],
        allow_all_services: true,
        allow_all_nodes: true,
      },
      requestedRead,
      inventory,
    );
    expect(comparison.matches).toBe(true);
    expect(comparison.extras.map((e) => e.label)).toEqual([
      "profile",
      "email",
      "Write repositories",
      "All current and future services",
      "All current and future nodes",
      "Unreported node node",
    ]);
    expect(comparison.exact).toBe(false);
  });
  it("never claims unreported provider access is exact", () => {
    const inventory = loginInventory(),
      key = inventory.options.keys[0]!;
    inventory.connections[0]!.granted_scopes = null;
    expect(compareKey(key, requestedRead, inventory).matches).toBe(false);
    expect(
      compareKey(
        key,
        { ...requestedRead, service_permissions: [], services: ["github"] },
        inventory,
      ),
    ).toMatchObject({ matches: true, exact: false });
  });
  it("prefills only one exact connection, rejecting ambiguous, broader, unknown and node choices", () => {
    const inventory = loginInventory();
    expect(exactConnectionDefaults(requestedRead, inventory, "user")).toEqual([
      "svc",
    ]);
    inventory.connections[0]!.granted_scopes!.push("email");
    expect(exactConnectionDefaults(requestedRead, inventory, "user")).toEqual(
      [],
    );
    inventory.connections[0]!.granted_scopes = ["repo:read"];
    inventory.connections.push({ ...inventory.connections[0]!, id: "svc2" });
    inventory.options.services.push({
      id: "svc2",
      owner_id: "user",
      name: "Second account",
    });
    expect(exactConnectionDefaults(requestedRead, inventory, "user")).toEqual(
      [],
    );
  });
  it("uses catalog slug and canonicalizes Google identity aliases only for Google", () => {
    const inventory = loginInventory();
    inventory.catalog.push({
      slug: "api-google-drive",
      name: "Drive",
      default_scopes: [
        "email",
        "https://www.googleapis.com/auth/userinfo.email",
      ],
    });
    const options = permissionOptions(inventory);
    expect(options.some((o) => o.value === "github::repo:read")).toBe(true);
    expect(options.filter((o) => o.group === "api-google-drive")).toHaveLength(
      1,
    );
  });
});
