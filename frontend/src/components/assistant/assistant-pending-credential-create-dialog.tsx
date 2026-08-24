import { useRef, useState } from "react";
import { KeyRound } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api } from "@/lib/api-client";
import { nodeCredentialActionParamsSchema } from "@/schemas/assistant-actions";
import {
  assertNoSensitiveActionParams,
  errorMessage,
  SECRET_VALUE_PATTERN,
} from "./assistant-action-dialog-utils";
import {
  assistantPendingCredentialEffectResponseSchema,
  readNodeAuthorization,
  readPendingCredentialAuthorization,
} from "./assistant-node-action-shared";

export interface AssistantPendingCredentialCreateParams {
  readonly nodeId: string;
  readonly serviceSlug: string;
  readonly injectionMethod: "header" | "query-param" | "path-prefix";
  readonly fieldName: string;
  readonly targetUrl?: string;
  readonly label?: string;
}

export function AssistantPendingCredentialCreateDialog({
  open,
  onOpenChange,
  actionRequestId,
  params,
  mode,
  onComplete,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly actionRequestId: string;
  readonly params: AssistantPendingCredentialCreateParams;
  readonly mode: "inject" | "push";
  readonly onComplete: (pendingCredentialId: string) => void;
}) {
  const submittingRef = useRef(false);
  const [serviceSlug, setServiceSlug] = useState(params.serviceSlug);
  const [injectionMethod, setInjectionMethod] = useState(
    params.injectionMethod,
  );
  const [fieldName, setFieldName] = useState(params.fieldName);
  const [targetUrl, setTargetUrl] = useState(params.targetUrl ?? "");
  const [label, setLabel] = useState(params.label ?? "");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [resultId, setResultId] = useState<string | null>(null);

  function close() {
    submittingRef.current = false;
    setSubmitting(false);
    setError(null);
    setResultId(null);
    onOpenChange(false);
  }

  async function submit() {
    if (submittingRef.current || resultId) return;
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      assertNoSensitiveActionParams(params);
      if (SECRET_VALUE_PATTERN.test(label)) {
        throw new Error("Labels cannot contain secret-shaped values.");
      }
      const reviewed = nodeCredentialActionParamsSchema.parse({
        nodeId: params.nodeId,
        serviceSlug,
        injectionMethod,
        fieldName,
        ...(targetUrl.trim() ? { targetUrl: targetUrl.trim() } : {}),
        ...(label.trim() ? { label: label.trim() } : {}),
      });
      const node = await readNodeAuthorization(reviewed.nodeId);
      if (
        node.id !== reviewed.nodeId ||
        !node.is_active ||
        node.lifecycle !== "active"
      ) {
        throw new Error("NyxID could not verify the active destination node.");
      }
      const path =
        mode === "inject"
          ? "/assistant/actions/nodes/inject-credential"
          : "/assistant/actions/nodes/pending-credential-push";
      const response = assistantPendingCredentialEffectResponseSchema.parse(
        await api.post<unknown>(path, {
          actionRequestId,
          nodeId: reviewed.nodeId,
          serviceSlug: reviewed.serviceSlug,
          injectionMethod: reviewed.injectionMethod,
          fieldName: reviewed.fieldName,
          targetUrl: reviewed.targetUrl,
          label: reviewed.label,
        }),
      );
      const evidence = await readPendingCredentialAuthorization(
        reviewed.nodeId,
        response.resource.pendingCredentialId,
      );
      if (
        evidence.id !== response.resource.pendingCredentialId ||
        evidence.node_id !== reviewed.nodeId ||
        !evidence.is_active ||
        evidence.consumed_at !== null ||
        evidence.declined_at !== null
      ) {
        throw new Error(
          "NyxID could not verify the pending credential request.",
        );
      }
      setResultId(evidence.id);
    } catch (caught) {
      setError(
        errorMessage(
          caught,
          "NyxID could not create this pending credential request.",
        ),
      );
    } finally {
      submittingRef.current = false;
      setSubmitting(false);
    }
  }

  const title =
    mode === "inject" ? "Inject node credential" : "Push pending credential";
  return (
    <Dialog open={open} onOpenChange={(next) => !next && close()}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <KeyRound className="size-4" />
            {resultId ? "Pending credential confirmed" : title}
          </DialogTitle>
          <DialogDescription>
            {resultId
              ? "The canonical pending-credential projection confirms the active request."
              : "Review the injection shape. Credential material is encrypted through the node flow and is never accepted from chat."}
          </DialogDescription>
        </DialogHeader>

        {resultId ? (
          <p className="break-all border-y border-border py-4 font-mono text-[12px] text-muted-foreground">
            {resultId}
          </p>
        ) : (
          <div className="space-y-4 border-y border-border py-4">
            <div className="flex items-center justify-between gap-4 text-[12px]">
              <span className="text-muted-foreground">Node</span>
              <Badge
                variant="secondary"
                className="max-w-[70%] truncate font-mono"
              >
                {params.nodeId}
              </Badge>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div className="space-y-1.5">
                <Label htmlFor={`${mode}-service-slug`}>Service slug</Label>
                <Input
                  id={`${mode}-service-slug`}
                  value={serviceSlug}
                  onChange={(event) => setServiceSlug(event.target.value)}
                />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor={`${mode}-injection-method`}>
                  Injection method
                </Label>
                <Select
                  value={injectionMethod}
                  onValueChange={(value) =>
                    setInjectionMethod(value as typeof injectionMethod)
                  }
                >
                  <SelectTrigger id={`${mode}-injection-method`}>
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="header">Header</SelectItem>
                    <SelectItem value="query-param">Query parameter</SelectItem>
                    <SelectItem value="path-prefix">Path prefix</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor={`${mode}-field-name`}>Field name</Label>
              <Input
                id={`${mode}-field-name`}
                value={fieldName}
                onChange={(event) => setFieldName(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor={`${mode}-target-url`}>
                Target URL (optional)
              </Label>
              <Input
                id={`${mode}-target-url`}
                type="url"
                value={targetUrl}
                onChange={(event) => setTargetUrl(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor={`${mode}-label`}>Label (optional)</Label>
              <Input
                id={`${mode}-label`}
                value={label}
                onChange={(event) => setLabel(event.target.value)}
              />
            </div>
          </div>
        )}

        {error ? (
          <p role="alert" className="text-[11px] text-destructive">
            {error}
          </p>
        ) : null}
        <DialogFooter>
          {resultId ? (
            <Button
              type="button"
              variant="primary"
              onClick={() => {
                onComplete(resultId);
                close();
              }}
            >
              Done
            </Button>
          ) : (
            <>
              <Button type="button" variant="outline" onClick={close}>
                Cancel
              </Button>
              <Button
                type="button"
                variant="primary"
                isLoading={submitting}
                disabled={
                  submitting || !serviceSlug.trim() || !fieldName.trim()
                }
                onClick={() => void submit()}
              >
                {mode === "inject" ? "Inject credential" : "Push credential"}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
