import { ArrowLeft, ShieldCheck, ShieldX } from "lucide-react";
import { Button } from "@/components/ui/button";

export function FullAccountConfirm({
  pending,
  expired,
  confirmLoading,
  onBack,
  onReject,
  onConfirm,
  confirmLabel,
}: {
  pending: boolean;
  expired: boolean;
  confirmLoading: boolean;
  onBack: () => void;
  onReject: () => void;
  onConfirm: () => void;
  confirmLabel: string;
}) {
  return (
    <section className="space-y-4 border-t border-border/50 pt-4">
      <h2 className="text-[15px] font-semibold">Confirm full account access</h2>
      <p className="text-[12px] text-muted-foreground">
        This machine receives an account session with access to your account,
        services, credentials, and organization permissions. The session can
        refresh until it expires or you revoke it in Settings.
      </p>
      <div className="flex flex-wrap justify-end gap-2">
        <Button disabled={pending} onClick={onBack}>
          <ArrowLeft className="size-3" /> Back
        </Button>
        <Button
          variant="destructive"
          disabled={pending || expired}
          onClick={onReject}
        >
          <ShieldX className="size-3" /> Reject
        </Button>
        <Button
          disabled={pending || expired}
          isLoading={confirmLoading}
          className="border-success/30 bg-success/10 text-success hover:bg-success/20"
          onClick={onConfirm}
        >
          <ShieldCheck className="size-3" /> {confirmLabel}
        </Button>
      </div>
    </section>
  );
}
