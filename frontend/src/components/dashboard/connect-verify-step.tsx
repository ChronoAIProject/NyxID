import { useState } from "react";
import { Check, ExternalLink, Loader2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CopyableField } from "@/components/shared/copyable-field";
import { useCreateApiKey } from "@/hooks/use-api-keys";
import {
  FIRST_PROXY_CALL_SUCCEEDED_EVENT,
  VERIFY_KEY_LOADING_END_EVENT,
  VERIFY_KEY_LOADING_START_EVENT,
} from "@/hooks/use-proxy-onboarding";
import type { ApiKeyCreateResponse } from "@/types/api";

const OPENAI_SHAPED_HINTS =
  /(openai|anthropic|claude|gemini|deepseek|groq|together|mistral|fireworks|perplexity|cohere|xai|grok)/i;

const PROBE_TIMEOUT_MS = 8000;

/**
 * Phase machine — every transition is gated by an explicit user click
 * (per Calvin's feedback: "the lift should be to ask them if they want
 * to create a key for this ai service instead of doing it automatically.
 * test comes after key is created"). No background activity ever runs
 * without the user pulling a trigger first.
 */
type Phase =
  | "connected"       // initial — service is connected, ask if they want an Agent Key
  | "minting"         // user clicked Create Agent Key, mint in flight
  | "mint_failed"     // mint errored — show retry + skip
  | "key_ready"       // mint succeeded — show key + offer to test
  | "probing"         // user clicked Test — probe in flight
  | "probe_success"   // probe returned 2xx — the actual end-to-end aha
  | "probe_failed";   // probe returned 4xx/5xx/timeout — show diagnose hint + retry

interface CreatedKey {
  readonly id: string;
  readonly slug: string;
  readonly serviceName: string;
  /**
   * Drives whether the test button is even offered. Non-credential
   * completions (device_code, oauth) typically have non-OpenAI-shape
   * backends (Codex on chatgpt.com etc.) where re-probing /v1/models
   * would false-fail against a working connection. We don't hide the
   * test entirely — we just don't run it without an opt-in.
   */
  readonly completionMode: "credential" | "device_code" | "oauth" | "none";
}

interface ConnectVerifyStepProps {
  readonly createdKey: CreatedKey;
  readonly isNodeRouted: boolean;
  readonly onDone: () => void;
  readonly onViewDetails: () => void;
}

function probePathForSlug(slug: string): string {
  return OPENAI_SHAPED_HINTS.test(slug) ? "v1/models" : "";
}

function proxyBaseUrl(slug: string): string {
  return `${window.location.origin}/api/v1/proxy/s/${slug}`;
}

function canProbe(createdKey: CreatedKey): boolean {
  // Only show the Test button when we have a probeable path AND the
  // upstream actually expects bearer creds — for Codex/ChatGPT-backed
  // services (auth_method=none + non-openai backend) the probe would
  // 404 against a working connection.
  return (
    createdKey.completionMode === "credential" &&
    probePathForSlug(createdKey.slug) !== ""
  );
}

/**
 * Wave-aha-1 A4+ (opt-in rework) — post-connect aha moment, but every
 * step requires the user to pull a trigger:
 *
 *   1. Service is connected → ask "Create an Agent Key for this?" with
 *      [Create Agent Key] [Maybe later]. No background mint.
 *   2. User clicks → mint runs visibly. If it fails, show the error
 *      and offer retry — instead of leaving a spinner spinning forever.
 *   3. Mint succeeds → show full key + proxy URL + (where applicable)
 *      env-var snippet, with [Test connection] / [I'll wire it myself].
 *   4. User clicks Test → bearer probe runs visibly. Success or precise
 *      failure copy. Retry available either way.
 *
 * The earlier auto-mint version hung silently when the mint request
 * never reached the BE (no surfaced error, no user agency). This
 * version makes failures impossible to miss because they always follow
 * a deliberate click.
 */
