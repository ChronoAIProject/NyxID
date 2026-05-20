import { useAuthStore } from "@/stores/auth-store";

/**
 * Where the NyxID logo should navigate: the dashboard when the user is signed
 * in, otherwise the public landing page. Used by every clickable logo so the
 * behavior stays consistent across the app, auth, blog, and legal pages.
 */
export function useLogoHref(): "/" | "/dashboard" {
  return useAuthStore((s) => s.isAuthenticated) ? "/dashboard" : "/";
}
