import { useState } from "react";
import { Copy } from "lucide-react";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useReplaceWhatsappRelayToken, type WhatsappRollbackToken } from "@/hooks/use-whatsapp-native";
import { copyToClipboard } from "@/lib/utils";

export function WhatsappRollbackTokenSection({ botId, revision }: { readonly botId: string; readonly revision: number }) {
  const [issued, setIssued] = useState<WhatsappRollbackToken | null>(null);
  const replace = useReplaceWhatsappRelayToken(botId, setIssued);
  function dismiss() {
    setIssued(null);
    replace.reset();
  }
  function copy(value: string, label: string) {
    void copyToClipboard(value).then(() => toast.success(`${label} copied`)).catch(() => toast.error("Could not copy"));
  }
  return <div className="space-y-3 border-t border-border p-5">
    <p role="status" className="text-12">Restore the NyxID relay callback in Meta with a new Verify Token, then verify the callback before the gateway can be released.</p>
    <p className="text-12 text-muted-foreground">Generating a replacement immediately invalidates the old relay Verify Token. The new token is shown once. This action does not move the bot or change Meta’s callback.</p>
    {replace.isError && <ErrorBanner message={replace.error.message} />}
    <div className="flex justify-end">
      <Button type="button" variant="primary" disabled={replace.isPending || issued !== null} isLoading={replace.isPending} onClick={() => replace.mutate(revision)}>Generate replacement Verify Token</Button>
    </div>
    <Dialog open={issued !== null} onOpenChange={(open) => { if (!open) dismiss(); }}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Save the new Verify Token</DialogTitle>
          <DialogDescription>This Verify Token is shown only once. Copy it now. The old relay Verify Token no longer verifies.</DialogDescription>
        </DialogHeader>
        {issued && <div className="space-y-4">
          <p className="text-12">Point Meta’s callback back to the relay Callback URL below with this new Verify Token.</p>
          {([["Relay Callback URL", issued.relay_webhook_url], ["New Verify Token", issued.verify_token]] as const).map(([label, value]) => <div className="space-y-2" key={label}>
            <p className="text-12 font-medium">{label}</p>
            <div className="flex items-center gap-2 rounded-lg border border-border bg-muted px-3 py-2">
              <code className="min-w-0 flex-1 break-all font-mono text-12">{value}</code>
              <Button type="button" variant="ghost" size="icon" className="h-7 w-7 shrink-0" aria-label={`Copy ${label}`} onClick={() => copy(value, label)}><Copy className="h-3.5 w-3.5" /></Button>
            </div>
          </div>)}
          <p className="text-12 text-muted-foreground">After closing this dialog, you cannot retrieve this token. If it is lost, generate another replacement; that immediately invalidates this one.</p>
        </div>}
        <DialogFooter><Button type="button" variant="primary" onClick={dismiss}>I have saved it</Button></DialogFooter>
      </DialogContent>
    </Dialog>
  </div>;
}
