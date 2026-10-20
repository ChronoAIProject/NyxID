import { useParams } from "@tanstack/react-router";
import { DeveloperAppDetail } from "@/components/developer-apps/developer-app-detail";
import { useOrg } from "@/hooks/use-orgs";
import { useBreadcrumbLabel } from "@/components/layout/breadcrumb-context";

export function OrgDeveloperAppDetailPage() {
  const { orgId, clientId } = useParams({
    from: "/dashboard/orgs/$orgId/developer-apps/$clientId",
  });
  const { data: org } = useOrg(orgId);
  const orgLabel = org?.display_name ?? "Organization";
  const orgPath = `/orgs/${orgId}`;
  useBreadcrumbLabel(org?.display_name, orgPath);

  return (
    <DeveloperAppDetail
      clientId={clientId}
      backTo={{ to: orgPath, label: orgLabel, search: { tab: "developer-apps" } }}
    />
  );
}
