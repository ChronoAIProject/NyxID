import { Outlet } from "@tanstack/react-router";
import { PortalMarkLogo } from "@/components/shared/portal-mark-logo";

/* ── VoidPortal Auth Layout ── */
export function AuthLayout() {
  return (
    <div className="flex min-h-screen flex-col items-center justify-center bg-background p-4">
      <div className="flex w-full max-w-[420px] flex-col items-center gap-8">
        {/* ── Logo (Portal Mark + wordmark) ── */}
        <div className="flex items-center gap-3">
          <PortalMarkLogo size={36} className="shrink-0" />
          <span className="logo-wordmark text-[22px]">NyxID</span>
        </div>

        {/* ── Auth Card ── */}
        <div className="w-full rounded-[10px] border border-border bg-card p-8">
          <Outlet />
        </div>

        {/* ── Footer ── */}
        <p className="text-center text-[11px] text-text-tertiary">
          Secure identity and access management by NyxID
        </p>
      </div>
    </div>
  );
}