export function ConnectVerifyStep({
  createdKey,
  isNodeRouted,
  onDone,
  onViewDetails,
}: ConnectVerifyStepProps) {
  const createApiKey = useCreateApiKey();
  const [phase, setPhase] = useState<Phase>("connected");
  const [agentKey, setAgentKey] = useState<ApiKeyCreateResponse | null>(null);
  const [mintError, setMintError] = useState<string | null>(null);
  const [httpStatus, setHttpStatus] = useState<number | null>(null);
  const [probeError, setProbeError] = useState<string | null>(null);

  const showProbeOption = canProbe(createdKey);

  function triggerMint() {
    setPhase("minting");
    setMintError(null);
    createApiKey.mutate(
      {
        name: `Key for ${createdKey.serviceName}`,
        scopes: ["proxy"],
        allow_all_services: false,
        allowed_service_ids: [createdKey.id],
        allow_all_nodes: true,
      },
      {
        onSuccess: (key) => {
          setAgentKey(key);
          setPhase("key_ready");
        },
        onError: (err) => {
          setMintError(err instanceof Error ? err.message : String(err));
          setPhase("mint_failed");
        },
      },
    );
  }

  async function triggerProbe() {
    if (!agentKey) return;
    setPhase("probing");
    setProbeError(null);
    setHttpStatus(null);

    window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_START_EVENT));

    const url = `${proxyBaseUrl(createdKey.slug)}/${probePathForSlug(createdKey.slug)}`;
    const controller = new AbortController();
    const timer = window.setTimeout(
      () => controller.abort(),
      PROBE_TIMEOUT_MS,
    );

    let status: number | null = null;
    try {
      const res = await fetch(url, {
        method: "GET",
        // Bearer = the user's just-minted Agent Key. credentials:"omit"
        // so we don't accidentally fall back to the session cookie —
        // we want to prove the bearer path the user's TOOL will take.
        credentials: "omit",
        headers: {
          Authorization: `Bearer ${agentKey.full_key}`,
          "Content-Type": "application/json",
        },
        signal: controller.signal,
      });
      status = res.status;
    } catch {
      status = null;
    } finally {
      window.clearTimeout(timer);
    }

    window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_END_EVENT));
    setHttpStatus(status);

    const ok = status !== null && status >= 200 && status < 400;
    if (ok) {
      setPhase("probe_success");
      window.dispatchEvent(
        new CustomEvent(FIRST_PROXY_CALL_SUCCEEDED_EVENT),
      );
    } else {
      setPhase("probe_failed");
      setProbeError(diagnose(status, isNodeRouted));
    }
  }

  return (
    <div className="space-y-4 py-2">
      {/* Top status banner — what just happened / what we're doing now */}
      <StatusBanner
        phase={phase}
        serviceName={createdKey.serviceName}
        mintError={mintError}
        probeError={probeError}
        httpStatus={httpStatus}
      />

      {/* Once minted, the Agent Key + URL panel stays visible from
          key_ready through probe_success / probe_failed so the user can
          copy it regardless of test outcome. */}
      {agentKey && (
        <AgentKeyPanel
          agentKey={agentKey}
          createdKey={createdKey}
          showOpenAiEnvSnippet={phase === "probe_success"}
        />
      )}

      {/* Action buttons — gate every transition behind an explicit click. */}
      <Footer
        phase={phase}
        showProbeOption={showProbeOption}
        onMint={triggerMint}
        onProbe={triggerProbe}
        onDone={onDone}
        onViewDetails={onViewDetails}
        hasKey={agentKey !== null}
      />
    </div>
  );
}

