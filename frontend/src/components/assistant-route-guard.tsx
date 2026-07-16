import { useEffect } from "react";
import type { ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";
import { shouldRedirectFromAssistant } from "@/lib/assistant-availability";
import { useAuthStore } from "@/stores/auth-store";

export function AssistantRouteGuard({
  children,
}: {
  readonly children: ReactNode;
}) {
  const navigate = useNavigate();
  const isLoading = useAuthStore((s) => s.isLoading);
  const user = useAuthStore((s) => s.user);
  const shouldRedirect = shouldRedirectFromAssistant({ isLoading, user });

  useEffect(() => {
    if (shouldRedirect) {
      void navigate({ to: "/dashboard", replace: true });
    }
  }, [navigate, shouldRedirect]);

  if (isLoading || shouldRedirect) {
    return null;
  }

  return <>{children}</>;
}
