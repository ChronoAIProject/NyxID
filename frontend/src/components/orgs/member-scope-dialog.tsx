import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { translateErrorMessage } from "@/lib/error-messages";
import { useMemo, useState } from "react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { ServiceIcon } from "@/components/service-icon";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label, FieldHelp } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import { ApiError } from "@/lib/api-client";
import { useKeys } from "@/hooks/use-keys";
import { useUpdateMember } from "@/hooks/use-org-members";
import { useOrgRoleScopes } from "@/hooks/use-org-role-scopes";
import type { MemberResponse } from "@/schemas/orgs";
import type { KeyInfo } from "@/types/keys";
import { connectionIconUrl } from "@/lib/connection-icon";

interface MemberScopeDialogProps {
  readonly orgId: string;
  readonly member: MemberResponse | null;
  readonly onOpenChange: (open: boolean) => void;
}

/**
 * Edit `allowed_service_ids` for a single member of an org. Admins use this
 * to restrict which org services a member can proxy through, or clear the
 * restriction to grant full access to every org service.
 *
 * The picker lists every service visible to the admin under the given org
 * (i.e. services whose `credential_source` is this org). Each service is a
 * checkbox; unchecking all services implicitly becomes "no access" and the
 * backend enforces that at proxy time. To restore unrestricted access, use
 * the "Allow all services" toggle which clears the `allowed_service_ids`
 * field back to `null`.
 */
