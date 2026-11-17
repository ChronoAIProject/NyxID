import { useEffect, useState } from "react";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import { useAuthStore } from "@/stores/auth-store";

/** Remains visible outside hidden route/portal content; fast checks avoid a flash. */
export function AccountVerificationStatus() {
  useUiTranslation();
  const verifying = useAuthStore((state) => state.isIdentityVerifying);
  const [delayed, setDelayed] = useState(false);
  useEffect(() => {
    if (!verifying) return;
    const timer = setTimeout(() => setDelayed(true), 300);
    return () => {
      clearTimeout(timer);
      setDelayed(false);
    };
  }, [verifying]);
  if (!verifying || !delayed) return null;
  const message = translateUi("Checking which account is signed in…");
  return (
    <div
      role="status"
      aria-live="polite"
      aria-atomic="true"
      aria-label={message}
      className="visible fixed inset-0 z-[1000] flex items-center justify-center bg-background p-6 type-body text-muted-foreground"
    >
      {message}
    </div>
  );
}
