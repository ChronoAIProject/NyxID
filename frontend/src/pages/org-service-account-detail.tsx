import { useParams } from "@tanstack/react-router";
import { ServiceAccountDetail } from "@/components/service-accounts/service-account-detail";
import { useOrg } from "@/hooks/use-orgs";
import { useBreadcrumbLabel } from "@/components/layout/breadcrumb-context";

export function OrgServiceAccountDetailPage() {
  const { orgId, saId } = useParams({
    from: "/dashboard/orgs/$orgId/service-accounts/$saId",
  });
  const { data: org } = useOrg(orgId);
  const orgLabel = org?.display_name ?? "Organization";
  const orgPath = `/orgs/${orgId}`;
  useBreadcrumbLabel(org?.display_name, orgPath);
  return (
    <ServiceAccountDetail
      saId={saId}
      backTo={{ to: orgPath, label: orgLabel, search: { tab: "service-accounts" } }}
      showProviderSections={false}
      showKeyReadGrantSection
    />
  );
}
