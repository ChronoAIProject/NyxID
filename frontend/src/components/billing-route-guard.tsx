import { useEffect } from "react";
import type { ReactNode } from "react";
import { billingCheckoutReturn, consumeBillingCheckoutReturn } from "@/lib/billing-checkout-return";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { shouldRedirectFromBilling } from "@/lib/billing-availability";
import { useAuthStore } from "@/stores/auth-store";

export function BillingRouteGuard({
  children,
}: {
  readonly children: ReactNode;
}) {
  const navigate = useNavigate();
  const search = useSearch({ strict: false });
  const isLoading = useAuthStore((s) => s.isLoading);
  const user = useAuthStore((s) => s.user);
  const shouldRedirect = shouldRedirectFromBilling({ isLoading, user });
  const checkoutReturn = billingCheckoutReturn(user?.id, search);

  useEffect(() => {
    if (isLoading || !user || shouldRedirect) return;
    const destination = consumeBillingCheckoutReturn(user.id, search);
    if (destination) void navigate({ ...destination, replace: true });
  }, [isLoading, user, shouldRedirect, search, navigate]);

  useEffect(() => {
    if (shouldRedirect) {
      void navigate({ to: "/dashboard", replace: true });
    }
  }, [navigate, shouldRedirect]);

  if (isLoading || shouldRedirect || checkoutReturn) {
    return null;
  }

  return <>{children}</>;
}
