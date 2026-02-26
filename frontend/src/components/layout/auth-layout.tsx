import { Link, Outlet } from "@tanstack/react-router";
import { PortalMarkLogo } from "@/components/shared/portal-mark-logo";

/* ── VoidPortal Auth Layout ── */
export function AuthLayout() {
  return (
    <div
      className="flex min-h-dvh flex-col items-center justify-center bg-background p-4"
      style={{
        paddingTop: "max(1rem, var(--sat))",
        paddingBottom: "max(1rem, var(--sab))",
        paddingLeft: "max(1rem, var(--sal))",
        paddingRight: "max(1rem, var(--sar))",
      }}
    >
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
        <div className="flex flex-col items-center gap-1.5">
          <p className="text-center text-[11px] text-text-tertiary">
            Secure identity and access management by NyxID
          </p>
          <Link
            to="/privacy"
            className="text-[11px] text-violet-400 underline-offset-2 hover:text-violet-300 hover:underline"
          >
            Privacy Policy
          </Link>
        </div>
      </div>
    </div>
  );
}
