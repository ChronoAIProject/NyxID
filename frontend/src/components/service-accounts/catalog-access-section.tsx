import { toast } from "sonner";
import { useUpdateServiceAccount } from "@/hooks/use-service-accounts";
import { useChangeReview } from "@/components/shared/change-review-dialog";
import { DetailSection } from "@/components/shared/detail-section";
import { Button } from "@/components/ui/button";
import type { Role } from "@/types/rbac";
import type {
  ServiceAccount,
  UpdateServiceAccountRequest,
} from "@/types/service-accounts";
const READ = "nyxid:catalog:skills:read";
const WRITE = "nyxid:catalog:skills:write";

const SUPPORTED = new Set([
  "catalog:skills:read",
  "catalog:skills:write",
  "user-services:read",
  "proxy",
]);

export function CatalogAccessSection({
  account,
  roles,
  onEditAccount,
}: {
  readonly account: ServiceAccount;
  readonly roles: readonly Role[];
  readonly onEditAccount: () => void;
}) {
  const update = useUpdateServiceAccount();
  const assigned = roles.filter((role) => account.role_ids.includes(role.id));
  const global = assigned.filter((role) => role.client_id === null);
  const read = global.some((role) => role.permissions.includes(READ));
  const write = global.some((role) => role.permissions.includes(WRITE));
  const scopes = new Set(account.allowed_scopes.split(/\s+/).filter(Boolean));
  const required = [
    ...(read ? ["catalog:skills:read", "user-services:read"] : []),
    ...(write ? ["catalog:skills:write"] : []),
  ];
  const missing = required.filter((scope) => !scopes.has(scope));
  const unsupported = [...scopes].filter((scope) => !SUPPORTED.has(scope));
  const snapshot = JSON.stringify({
    id: account.id,
    roles: [...account.role_ids].sort(),
    scopes: [...scopes].sort(),
    purpose: account.purpose,
    protected: account.platform_protected,
    active: account.is_active,
    permissions: assigned
      .map((role) => ({
        id: role.id,
        client: role.client_id,
        permissions: [...role.permissions].sort(),
      }))
      .sort((a, b) => a.id.localeCompare(b.id)),
  });
  const review = useChangeReview<{
    snapshot: string;
    expected: NonNullable<UpdateServiceAccountRequest["expected_access"]>;
    scopes: string;
    roles: readonly string[];
  }>(
    async (pending) => {
      const result = await update.mutateAsync({
        saId: account.id,
        data: {
          role_ids: pending.roles,
          allowed_scopes: pending.scopes,
          expected_access: pending.expected,
        },
      });
      if (result.purpose !== "catalog_editor" || !result.platform_protected)
        throw new Error(
          "Catalog access was not activated. Reload the account and verify its global role permissions.",
        );
      toast.success(
        "Catalog access applied. Request a token with the configured scopes.",
      );
    },
    (pending) => pending.snapshot !== snapshot,
    account.id,
  );
  const activated = account.purpose === "catalog_editor";
  const applicationCurrent =
    activated && account.platform_protected && missing.length === 0;
  const accountReady =
    activated && account.platform_protected && account.is_active;
  const readReady =
    accountReady &&
    read &&
    scopes.has("catalog:skills:read") &&
    scopes.has("user-services:read");
  const writeReady =
    accountReady && write && scopes.has("catalog:skills:write");
  function apply() {
    const nextScopes = [...scopes, ...missing].join(" ");
    review.review(
      {
        snapshot,
        scopes: nextScopes,
        roles: [...account.role_ids],
        expected: {
          role_ids: [...account.role_ids],
          allowed_scopes: account.allowed_scopes,
          purpose: account.purpose ?? "general",
          platform_protected: account.platform_protected ?? false,
          is_active: account.is_active,
        },
      },
      [
        ...(!activated || !account.platform_protected
          ? [
              {
                field: "Catalog access and management",
                before: activated
                  ? "Catalog editor; protection not applied"
                  : "Catalog access not applied",
                after:
                  "All current and future catalog services; managed by platform administrators",
              },
            ]
          : []),
        ...(missing.length
          ? [
              {
                field: "Allowed scopes",
                before: account.allowed_scopes,
                after: nextScopes,
              },
            ]
          : []),
      ],
    );
  }
  return (
    <DetailSection title="Catalog access">
      <div className="space-y-3 p-5 text-sm">
        <p>
          Use the existing account to read and amend platform catalog skills. No
          connection grants or new secret are needed.
        </p>
        <p>
          Catalog read role: {read ? "Assigned" : "Missing"}. Catalog write
          role: {write ? "Assigned" : "Missing"}.
        </p>
        <p>
          Required global role permissions: <code>{READ}</code> and{" "}
          <code>{WRITE}</code>.{" "}
          <a href="/admin/roles" className="underline">
            Manage roles
          </a>
          .
        </p>
        {assigned.map((role) => (
          <a
            key={role.id}
            href={`/admin/roles/${role.id}`}
            className="mr-3 underline"
          >
            {role.name}
          </a>
        ))}
        <p>
          Catalog read settings: {readReady ? "Ready" : "Incomplete"}. Catalog
          write settings: {writeReady ? "Ready" : "Incomplete"}.
        </p>
        {!activated && (
          <p>Catalog access has not been applied to this account.</p>
        )}
        <p>
          Access covers all current and future catalog services. Requests
          require a token with the configured scopes.
        </p>
        {!account.is_active && (
          <p>
            The account is disabled. Applying catalog access will keep it
            disabled.
          </p>
        )}
        {missing.length > 0 && <p>Scopes to add: {missing.join(" ")}</p>}
        {(read || write || activated) && unsupported.length > 0 && (
          <p role="alert">
            Edit unsupported scopes before applying catalog access:{" "}
            {unsupported.join(" ")}
          </p>
        )}
        <div className="flex gap-2">
          <Button
            onClick={apply}
            disabled={
              (!read && !write) ||
              unsupported.length > 0 ||
              applicationCurrent ||
              review.saving
            }
          >
            Apply catalog access
          </Button>
          <Button variant="outline" onClick={onEditAccount}>
            Edit account settings
          </Button>
        </div>
      </div>
      {review.dialog}
    </DetailSection>
  );
}
