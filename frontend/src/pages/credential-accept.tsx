import { useRef, useState } from "react";
import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { toast } from "sonner";
import { ShieldCheck, Terminal } from "lucide-react";
import { useNode } from "@/hooks/use-nodes";
import { ApiError, api } from "@/lib/api-client";
import { buildRciContext, encrypt } from "@/lib/crypto";
import { getSafeCredentialAcceptReturnTo } from "@/lib/return-url";
import { acceptNodeCredentialSecretSchema } from "@/schemas/nodes";
import { PageHeader } from "@/components/shared/page-header";
import { useBreadcrumbLabel } from "@/components/layout/dashboard-layout";
import { DetailSection } from "@/components/shared/detail-section";
import { ErrorBanner } from "@/components/shared/error-banner";
import { Badge } from "@/components/ui/badge";
import { Button, ButtonIcon } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import type { CiphertextEnvelope } from "@/lib/crypto";
import type {
  NodePendingCredentialCiphertextResponse,
  FanOutPendingCredentialCiphertextResponse,
  FanOutPendingCredentialPubkeysResponse,
  FanOutPendingCredentialResponse,
  NodePendingCredentialInfo,
  NodePendingCredentialPubkeyResponse,
  NodePendingCredentialRemoteState,
} from "@/types/nodes";
import type { FormEvent } from "react";

const PUBKEY_WAIT_MS = 30_000;
const POLL_WAIT_MS = 60_000;
const PUBKEY_DELAYS_MS = [500, 1_000, 2_000, 4_000, 8_000] as const;
const POLL_DELAYS_MS = [1_000, 2_000, 3_000, 5_000] as const;

type AcceptStatus =
  | "idle"
  | "waiting_pubkey"
  | "encrypting"
  | "posting"
  | "polling"
  | "consumed"
  | "partial_decrypted"
  | "decrypt_failed"
  | "expired"
  | "declined"
  | "timeout"
  | "legacy_fallback"
  | "error";

interface PendingCredentialListResponse {
  readonly pending_credentials: readonly PendingCredentialWithState[];
}

type PendingCredentialWithState = NodePendingCredentialInfo & {
  readonly remote_state?: NodePendingCredentialRemoteState | null;
};

const statusLabel: Readonly<Record<AcceptStatus, string>> = {
  idle: "Ready",
  waiting_pubkey: "Waiting for node key",
  encrypting: "Encrypting",
  posting: "Sending ciphertext",
  polling: "Waiting for node",
  consumed: "Stored",
  partial_decrypted: "Partially stored",
  decrypt_failed: "Decrypt failed",
  expired: "Expired",
  declined: "Declined",
  timeout: "Timed out",
  legacy_fallback: "Manual setup",
  error: "Error",
};

const terminalDescriptions: Partial<Readonly<Record<AcceptStatus, string>>> = {
  consumed: "The node consumed the encrypted credential.",
  partial_decrypted: "Some nodes stored the credential and some still need retry.",
  decrypt_failed: "The node could not decrypt the submitted envelope.",
  expired: "This pending credential expired before completion.",
  declined: "The node operator declined this pending credential.",
  timeout:
    "The node did not report completion before the browser stopped waiting.",
  legacy_fallback:
    "Use the node CLI on the machine that runs the agent to enter the credential locally.",
};

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms));
}

function nextDelay(delays: readonly number[], attempt: number): number {
  return delays[Math.min(attempt, delays.length - 1)] ?? delays[0] ?? 1_000;
}

function isPubkeyAwaiting(error: unknown): boolean {
  return (
    error instanceof ApiError &&
    (error.status === 404 || error.errorCode === 8009)
  );
}

function resolveTerminalState(
  pending: PendingCredentialWithState,
): AcceptStatus | null {
  if (pending.consumed_at || pending.remote_state === "consumed") {
    return "consumed";
  }
  if (pending.declined_at) {
    return "declined";
  }
  if (pending.remote_state === "decrypt_failed") {
    return "decrypt_failed";
  }
  if (pending.remote_state === "expired") {
    return "expired";
  }
  if (Date.parse(pending.expires_at) <= Date.now()) {
    return "expired";
  }
  if (!pending.is_active && !pending.consumed_at && !pending.declined_at) {
    return "expired";
  }
  return null;
}

function terminalVariant(status: AcceptStatus) {
  if (status === "consumed") return "success";
  if (status === "legacy_fallback" || status === "timeout") return "warning";
  if (
    status === "partial_decrypted" ||
    status === "decrypt_failed" ||
    status === "expired" ||
    status === "declined" ||
    status === "error"
  ) {
    return "destructive";
  }
  return "secondary";
}

