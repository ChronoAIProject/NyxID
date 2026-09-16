import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { ChevronDown, ChevronRight, RefreshCw, TestTube2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription, DialogFooter } from "@/components/ui/dialog";
import { CopyableField } from "@/components/shared/copyable-field";
import { useCodexConnection, useVerifyCodexConnection } from "@/hooks/use-codex-connection";
import type { CodexConnection } from "@/schemas/codex-connection";

const labels = { not_connected: "Not connected", saved: "Saved, verification pending", usable: "Usable", reconnect_required: "Reconnect required" } as const;

export function CodexConnectionSection() {
  const [open, setOpen] = useState(false);
  const [reviewed, setReviewed] = useState<CodexConnection | null>(null);
  const query = useCodexConnection(open);
  const verification = useVerifyCodexConnection();
  const connection = query.data;
  const status = connection?.status;

  return (
    <section className="mb-6 border-b border-border/50 pb-4">
      <button type="button" onClick={() => setOpen(!open)} aria-expanded={open}
        className="flex min-h-8 items-center gap-2 text-[13px] font-medium">
        {open ? <ChevronDown className="size-3" /> : <ChevronRight className="size-3" />}Codex connection
      </button>
      {open && <div className="mt-3 space-y-4">
        <div className="flex flex-wrap items-center gap-3">
          <span className="text-xs text-muted-foreground">OpenAI Responses</span>
          {status && <Badge variant={status === "usable" ? "success" : status === "reconnect_required" ? "warning" : "secondary"}>{labels[status]}</Badge>}
          <Button variant="ghost" size="icon" title="Refresh connection status" aria-label="Refresh connection status"
            disabled={query.isFetching || verification.isPending} onClick={() => { verification.reset(); void query.refetch(); }}>
            <RefreshCw className="size-3" />
          </Button>
          {connection?.service_id && <Link to="/keys/$keyId" params={{ keyId: connection.service_id }} className="text-xs text-primary hover:underline">Manage service</Link>}
        </div>
        {query.isPending && <p role="status" className="text-xs text-muted-foreground">Checking connection...</p>}
        {(query.isError || verification.isError) && <p role="alert" className="text-xs text-destructive">Connection verification is unavailable. Retry or use separate authorization.</p>}
        <CopyableField label="Local credential connection" value="nyxid provider connect-codex" size="sm" />
        <p className="text-xs text-muted-foreground">API-key file import requires approval of the NyxID instance and account. ChatGPT OAuth and OS credential storage use separate authorization. OpenAI API charges apply.</p>
        <div className="flex flex-wrap gap-2">
          <Button size="sm" disabled={!connection?.connection || verification.isPending || query.isFetching}
            onClick={() => setReviewed(connection ?? null)} isLoading={verification.isPending}><TestTube2 className="size-3" />Verify connection</Button>
          <Link to="/keys" search={{ slug: "llm-openai-codex" }} className="inline-flex h-7 items-center px-2 text-xs text-primary hover:underline">Authorize Codex separately</Link>
        </div>
        <p className="text-xs text-text-tertiary">Disable pauses the service. Delete removes its NyxID credential and endpoint; it does not revoke the upstream API key or change local Codex.</p>
      </div>}
      <Dialog open={reviewed !== null} onOpenChange={(value) => { if (!value) setReviewed(null); }}>
        <DialogContent>
          <DialogHeader><DialogTitle>Verify OpenAI connection</DialogTitle>
            <DialogDescription>A small paid Responses request will use the saved connection for {reviewed?.account_email}. No local credentials are read.</DialogDescription></DialogHeader>
          <DialogFooter><Button variant="outline" onClick={() => setReviewed(null)}>Cancel</Button>
            <Button variant="primary" disabled={!reviewed?.connection} onClick={() => {
              setReviewed(null);
              if (reviewed?.connection) verification.mutate(reviewed);
            }}><TestTube2 className="size-3" />Verify</Button></DialogFooter>
        </DialogContent>
      </Dialog>
    </section>
  );
}