function StatusBanner({
  phase,
  serviceName,
  mintError,
  probeError,
  httpStatus,
}: {
  readonly phase: Phase;
  readonly serviceName: string;
  readonly mintError: string | null;
  readonly probeError: string | null;
  readonly httpStatus: number | null;
}) {
  switch (phase) {
    case "connected":
      return (
        <Banner
          icon="check"
          tone="success"
          title={`${serviceName} connected`}
        >
          NyxID has stored your credentials. To let your AI tools use this
          connection, you&apos;ll need an Agent Key scoped to this service.
        </Banner>
      );
    case "minting":
      return (
        <Banner icon="spinner" tone="neutral" title="Creating your Agent Key…">
          Minting a scoped key with the <code>proxy</code> scope and access
          to {serviceName} only.
        </Banner>
      );
    case "mint_failed":
      return (
        <Banner
          icon="warn"
          tone="warn"
          title="Couldn’t create the Agent Key"
        >
          {mintError ??
            "The server didn't accept the request."}{" "}
          You can retry, or skip and create one later from{" "}
          <span className="font-medium">/keys → Agent Keys → Create</span>.
        </Banner>
      );
    case "key_ready":
      return (
        <Banner
          icon="check"
          tone="success"
          title={`Agent Key created for ${serviceName}`}
        >
          Save the key now (it&apos;s only shown once). To prove the
          end-to-end path works, run a test below — or skip and wire it
          straight into your AI tool.
        </Banner>
      );
    case "probing":
      return (
        <Banner
          icon="spinner"
          tone="neutral"
          title="Testing the end-to-end path…"
        >
          Calling {serviceName} with the new Agent Key as bearer — the same
          path your AI tool will take. This takes a couple of seconds.
        </Banner>
      );
    case "probe_success":
      return (
        <Banner
          icon="check"
          tone="success"
          title={`Your AI tool can now call ${serviceName} through NyxID`}
        >
          End-to-end test succeeded
          {httpStatus !== null ? ` (HTTP ${String(httpStatus)})` : ""}.
          Copy the Agent Key + Base URL into your tool of choice — you&apos;re done.
        </Banner>
      );
    case "probe_failed":
      return (
        <Banner
          icon="warn"
          tone="warn"
          title="End-to-end test didn’t succeed"
        >
          <span>
            {probeError ??
              "The probe returned an unexpected response."}
            {httpStatus !== null ? ` (HTTP ${String(httpStatus)})` : ""}
            {" "}The Agent Key is still good — copy it above and use it from
            your tool, or retry the test below.
          </span>
        </Banner>
      );
  }
}

function AgentKeyPanel({
  agentKey,
  createdKey,
  showOpenAiEnvSnippet,
}: {
  readonly agentKey: ApiKeyCreateResponse;
  readonly createdKey: CreatedKey;
  readonly showOpenAiEnvSnippet: boolean;
}) {
  const proxyUrl = proxyBaseUrl(createdKey.slug);
  return (
    <div className="space-y-3 rounded-xl border border-border/50 bg-card p-4">
      <div className="space-y-1">
        <p className="text-[12px] font-semibold text-foreground">
          Your new Agent Key
        </p>
        <p className="text-[11px] text-muted-foreground">
          Save this now. The full secret is only shown here once — only the
          prefix is visible afterwards. Scoped to{" "}
          <span className="font-medium text-foreground">
            {createdKey.serviceName}
          </span>{" "}
          only, with the <code>proxy</code> scope.
        </p>
      </div>
      <CopyableField label="Agent Key" value={agentKey.full_key} />

      <div className="space-y-1 pt-1">
        <p className="text-[12px] font-semibold text-foreground">Proxy URL</p>
        <p className="text-[11px] text-muted-foreground">
          Point your AI tool at this URL instead of the service&apos;s direct
          API. NyxID brokers the call.
        </p>
      </div>
      <CopyableField label="Base URL" value={proxyUrl} />

      {showOpenAiEnvSnippet ? (
        <>
          <div className="space-y-1 pt-1">
            <p className="text-[12px] font-semibold text-foreground">
              Wire it up
            </p>
            <p className="text-[11px] text-muted-foreground">
              Most OpenAI-compatible tools (openai-python, Cursor,
              Continue.dev, …) read these two env vars.
            </p>
          </div>
          <CopyableField
            label="Shell env"
            value={`export OPENAI_API_KEY="${agentKey.full_key}"\nexport OPENAI_BASE_URL="${proxyUrl}/v1"`}
          />
        </>
      ) : (
        <div className="space-y-1 pt-1">
          <p className="text-[12px] font-semibold text-foreground">
            Wire it up
          </p>
          <p className="text-[11px] text-muted-foreground">
            Paste the Agent Key + Base URL above into your tool&apos;s
            provider configuration. Each tool wires this differently — check
            its docs if you&apos;re unsure.
          </p>
        </div>
      )}
    </div>
  );
}

