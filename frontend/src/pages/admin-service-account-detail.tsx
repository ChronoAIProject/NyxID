import { useParams } from "@tanstack/react-router";
import { ServiceAccountDetail } from "@/components/service-accounts/service-account-detail";

export function AdminServiceAccountDetailPage() {
  const { saId } = useParams({
    from: "/dashboard/admin/service-accounts/$saId",
  });

  return (
    <ServiceAccountDetail
      saId={saId}
      backTo={{ to: "/admin/service-accounts", label: "Service Accounts" }}
      showProviderSections
    />
  );
}
