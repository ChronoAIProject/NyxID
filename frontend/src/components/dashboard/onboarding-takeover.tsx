import { useState } from "react";
import { toast } from "sonner";
import { ArrowRight, Cable, KeyRound, ShieldCheck } from "lucide-react";
import { useAuthStore } from "@/stores/auth-store";
import { useCompleteOnboarding } from "@/hooks/use-onboarding";
import { AddKeyDialog } from "@/components/dashboard/add-key-dialog";
import { Button, ButtonIcon } from "@/components/ui/button";

/**
 * First-run AI-services onboarding. Rendered by `DashboardLayout` in place
 * of the normal dashboard chrome when `useShouldShowOnboarding` reports the
 * user hasn't completed it — no separate route, the wizard wraps over the
 * dashboard. Both actions stamp the server-side flag and refresh the
 * auth-store user, which makes the layout swap back to the real dashboard.
 */
export function OnboardingTakeover() {
  const user = useAuthStore((s) => s.user);
  const userName = user?.display_name ?? "there";
  const [addServiceOpen, setAddServiceOpen] = useState(false);
  const completeOnboarding = useCompleteOnboarding();

  /**
   * Stamp the server-side flag and refresh the auth-store user. Once the
   * user refreshes, `useShouldShowOnboarding` flips to `hidden` and
   * `DashboardLayout` unmounts this takeover for the real dashboard. On
   * failure we keep the user here to retry rather than stranding them.
   */
  async function markComplete() {
    try {
      await completeOnboarding.mutateAsync({ key: "ai_services" });
      await useAuthStore.getState().checkAuth();
    } catch {
      toast.error("Couldn't save your progress. Please try again.");
    }
  }

  function handleDialogChange(open: boolean) {
    setAddServiceOpen(open);
    // Dialog dismissed (service added or not) — mark onboarding done so the
    // layout drops the takeover and shows the real dashboard.
    if (!open) void markComplete();
  }

  return (
    <div className="flex min-h-dvh flex-col items-center justify-start bg-background px-6 pt-[12vh] pb-12">
      <div className="flex w-full max-w-md flex-col items-center gap-8 text-center">
        {/* Brand mark */}
        <div className="flex h-16 w-16 items-center justify-center rounded-2xl border border-nyx-500/30 bg-nyx-500/10">
          <Cable className="h-7 w-7 text-nyx-secondary-400" />
        </div>

        {/* Copy */}
        <div className="space-y-3">
          <h1
            className="text-[28px] font-bold leading-[1.1] text-foreground"
            style={{ letterSpacing: "-0.03em" }}
          >
            Welcome, {userName}
          </h1>
          <p className="text-[14px] leading-relaxed text-muted-foreground">
            Connect your first AI service to get started with NyxID. Your
            agents will proxy requests through NyxID so credentials never
            leave your control.
          </p>
        </div>

        {/* CTA */}
        <Button
          variant="primary"
          size="lg"
          className="w-full max-w-xs"
          onClick={() => setAddServiceOpen(true)}
        >
          <ButtonIcon variant="primary">
            <ArrowRight className="h-4 w-4" />
          </ButtonIcon>
          Connect a Service
        </Button>

        {/* Skip */}
        <button
          type="button"
          onClick={() => void markComplete()}
          disabled={completeOnboarding.isPending}
          className="text-[12px] text-text-tertiary transition-colors duration-200 hover:text-foreground disabled:opacity-50"
        >
          Skip for now
        </button>

        {/* Trust signals */}
        <div className="flex items-center gap-6 text-[11px] text-text-tertiary">
          <span className="flex items-center gap-1.5">
            <ShieldCheck className="h-3.5 w-3.5" />
            End-to-end encrypted
          </span>
          <span className="flex items-center gap-1.5">
            <KeyRound className="h-3.5 w-3.5" />
            Zero credential exposure
          </span>
        </div>
      </div>

      <AddKeyDialog open={addServiceOpen} onOpenChange={handleDialogChange} />
    </div>
  );
}
