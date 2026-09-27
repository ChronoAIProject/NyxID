import type { ReactNode } from "react";
import { useNavigate } from "@tanstack/react-router";
import { CreditCard, ShieldCheck } from "lucide-react";
import { DrainedCreditsIcon } from "@/components/icons/empty-state";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useOrgs } from "@/hooks/use-orgs";
import type { CreditsPayer } from "@/lib/credits-denial";

export interface CreditsDeniedDialogProps {
  readonly payer: CreditsPayer;
  /** Whether this deployment lets the signed-in person buy credits. */
  readonly billingAvailable: boolean;
  readonly onDismiss: () => void;
}

/**
 * Shown when NyxID's billing gate refused a foreground request. The gate can
 * also refuse a positive-but-insufficient balance, reservations or a wallet
 * without a payment instrument, so the copy never asserts one cause.
 */
export default function CreditsDeniedDialog({
  payer,
  billingAvailable,
  onDismiss,
}: CreditsDeniedDialogProps) {
  const navigate = useNavigate();
  const org = typeof payer === "object" ? payer.org : null;
  const orgs = useOrgs();
  const orgName =
    org &&
    (org.name ??
      orgs.data?.find((item) => item.id === org.id)?.display_name ??
      null);
  const canPurchase = billingAvailable && !org;

  function purchase() {
    onDismiss();
    void navigate({
      to: "/billing",
      search: { tab: "billing", action: "topup" },
    });
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onDismiss()}>
      <DialogContent className="md:max-w-[420px]">
        {/* Cropped to the art: the family viewBox leaves room below it. */}
        <DrainedCreditsIcon
          viewBox="4 -2 94 104"
          className="mx-auto mt-1 h-[112px] w-[101px] shrink-0 text-muted-foreground/80"
          data-testid="credits-denied-illustration"
        />
        <DialogHeader className="items-center space-y-2 text-center">
          <DialogTitle>Not enough credits to continue</DialogTitle>
          <DialogDescription className="max-w-[330px] text-[12px] leading-relaxed">
            {org
              ? `This request is billed to ${orgName ?? "your organization"}'s credits, which have been used up or have expired, so it couldn't run.`
              : "Your free platform credits have been used up or have expired, so this request couldn't run."}
          </DialogDescription>
        </DialogHeader>
        <ul className="space-y-2" aria-label="Ways to continue">
          <OptionRow
            icon={<ShieldCheck className="h-3.5 w-3.5" />}
            title="Ask an admin"
          >
            {org
              ? `Only an admin of ${orgName ?? "that organization"} can add credits to it.`
              : "An admin can add credits to your account."}
          </OptionRow>
          {canPurchase && (
            <OptionRow
              icon={<CreditCard className="h-3.5 w-3.5" />}
              title="Purchase platform credits"
            >
              Buy credits for your account and keep going.
            </OptionRow>
          )}
        </ul>
        {payer === "unknown" && (
          <p className="text-[11px] leading-relaxed text-muted-foreground">
            If this is billed to an organization, ask one of its admins instead.
          </p>
        )}
        <DialogFooter>
          {canPurchase ? (
            <>
              <Button variant="outline" onClick={onDismiss}>
                Not now
              </Button>
              <Button variant="primary" onClick={purchase}>
                Purchase credits
              </Button>
            </>
          ) : (
            <Button variant="primary" onClick={onDismiss}>
              Got it
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function OptionRow({
  icon,
  title,
  children,
}: {
  icon: ReactNode;
  title: string;
  children: ReactNode;
}) {
  return (
    <li className="flex items-start gap-3 rounded-lg border border-border/60 bg-white/[0.02] px-3 py-2.5">
      <span
        className="mt-px flex h-7 w-7 shrink-0 items-center justify-center rounded-md border border-white/[0.08] bg-white/[0.04] text-muted-foreground"
        aria-hidden="true"
      >
        {icon}
      </span>
      <span className="min-w-0 space-y-0.5">
        <span className="block text-[12px] font-medium text-foreground">
          {title}
        </span>
        <span className="block text-[11px] leading-relaxed text-muted-foreground">
          {children}
        </span>
      </span>
    </li>
  );
}