function Footer({
  phase,
  showProbeOption,
  onMint,
  onProbe,
  onDone,
  onViewDetails,
  hasKey,
}: {
  readonly phase: Phase;
  readonly showProbeOption: boolean;
  readonly onMint: () => void;
  readonly onProbe: () => void;
  readonly onDone: () => void;
  readonly onViewDetails: () => void;
  readonly hasKey: boolean;
}) {
  // "View details" only makes sense after a key exists AND the dialog is
  // about to be dismissed anyway. Keep it available from key_ready
  // onward so users with multi-service setups can keep poking.
  const showViewDetails = hasKey;

  return (
    <div className="flex flex-wrap items-center justify-end gap-2">
      {showViewDetails && (
        <Button variant="outline" onClick={onViewDetails}>
          <ExternalLink className="mr-1.5 h-3.5 w-3.5" />
          View details
        </Button>
      )}

      {phase === "connected" && (
        <>
          <Button variant="outline" onClick={onDone}>
            Maybe later
          </Button>
          <Button variant="primary" size="lg" onClick={onMint}>
            Create Agent Key
          </Button>
        </>
      )}

      {phase === "minting" && (
        <Button variant="primary" size="lg" disabled isLoading>
          Creating…
        </Button>
      )}

      {phase === "mint_failed" && (
        <>
          <Button variant="outline" onClick={onDone}>
            Maybe later
          </Button>
          <Button variant="primary" size="lg" onClick={onMint}>
            Try again
          </Button>
        </>
      )}

      {phase === "key_ready" && (
        <>
          {showProbeOption ? (
            <>
              <Button variant="outline" onClick={onDone}>
                I&apos;ll wire it myself
              </Button>
              <Button variant="primary" size="lg" onClick={onProbe}>
                Test connection
              </Button>
            </>
          ) : (
            <Button variant="primary" size="lg" onClick={onDone}>
              Done
            </Button>
          )}
        </>
      )}

      {phase === "probing" && (
        <Button variant="primary" size="lg" disabled isLoading>
          Testing…
        </Button>
      )}

      {(phase === "probe_success" || phase === "probe_failed") && (
        <>
          {phase === "probe_failed" && (
            <Button variant="outline" onClick={onProbe}>
              Retry test
            </Button>
          )}
          <Button variant="primary" size="lg" onClick={onDone}>
            Done
          </Button>
        </>
      )}
    </div>
  );
}

function Banner({
  icon,
  tone,
  title,
  children,
}: {
  readonly icon: "spinner" | "check" | "warn";
  readonly tone: "neutral" | "success" | "warn";
  readonly title: string;
  readonly children: React.ReactNode;
}) {
  const tonalRing =
    tone === "success"
      ? "bg-success/20"
      : tone === "warn"
        ? "bg-warning/20"
        : "bg-muted";
  const glyph =
    icon === "spinner" ? (
      <Loader2 className="h-4 w-4 animate-spin text-muted-foreground" aria-hidden />
    ) : icon === "check" ? (
      <Check className="h-4 w-4 text-success" aria-hidden />
    ) : (
      <X className="h-4 w-4 text-warning" aria-hidden />
    );
  return (
    <div className="rounded-xl border border-border/50 bg-card p-4">
      <div className="flex items-start gap-3">
        <span
          className={`mt-0.5 inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-full ${tonalRing}`}
        >
          {glyph}
        </span>
        <div className="space-y-1">
          <p className="text-[13px] font-semibold text-foreground">{title}</p>
          <p className="text-[12px] text-muted-foreground">{children}</p>
        </div>
      </div>
    </div>
  );
}

function diagnose(
  status: number | null,
  isNodeRouted: boolean,
): string {
  if (status === null) {
    return "The probe timed out or was blocked by the browser.";
  }
  if (status === 401 || status === 403) {
    return `The downstream rejected the bearer-authenticated call (HTTP ${String(status)}). The upstream credential may have been mistyped or revoked.`;
  }
  if (status === 404) {
    if (isNodeRouted) {
      return "The proxy returned 404 — node-routed services need the node agent online to respond.";
    }
    return "The proxy returned 404 — the slug isn't routable yet, or the downstream doesn't expose this probe path.";
  }
  if (status === 429) {
    return "The downstream is rate-limiting (HTTP 429). Your credential + Agent Key work — retry in a minute.";
  }
  if (status >= 500) {
    return `The downstream returned a server error (HTTP ${String(status)}). NyxID's part of the path is fine; the upstream service is having issues right now.`;
  }
  return `The probe returned an unexpected HTTP ${String(status)}.`;
}
