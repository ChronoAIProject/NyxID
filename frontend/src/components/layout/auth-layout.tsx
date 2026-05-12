import { Link, Outlet } from "@tanstack/react-router";

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
        {/* Logo */}
        <div className="flex items-center">
          <img src="/nyxid-coloured-logo.svg" alt="NyxID" className="h-10 w-auto" />
        </div>

        {/* Auth Card */}
        <div className="w-full rounded-xl border border-border bg-card p-8">
          <Outlet />
        </div>

        {/* Footer */}
        <div className="flex flex-col items-center gap-1.5">
          <p className="text-center text-[11px] text-text-tertiary">
            Secure identity and access management by NyxID
          </p>
          <Link
            to="/privacy"
            className="text-[11px] text-nyx-secondary-400 underline-offset-2 hover:text-nyx-300 hover:underline"
          >
            Privacy Policy
          </Link>
        </div>
      </div>
    </div>
  );
}
