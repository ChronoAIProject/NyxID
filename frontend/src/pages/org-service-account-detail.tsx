import { useParams } from "@tanstack/react-router";
import { ServiceAccountDetail } from "@/components/service-accounts/service-account-detail";
import { useOrg } from "@/hooks/use-orgs";

export function OrgServiceAccountDetailPage() {
  const { orgId, saId } = useParams({
    from: "/dashboard/orgs/$orgId/service-accounts/$saId",
  });
  const { data: org } = useOrg(orgId);
  const orgLabel = org?.display_name ?? "Organization";
  const orgPath = `/orgs/${orgId}`;
  // Org admins (NOT members or viewers) can write to org-owned service
  // accounts; the backend authorizes this via
  // `require_admin_or_owning_org_admin`. The org detail response carries
  // `your_role` for the current user, so we don't need a separate
  // membership lookup.
  const canWrite = org?.your_role === "admin";

  return (
    <ServiceAccountDetail
      saId={saId}
      backTo={{ to: orgPath, label: orgLabel }}
      showProviderSections={false}
      canWrite={canWrite}
    />
  );
}
