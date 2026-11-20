import { useId, useState } from "react";
import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";
import {
  useDeleteServiceApprovalConfig,
  useSetServiceApprovalConfig,
  useApprovalAuthoringCapabilities,
} from "@/hooks/use-approvals";
import { ApprovalPolicyEditor } from "@/components/shared/approval-policy-editor";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogBody,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ErrorBanner } from "@/components/shared/error-banner";
import type {
  ServiceApprovalConfigItem,
  ApprovalMode,
  ApprovalEffect,
  ApprovalRule,
} from "@/types/approvals";

/** Stored policies are edited as ordered lists. Opening or saving never compiles
 * them through the action wizard, including policies authored by older clients. */
export function StoredApprovalPolicyDialog({
  config,
  apiKeys,
  onClose,
}: {
  readonly config: ServiceApprovalConfigItem;
  readonly apiKeys: readonly { readonly id: string; readonly name: string }[];
  readonly onClose: () => void;
}) {
  useUiTranslation();
  const id = useId();
  const save = useSetServiceApprovalConfig();
  const remove = useDeleteServiceApprovalConfig();
  const capabilities = useApprovalAuthoringCapabilities();
  const [mode, setMode] = useState(config.approval_mode);
  const [required, setRequired] = useState(config.approval_required);
  const [days, setDays] = useState(config.grant_expiry_days?.toString() ?? "");
  const [disabled, setDisabled] = useState(config.disabled ?? false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const ready =
    capabilities.data?.personal_rules === true &&
    !capabilities.error &&
    Boolean(config.revision);
  const busy = save.isPending || remove.isPending;
  const validDays =
    days === "" ||
    (/^\d+$/.test(days) && Number(days) >= 1 && Number(days) <= 365);
  const error = save.error ?? remove.error;
  async function persist(
    rules: readonly ApprovalRule[],
    fallback: ApprovalEffect | null,
  ) {
    if (!ready || !validDays) return;
    try {
      await save.mutateAsync({
        serviceId: config.service_id,
        approvalRequired: required,
        approvalMode: mode,
        rules,
        defaultEffect: fallback,
        grantExpiryDays: days === "" ? null : Number(days),
        disabled,
        expectedRevision: config.revision,
      });
      onClose();
    } catch {
      /* The mutation error remains visible; preserve the draft. */
    }
  }
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open && !busy) onClose();
      }}
    >
      <DialogContent className="bg-background sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{config.service_name}</DialogTitle>
          <DialogDescription>
            {translateUi(
              "Rules are checked in order. The first matching rule decides; unmatched requests use the saved fallback.",
            )}
          </DialogDescription>
        </DialogHeader>
        <DialogBody className="space-y-4">
          {error && (
            <ErrorBanner
              message={
                error instanceof Error
                  ? error.message
                  : translateUi("Failed to save the protection")
              }
            />
          )}
          {!ready && (
            <p className="type-secondary text-text-tertiary">
              {translateUi(
                "Policy editing is available after every server supports approval rules. Reload to check again.",
              )}
            </p>
          )}
          <Label className="flex items-center justify-between">
            {translateUi("Enabled")}
            <Switch
              checked={!disabled}
              disabled={busy || !ready}
              onCheckedChange={(enabled) => setDisabled(!enabled)}
            />
          </Label>
          <Label className="flex items-center justify-between">
            {translateUi("Legacy approval required")}
            <Switch
              checked={required}
              disabled={busy || !ready}
              onCheckedChange={setRequired}
            />
          </Label>
          <Label htmlFor={`${id}-mode`}>{translateUi("Approval mode")}</Label>
          <Select
            value={mode}
            disabled={busy || !ready}
            onValueChange={(value) => setMode(value as ApprovalMode)}
          >
            <SelectTrigger id={`${id}-mode`}>
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="per_request">
                {translateUi("Per request")}
              </SelectItem>
              <SelectItem value="grant">
                {translateUi("Time-based grant")}
              </SelectItem>
            </SelectContent>
          </Select>
          <Label htmlFor={`${id}-days`}>
            {translateUi("Remembered approval days")}
          </Label>
          <Input
            id={`${id}-days`}
            type="number"
            min={1}
            max={365}
            value={days}
            placeholder={translateUi("Use account default")}
            disabled={busy || !ready}
            onChange={(event) => setDays(event.target.value)}
          />
          <p className="type-secondary text-text-tertiary">
            {translateUi(
              "Leave empty to use the account default. Existing remembered approvals keep their expiry.",
            )}
          </p>
          {config.rules.some((rule) => (rule.api_key_ids?.length ?? 0) > 0) && (
            <p className="type-secondary text-text-tertiary">
              {translateUi(
                "The live assistant turn's own stateless model call keeps its existing approval exemption.",
              )}
            </p>
          )}
          <ApprovalPolicyEditor
            exact
            approvalMode={mode}
            defaultEffect={config.default_effect}
            rules={config.rules}
            apiKeys={apiKeys}
            disabled={busy || !ready || !validDays}
            onSave={() => undefined}
            onSaveExact={(rules, fallback) => {
              void persist(rules, fallback);
            }}
          />
          {confirmRemove ? (
            <div className="flex flex-wrap gap-2">
              <Button
                variant="destructive"
                disabled={busy || !ready}
                onClick={() => {
                  void remove
                    .mutateAsync({
                      serviceId: config.service_id,
                      expectedRevision: config.revision,
                    })
                    .then(onClose)
                    .catch(() => undefined);
                }}
              >
                {translateUi("Confirm removal")}
              </Button>
              <Button
                variant="outline"
                disabled={busy}
                onClick={() => setConfirmRemove(false)}
              >
                {translateUi("Cancel")}
              </Button>
            </div>
          ) : (
            <Button
              variant="outline"
              disabled={busy || !ready}
              onClick={() => setConfirmRemove(true)}
            >
              {translateUi("Remove protection")}
            </Button>
          )}
        </DialogBody>
      </DialogContent>
    </Dialog>
  );
}