export function MemberScopeDialog({
  orgId,
  member,
  onOpenChange,
}: MemberScopeDialogProps) {
  useUiTranslation();
  return (
    <Dialog open={member !== null} onOpenChange={onOpenChange}>
      <DialogContent>
        {member && (
          // Key by membership id so reopening the dialog for a different
          // member fully resets the local form state. This avoids the
          // "sync props into state via useEffect" footgun that React 19's
          // eslint rule flags as a cascading render.
          <MemberScopeForm
            key={member.membership_id}
            orgId={orgId}
            member={member}
            onClose={() => onOpenChange(false)}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

function MemberScopeForm({
  orgId,
  member,
  onClose,
}: {
  readonly orgId: string;
  readonly member: MemberResponse;
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const { data: keys, isLoading: keysLoading } = useKeys();
  const { data: roleScopes, isLoading: roleScopesLoading } =
    useOrgRoleScopes(orgId);
  const updateMutation = useUpdateMember();

  const orgServices = useMemo(
    (): readonly KeyInfo[] =>
      (keys ?? []).filter(
        (k) =>
          k.credential_source?.type === "org" &&
          k.credential_source.org_id === orgId,
      ),
    [keys, orgId],
  );

  const initialOverrideScope = member.allowed_service_ids;
  const initialEffectiveScope = member.effective_allowed_service_ids;
  const [scopeMode, setScopeMode] = useState(member.scope_source);
  const [allowAll, setAllowAll] = useState(
    member.scope_source === "override"
      ? initialOverrideScope === null
      : initialEffectiveScope === null,
  );
  const [selectedIds, setSelectedIds] = useState<readonly string[]>(
    member.scope_source === "override"
      ? (initialOverrideScope ?? [])
      : (initialEffectiveScope ?? []),
  );

  function toggleService(serviceId: string) {
    setSelectedIds((prev) =>
      prev.includes(serviceId)
        ? prev.filter((id) => id !== serviceId)
        : [...prev, serviceId],
    );
  }

  async function handleSave() {
    try {
      await updateMutation.mutateAsync({
        orgId,
        memberId: member.user_id,
        body:
          scopeMode === "inherit"
            ? { scope_source: "inherit" }
            : {
                scope_source: "override",
                allowed_service_ids: allowAll ? null : [...selectedIds],
              },
      });
      toast.success(translateUi("Service access updated"));
      onClose();
    } catch (err) {
      toast.error(
        translateErrorMessage(err instanceof ApiError ? err.message : "Failed to update scope"),
      );
    }
  }

  const memberName = member.display_name ?? member.email ?? member.user_id;
  const inheritedScope =
    roleScopes?.find((scope) => scope.role === member.role)
      ?.allowed_service_ids ?? null;
  const previewLoading = keysLoading || roleScopesLoading;
  const inheritedSummary = describeScope(inheritedScope, orgServices);

  return (
    <>
      <DialogHeader>
        <DialogTitle>{translateUi("Edit service access")}</DialogTitle>
        <DialogDescription>{translateUi("Choose which org services ")}<strong>{memberName}</strong>{translateUi(" is allowed to proxy through.")}</DialogDescription>
      </DialogHeader>

      <div className="space-y-4">
        <div className="grid gap-2">
          <Label className="flex cursor-pointer items-start gap-3 rounded-lg border border-border bg-muted/30 p-3">
            <input
              type="radio"
              name="member-scope-mode"
              value="inherit"
              checked={scopeMode === "inherit"}
              onChange={() => setScopeMode("inherit")}
              className="mt-1"
            />
            <span className="space-y-1">
              <span className="block text-foreground">{translateUi("Inherit from role default")}</span>
              <FieldHelp className="block text-muted-foreground">
                {previewLoading
                  ? translateUi("Loading current role permissions...")
                  : translateUi("This member can access: {{value1}}", { value1: inheritedSummary })}
              </FieldHelp>
            </span>
          </Label>

          <Label className="flex cursor-pointer items-start gap-3 rounded-lg border border-border bg-muted/30 p-3">
            <input
              type="radio"
              name="member-scope-mode"
              value="override"
              checked={scopeMode === "override"}
              onChange={() => setScopeMode("override")}
              className="mt-1"
            />
            <span className="space-y-1">
              <span className="block text-foreground">{translateUi("Customize for this member")}</span>
              <FieldHelp className="block text-muted-foreground">{translateUi("Custom scopes override role defaults until reset.")}</FieldHelp>
              {scopeMode === "override" && member.scope_source === "inherit" && (
                <FieldHelp variant="emphasis" className="block text-amber-600 dark:text-amber-400">{translateUi("Snapshot warning: saving will freeze this member at the current role scope (")}{inheritedSummary}{translateUi("). Future changes to the role default won't apply until you reset to inherit.")}</FieldHelp>
              )}
            </span>
          </Label>
        </div>

        {scopeMode === "override" && (
          <div className="flex items-start gap-3 rounded-lg border border-border bg-muted/30 p-3">
            <Checkbox
              id="member-scope-allow-all"
              checked={allowAll}
              onCheckedChange={(checked) => setAllowAll(checked === true)}
              className="mt-0.5"
            />
            <div className="space-y-1">
              <Label
                htmlFor="member-scope-allow-all"
                className="cursor-pointer"
              >{translateUi("Allow all org services")}</Label>
              <p className="text-13 text-muted-foreground">{translateUi("When enabled, the member can use every current and future service this org owns.")}</p>
            </div>
          </div>
        )}

        {scopeMode === "override" && !allowAll && (
          <div className="space-y-2">
            <Label className="text-muted-foreground">{translateUi("Services")}</Label>
            {keysLoading ? (
              <div className="space-y-2">
                <Skeleton className="h-8 w-full" />
                <Skeleton className="h-8 w-full" />
                <Skeleton className="h-8 w-full" />
              </div>
            ) : orgServices.length === 0 ? (
              <div className="rounded-lg border border-dashed border-border p-4 text-center text-13 text-muted-foreground">{translateUi("This org has no services yet. Add one under AI Services first.")}</div>
            ) : (
              <div className="max-h-64 space-y-1 overflow-y-auto rounded-lg border border-border p-2">
                {orgServices.map((service) => {
                  const id = `member-scope-svc-${service.id}`;
                  const isChecked = selectedIds.includes(service.id);
                  return (
                    <div
                      key={service.id}
                      className="flex items-center gap-3 rounded px-2 py-1.5 hover:bg-accent/40"
                    >
                      <Checkbox
                        id={id}
                        checked={isChecked}
                        onCheckedChange={() => toggleService(service.id)}
                      />
                      <ServiceIcon
                        slug={service.catalog_service_slug}
                        iconUrl={connectionIconUrl(service)}
                        size="2xs"
                      />
                      <Label
                        htmlFor={id}
                        className="flex-1 cursor-pointer space-y-0.5"
                      >
                        <span className="block text-foreground">
                          {service.label}
                        </span>
                        <FieldHelp className="block text-muted-foreground">
                          {service.slug}
                        </FieldHelp>
                      </Label>
                    </div>
                  );
                })}
              </div>
            )}
            {orgServices.length > 0 && (
              <p className="text-13 text-muted-foreground">{translateUi("Selecting nothing revokes proxy access to every org service.")}</p>
            )}
          </div>
        )}
      </div>

      <DialogFooter>
        <Button
          variant="outline"
          onClick={onClose}
          disabled={updateMutation.isPending}
        >{translateUi("Cancel")}</Button>
        <Button
          variant="primary"
          onClick={() => void handleSave()}
          isLoading={updateMutation.isPending}
        >{translateUi("Save")}</Button>
      </DialogFooter>
    </>
  );
}

function describeScope(
  allowedServiceIds: readonly string[] | null,
  services: readonly KeyInfo[],
): string {
  if (allowedServiceIds === null) {
    return translateUi("Full access");
  }
  if (allowedServiceIds.length === 0) {
    return translateUi("No services");
  }
  const labels = allowedServiceIds.map((id) => {
    const service = services.find((item) => item.id === id);
    return service?.label ?? id;
  });
  if (labels.length <= 3) {
    return labels.join(", ");
  }
  return `${labels.slice(0, 3).join(", ")} and ${String(labels.length - 3)} more`;
}