export function CredentialAcceptPage() {
  const { nodeId, pendingId } = useParams({ strict: false }) as {
    nodeId?: string;
    pendingId: string;
  };
  const search = useSearch({ strict: false }) as { return_to?: string };
  const navigate = useNavigate();
  const secretInputRef = useRef<HTMLInputElement>(null);
  const {
    data: node,
    isLoading: nodeLoading,
    error: nodeError,
  } = useNode(nodeId ?? "");

  const [status, setStatus] = useState<AcceptStatus>("idle");
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [ciphertextResponse, setCiphertextResponse] =
    useState<NodePendingCredentialCiphertextResponse | null>(null);
  const [fanOutResponse, setFanOutResponse] =
    useState<FanOutPendingCredentialCiphertextResponse | null>(null);

  useBreadcrumbLabel("Accept credential");

  async function fetchPubkeyWithBackoff(): Promise<NodePendingCredentialPubkeyResponse | null> {
    if (!nodeId) return null;
    const startedAt = Date.now();
    let attempt = 0;
    while (Date.now() - startedAt < PUBKEY_WAIT_MS) {
      try {
        return await api.get<NodePendingCredentialPubkeyResponse>(
          `/nodes/${encodeURIComponent(nodeId)}/credentials/pending/${encodeURIComponent(pendingId)}`,
        );
      } catch (err) {
        if (!isPubkeyAwaiting(err)) {
          throw err;
        }
        const remaining = PUBKEY_WAIT_MS - (Date.now() - startedAt);
        if (remaining <= 0) break;
        await delay(Math.min(nextDelay(PUBKEY_DELAYS_MS, attempt), remaining));
        attempt += 1;
      }
    }
    return null;
  }

  async function fetchFanOutStatus(): Promise<FanOutPendingCredentialResponse> {
    return api.get<FanOutPendingCredentialResponse>(
      `/nodes/credentials/pending/${encodeURIComponent(pendingId)}/fan-out`,
    );
  }

  async function fetchFanOutPubkeysWithBackoff(): Promise<FanOutPendingCredentialPubkeysResponse | null> {
    const startedAt = Date.now();
    let attempt = 0;
    while (Date.now() - startedAt < PUBKEY_WAIT_MS) {
      const pubkeys = await api.get<FanOutPendingCredentialPubkeysResponse>(
        `/nodes/credentials/pending/${encodeURIComponent(pendingId)}/fan-out/pubkeys`,
      );
      if (
        pubkeys.targets.length > 0 &&
        pubkeys.targets.every((target) => Boolean(target.node_pubkey))
      ) {
        return pubkeys;
      }
      const remaining = PUBKEY_WAIT_MS - (Date.now() - startedAt);
      if (remaining <= 0) break;
      await delay(Math.min(nextDelay(PUBKEY_DELAYS_MS, attempt), remaining));
      attempt += 1;
    }
    return null;
  }

  async function postCiphertext(
    envelope: CiphertextEnvelope,
  ): Promise<NodePendingCredentialCiphertextResponse> {
    if (!nodeId) throw new Error("Node id is required.");
    return api.post<NodePendingCredentialCiphertextResponse>(
      `/nodes/${encodeURIComponent(nodeId)}/credentials/pending/${encodeURIComponent(pendingId)}/ciphertext`,
      envelope,
    );
  }

  async function postFanOutCiphertexts(body: {
    readonly fan_out_revision: number;
    readonly items: ReadonlyArray<
      CiphertextEnvelope & { readonly node_id: string; readonly generation: number }
    >;
  }): Promise<FanOutPendingCredentialCiphertextResponse> {
    return api.post<FanOutPendingCredentialCiphertextResponse>(
      `/nodes/credentials/pending/${encodeURIComponent(pendingId)}/fan-out/ciphertexts`,
      body,
    );
  }

  async function retryFailedFanOutNodes() {
    if (!fanOutResponse) return;
    setErrorMessage(null);
    try {
      const retry = await api.post<FanOutPendingCredentialResponse>(
        `/nodes/credentials/pending/${encodeURIComponent(fanOutResponse.fanout_id)}/fan-out/retry-failed`,
        { fan_out_revision: fanOutResponse.fan_out_revision },
      );
      setFanOutResponse(null);
      setCiphertextResponse(null);
      setStatus("idle");
      toast.info(`${String(retry.targets.length)} failed target(s) reset`);
    } catch (err) {
      setErrorMessage(
        err instanceof ApiError || err instanceof Error
          ? err.message
          : "Failed to retry fan-out targets",
      );
    }
  }

  async function fetchPendingMetadata(): Promise<PendingCredentialWithState> {
    if (!nodeId) throw new Error("Node id is required.");
    const res = await api.get<PendingCredentialListResponse>(
      `/nodes/${encodeURIComponent(nodeId)}/credentials/pending?include_history=true`,
    );
    const pending = res.pending_credentials.find(
      (credential) => credential.id === pendingId,
    );
    if (!pending) {
      throw new Error("Pending credential metadata was not found.");
    }
    return pending;
  }

  async function pollTerminalState(): Promise<AcceptStatus> {
    if (!nodeId) throw new Error("Node id is required.");
    const startedAt = Date.now();
    let attempt = 0;
    while (Date.now() - startedAt < POLL_WAIT_MS) {
      const res = await api.get<PendingCredentialListResponse>(
        `/nodes/${encodeURIComponent(nodeId)}/credentials/pending?include_history=true`,
      );
      const pending = res.pending_credentials.find(
        (credential) => credential.id === pendingId,
      );
      if (pending) {
        const terminal = resolveTerminalState(pending);
        if (terminal) return terminal;
      }
      const remaining = POLL_WAIT_MS - (Date.now() - startedAt);
      if (remaining <= 0) break;
      await delay(Math.min(nextDelay(POLL_DELAYS_MS, attempt), remaining));
      attempt += 1;
    }
    return "timeout";
  }

  async function pollFanOutTerminalState(): Promise<AcceptStatus> {
    const startedAt = Date.now();
    let attempt = 0;
    while (Date.now() - startedAt < POLL_WAIT_MS) {
      const pending = await fetchFanOutStatus();
      const terminal = statusFromRemoteState(pending.remote_state ?? "ciphertext_received");
      if (terminal) return terminal;
      const remaining = POLL_WAIT_MS - (Date.now() - startedAt);
      if (remaining <= 0) break;
      await delay(Math.min(nextDelay(POLL_DELAYS_MS, attempt), remaining));
      attempt += 1;
    }
    return "timeout";
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setErrorMessage(null);
    setCiphertextResponse(null);
    setFanOutResponse(null);

    const input = secretInputRef.current;
    const secret = input?.value ?? "";
    const plaintext = new TextEncoder().encode(secret);
    if (input) input.value = "";

    const parsedSecret = acceptNodeCredentialSecretSchema.safeParse(plaintext);
    if (!parsedSecret.success) {
      plaintext.fill(0);
      setErrorMessage(
        parsedSecret.error.issues[0]?.message ?? "Invalid credential value.",
      );
      return;
    }

    if (nodeId && node?.capabilities?.remote_credential_crypto_v1 === false) {
      plaintext.fill(0);
      setStatus("legacy_fallback");
      return;
    }

    try {
      setStatus("waiting_pubkey");
      if (!nodeId) {
        const [pending, pubkeys] = await Promise.all([
          fetchFanOutStatus(),
          fetchFanOutPubkeysWithBackoff(),
        ]);
        if (!pubkeys) {
          setStatus("legacy_fallback");
          return;
        }

        setStatus("encrypting");
        const items = pubkeys.targets.map((target) => {
          if (!target.node_pubkey) {
            throw new Error("Fan-out target pubkey is not ready.");
          }
          const context = buildRciContext({
            node_id: target.node_id,
            pending_credential_id: pubkeys.fanout_id,
            service_slug: pending.service_slug,
            injection_method: pending.injection_method,
            field_name: pending.field_name,
            target_url: pending.target_url ?? null,
            version: target.version,
          });
          return {
            node_id: target.node_id,
            generation: target.generation,
            ...encrypt(plaintext, target.node_pubkey, context),
          };
        });

        setStatus("posting");
        const postResult = await postFanOutCiphertexts({
          fan_out_revision: pubkeys.fan_out_revision,
          items,
        });
        setFanOutResponse(postResult);
        const directTerminal = statusFromRemoteState(postResult.remote_state);
        if (directTerminal) {
          setStatus(directTerminal);
          return;
        }
        setStatus("polling");
        setStatus(await pollFanOutTerminalState());
        return;
      }

      const pubkey = await fetchPubkeyWithBackoff();
      if (!pubkey) {
        setStatus("legacy_fallback");
        return;
      }

      setStatus("encrypting");
      const pending = await fetchPendingMetadata();
      const context = buildRciContext({
        node_id: pubkey.node_id,
        pending_credential_id: pubkey.pending_id,
        service_slug: pubkey.service_slug,
        injection_method: pending.injection_method,
        field_name: pending.field_name,
        target_url: pending.target_url ?? null,
        version: pubkey.version,
      });
      const envelope = encrypt(plaintext, pubkey.node_pubkey, context);

      setStatus("posting");
      const postResult = await postCiphertext(envelope);
      setCiphertextResponse(postResult);

      const directTerminal = statusFromRemoteState(postResult.remote_state);
      if (directTerminal) {
        setStatus(directTerminal);
        toast.success("Credential accepted");
        return;
      }

      setStatus("polling");
      const terminal = await pollTerminalState();
      setStatus(terminal);
      if (terminal === "consumed") {
        toast.success("Credential accepted");
      }
    } catch (err) {
      setStatus("error");
      setErrorMessage(
        err instanceof ApiError || err instanceof Error
          ? err.message
          : "Failed to accept credential",
      );
    } finally {
      plaintext.fill(0);
    }
  }

  const isBusy =
    status === "waiting_pubkey" ||
    status === "encrypting" ||
    status === "posting" ||
    status === "polling";
  const terminalText = terminalDescriptions[status];

  return (
    <div className="space-y-8">
      <PageHeader
        title="Accept Credential"
        description="Encrypt a pending node credential for local storage."
        actions={
          <Button
            variant="outline"
            onClick={() => {
              const target = getSafeCredentialAcceptReturnTo(search.return_to);
              if (target) {
                window.location.assign(target);
                return;
              }
              if (nodeId) {
                void navigate({ to: "/nodes/$nodeId", params: { nodeId } });
              } else {
                void navigate({ to: "/nodes" });
              }
            }}
          >
            Back
          </Button>
        }
      />

      {nodeLoading ? (
        <div className="space-y-3">
          <Skeleton className="h-10 w-64" />
          <Skeleton className="h-48 w-full" />
        </div>
      ) : nodeError ? (
        <ErrorBanner
          message={
            nodeError instanceof ApiError
              ? nodeError.message
              : "Failed to load node."
          }
        />
      ) : (
        <DetailSection title="Pending credential">
          <div className="space-y-4 p-5">
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={terminalVariant(status)}>
                {statusLabel[status]}
              </Badge>
              {ciphertextResponse && (
                <Badge variant="secondary">
                  {ciphertextResponse.delivery_status}
                </Badge>
              )}
              {fanOutResponse && (
                <Badge variant="secondary">
                  {fanOutResponse.targets.length} targets
                </Badge>
              )}
            </div>

            {errorMessage && <ErrorBanner message={errorMessage} />}
            {terminalText && (
              <p className="text-[12px] text-muted-foreground">
                {terminalText}
              </p>
            )}

            {status === "legacy_fallback" ? (
              <div className="rounded-lg border border-border/50 bg-white/[0.03] p-4">
                <div className="mb-2 flex items-center gap-2 text-[13px] font-medium text-foreground">
                  <Terminal className="h-3.5 w-3.5 text-muted-foreground" />
                  Node CLI
                </div>
                <code className="block break-all rounded-lg bg-background px-3 py-2 text-[12px] text-muted-foreground">
                  nyxid node credentials pending
                </code>
                <code className="mt-2 block break-all rounded-lg bg-background px-3 py-2 text-[12px] text-muted-foreground">
                  nyxid node credentials accept &lt;service-slug&gt;
                </code>
              </div>
            ) : (
              <form
                className="space-y-4"
                onSubmit={(event) => void handleSubmit(event)}
              >
                <div className="space-y-2">
                  <Label htmlFor="credential-secret">Credential value</Label>
                  <Input
                    id="credential-secret"
                    ref={secretInputRef}
                    type="password"
                    autoComplete="new-password"
                    disabled={isBusy}
                  />
                </div>
                <Button
                  variant="primary"
                  type="submit"
                  disabled={isBusy}
                  isLoading={isBusy}
                >
                  <ButtonIcon variant="primary">
                    <ShieldCheck className="h-3 w-3" />
                  </ButtonIcon>
                  Accept
                </Button>
              </form>
            )}
            {fanOutResponse && (
              <div className="divide-y divide-border rounded-md border border-border">
                {fanOutResponse.targets.map((target) => (
                  <div
                    key={`${target.node_id}:${String(target.generation)}`}
                    className="flex items-center justify-between gap-3 px-3 py-2 text-[12px]"
                  >
                    <span className="truncate font-mono text-muted-foreground">
                      {target.node_id}
                    </span>
                    <Badge variant={target.error_code ? "destructive" : "secondary"}>
                      {target.remote_state ?? "pending"}
                    </Badge>
                  </div>
                ))}
              </div>
            )}
            {fanOutResponse?.remote_state === "partial_decrypted" && (
              <Button
                variant="outline"
                type="button"
                disabled={isBusy}
                onClick={() => void retryFailedFanOutNodes()}
              >
                Retry failed
              </Button>
            )}
          </div>
        </DetailSection>
      )}
    </div>
  );
}

function statusFromRemoteState(
  state: NodePendingCredentialRemoteState,
): AcceptStatus | null {
  if (state === "consumed") return "consumed";
  if (state === "partial_decrypted") return "partial_decrypted";
  if (state === "decrypt_failed") return "decrypt_failed";
  if (state === "expired") return "expired";
  return null;
}
