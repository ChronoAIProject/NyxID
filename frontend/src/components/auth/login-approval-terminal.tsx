import { useState } from "react";
import { CheckCircle2, LogOut, ShieldX } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { agentKeyErrorMessage } from "@/schemas/agent-key-login";
import { useAuthStore } from "@/stores/auth-store";

export function LoginApprovalTerminal({
  terminal,
  error,
  onError,
}: {
  terminal: "approved" | "denied" | "expired" | null;
  error: string | null;
  onError: (message: string) => void;
}) {
  const { isAuthenticated, logout } = useAuthStore();
  const [signingOut, setSigningOut] = useState(false);
  return (
    <section className="space-y-4 py-4 text-center" aria-live="polite">
      {terminal === "approved" ? (
        <CheckCircle2 className="mx-auto size-8 text-success" />
      ) : (
        <ShieldX className="mx-auto size-8 text-destructive" />
      )}
      <h2 className="text-[15px] font-semibold">
        {terminal === "approved"
          ? "Approved - return to the requesting device"
          : terminal === "denied"
            ? "Login rejected"
            : "Login request expired"}
      </h2>
      {isAuthenticated && terminal === "approved" && (
        <Button
          variant="outline"
          isLoading={signingOut}
          onClick={() => {
            setSigningOut(true);
            void logout()
              .catch((failure: unknown) =>
                onError(agentKeyErrorMessage(failure)),
              )
              .finally(() => setSigningOut(false));
          }}
        >
          <LogOut className="size-3" />
          Sign out of this browser
        </Button>
      )}
      {error && <ErrorBanner message={error} />}
    </section>
  );
}
