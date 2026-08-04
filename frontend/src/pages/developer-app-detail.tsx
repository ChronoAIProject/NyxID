import { useParams } from "@tanstack/react-router";
import { DeveloperAppDetail } from "@/components/developer-apps/developer-app-detail";

export function DeveloperAppDetailPage() {
  const { clientId } = useParams({
    from: "/dashboard/developer/apps/$clientId",
  });

  return (
    <DeveloperAppDetail
      clientId={clientId}
      backTo={{ to: "/developer/apps", label: "Developer Apps" }}
    />
  );
}
