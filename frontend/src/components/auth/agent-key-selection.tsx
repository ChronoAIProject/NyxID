import { useState } from "react";
import { ArrowLeft, KeyRound, Plus, ShieldCheck, ShieldX } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { AgentKeyCreateForm } from "@/components/auth/agent-key-create-form";
import {
  AgentKeyPermissions,
  AgentKeyIssuanceNotice,
} from "@/components/auth/agent-key-permissions";
import {
  newKeySelection,
  type AgentKeyApprove,
  type AgentKeyOptions,
  type AgentKeySummary,
} from "@/schemas/agent-key-login";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";

function localDateTime(value: string): string {
  const date = new Date(value);
  return new Date(date.getTime() - date.getTimezoneOffset() * 60000)
    .toISOString()
    .slice(0, 16);
}

export function AgentKeySelection({
  step,
  options,
  label,
  ownerId,
  ownerName,
  newKeyDraft,
  onDraftChange,
  pending,
  expired,
  throttled,
  onError,
  onStepChange,
  onReject,
  onConfirm,
  rejectLoading,
  confirmLoading,
  rejectLabel,
  confirmLabel,
}: {
  step: string;
  options: AgentKeyOptions | undefined;
  label: string;
  ownerId: string;
  ownerName: string;
  newKeyDraft: CreateApiKeyFormData | undefined;
  onDraftChange: (data: CreateApiKeyFormData) => void;
  pending: boolean;
  expired: boolean;
  throttled: () => boolean;
  onError: (message: string | null) => void;
  onStepChange: (step: "options" | "confirm") => void;
  onReject: () => void;
  onConfirm: (
    selection: AgentKeyApprove["selection"],
    credentialExpiry: string,
  ) => void;
  rejectLoading: boolean;
  confirmLoading: boolean;
  rejectLabel: string;
  confirmLabel: string;
}) {
  const [choice, setChoice] = useState<string>("");
  const [selection, setSelection] = useState<
    AgentKeyApprove["selection"] | null
  >(null);
  const [summary, setSummary] = useState<AgentKeySummary | null>(null);
  const [credentialExpiry, setCredentialExpiry] = useState("");
  function reviewExisting() {
    if (expired || throttled()) return;
    const key = options?.keys.find((key) => key.id === choice);
    if (!key) return;
    setSelection({ kind: "existing", api_key_id: key.id });
    setSummary(key);
    onStepChange("confirm");
  }
  function reviewNew(data: CreateApiKeyFormData) {
    if (!options || expired || throttled()) return;
    const result = newKeySelection(data);
    if (!result.success) {
      onError(result.error.issues[0]?.message ?? "Check the key details.");
      return;
    }
    onError(null);
    const selected = result.data;
    onDraftChange(data);
    const org = options.orgs.find((org) => org.id === selected.target_org_id);
    setSelection(selected);
    setSummary({
      id: "",
      name: selected.name,
      key_prefix: "",
      owner_type: org ? "org" : "personal",
      owner_id: org?.id ?? ownerId,
      owner_name: org?.name ?? ownerName,
      scopes: selected.scopes,
      allow_all_services: selected.allow_all_services,
      allow_all_nodes: selected.allow_all_nodes,
      allowed_service_ids: selected.allowed_service_ids,
      allowed_node_ids: selected.allowed_node_ids,
      allowed_services: options.services.filter((item) =>
        selected.allowed_service_ids.includes(item.id),
      ),
      allowed_nodes: options.nodes.filter((item) =>
        selected.allowed_node_ids.includes(item.id),
      ),
      expires_at: selected.expires_at || null,
      rate_limit_per_second: selected.rate_limit_per_second ?? null,
      rate_limit_burst: selected.rate_limit_burst ?? null,
      platform: selected.platform ?? null,
      created_now: true,
    });
    onStepChange("confirm");
  }
  return (
    <>
      {step === "options" && options && (
        <section className="space-y-4">
          <h2 className="text-[15px] font-semibold">Choose an Agent Key</h2>
          <fieldset disabled={pending || expired} className="space-y-3">
            {options.keys.map((key) => (
              <div
                key={key.id}
                className="flex items-start gap-3 rounded-lg border border-border p-3"
              >
                <input
                  type="radio"
                  name="agent-key-choice"
                  value={key.id}
                  checked={choice === key.id}
                  onChange={() => setChoice(key.id)}
                  aria-label={key.name}
                  className="mt-1 shrink-0"
                />
                <div className="min-w-0 flex-1">
                  <AgentKeyPermissions apiKey={key} />
                </div>
              </div>
            ))}
            <label className="flex items-center gap-2 text-[12px]">
              <input
                type="radio"
                name="agent-key-choice"
                value="new"
                checked={choice === "new"}
                onChange={() => setChoice("new")}
              />
              <Plus className="size-3" />
              Create a new key
            </label>
          </fieldset>
          {choice === "new" ? (
            <AgentKeyCreateForm
              options={options}
              label={label}
              initialValues={newKeyDraft}
              disabled={pending || expired}
              onReview={reviewNew}
            />
          ) : (
            <Button
              variant="primary"
              disabled={!choice || pending || expired}
              onClick={reviewExisting}
            >
              <KeyRound className="size-3" />
              Review permissions
            </Button>
          )}
        </section>
      )}
      {step === "confirm" && summary && (
        <section className="space-y-4 border-t border-border/50 pt-4">
          <h2 className="text-[15px] font-semibold">
            Confirm effective permissions
          </h2>
          <AgentKeyPermissions apiKey={summary} />
          <AgentKeyIssuanceNotice existing={selection?.kind === "existing"} />
          <div className="space-y-2">
            <label htmlFor="credential-expiry" className="text-[12px]">
              Login credential expiry (optional)
            </label>
            <Input
              id="credential-expiry"
              type="datetime-local"
              value={credentialExpiry}
              disabled={pending || expired}
              max={
                summary.expires_at
                  ? localDateTime(summary.expires_at)
                  : undefined
              }
              onChange={(event) => setCredentialExpiry(event.target.value)}
            />
          </div>
          <div className="flex flex-wrap justify-end gap-2">
            <Button disabled={pending} onClick={() => onStepChange("options")}>
              <ArrowLeft className="size-3" />
              Back
            </Button>
            <Button
              variant="destructive"
              disabled={pending || expired}
              isLoading={rejectLoading}
              onClick={onReject}
            >
              <ShieldX className="size-3" />
              {rejectLabel}
            </Button>
            <Button
              className="border-success/30 bg-success/10 text-success hover:bg-success/20"
              disabled={pending || expired}
              isLoading={confirmLoading}
              onClick={() => {
                if (selection) onConfirm(selection, credentialExpiry);
              }}
            >
              <ShieldCheck className="size-3" />
              {confirmLabel}
            </Button>
          </div>
        </section>
      )}
    </>
  );
}
