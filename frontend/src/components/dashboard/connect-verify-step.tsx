import { useState } from "react";
import type { ReactNode } from "react";
import { AlertTriangle, Check, KeyRound, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { CopyableField } from "@/components/shared/copyable-field";
import { useCreateApiKey } from "@/hooks/use-api-keys";
import {
  FIRST_PROXY_CALL_SUCCEEDED_EVENT,
  VERIFY_KEY_LOADING_END_EVENT,
  VERIFY_KEY_LOADING_START_EVENT,
} from "@/hooks/use-proxy-onboarding";
import {
  isKnownUntestable,
  probeAgentKey,
  type ProbeOutcome,
} from "@/lib/proxy-probe";
import type { ApiKeyCreateResponse } from "@/types/api";

// Slug-shape hint used only to decide whether to render the
// OPENAI_API_KEY / OPENAI_BASE_URL env-var snippet. The reliability
// of the probe itself is driven by proxy-probe.ts, not this pattern.
const OPENAI_SHAPED_HINTS =
  /(openai|anthropic|claude|gemini|deepseek|groq|together|mistral|fireworks|perplexity|cohere|xai|grok)/i;

/**
 * Phase machine — every transition is gated by an explicit user click
 * per Calvin's feedback ("the lift should be to ask them if they want
 * to create a key for this ai service instead of doing it
 * automatically. test comes after key is created"). Nothing fires in
 * the background.
 *
 * `probe_success` means "the Agent Key is valid" — the downstream may
 * have returned any HTTP status (see proxy-probe.ts). `probe_failed`
 * means NyxID itself rejected the request (bad key, scope violation,
 * unknown slug, network error).
 */
export type Phase =
  | "connected"
  | "minting"
  | "mint_failed"
  | "key_ready"
  | "probing"
  | "probe_success"
  | "probe_failed";

export interface CreatedKey {
  readonly id: string;
  readonly slug: string;
  readonly catalogSlug: string;
  readonly serviceName: string;
  readonly completionMode: "credential" | "device_code" | "oauth" | "none";
}

interface ConnectVerifyStepProps {
  readonly createdKey: CreatedKey;
  readonly isNodeRouted: boolean;
  readonly onDone: () => void;
}

function proxyBaseUrl(slug: string): string {
  return `${window.location.origin}/api/v1/proxy/s/${slug}`;
}

/**
 * Wave-aha-1 A4+ — umbrella-view post-connect setup.
 *
 * The DialogTitle owns the umbrella state ("<service> connected").
 * This component's body morphs per phase:
 *
 *   connected      → one-line "you need an Agent Key to use this"
 *                    + Maybe later / Create Agent Key
 *   minting        → inline spinner
 *   mint_failed    → inline error + Maybe later / Try again
 *   key_ready      → Agent Key panel + I'll wire / Test Agent Key
 *   probing        → panel stays visible + inline spinner
 *   probe_success  → panel + env snippet + probe diagnostic + Done
 *                    (success is defined by proxy-probe classification —
 *                    "agent key valid", not "downstream returned 2xx")
 *   probe_failed   → panel + inline error + Retry test / Done
 *                    (only reached when NyxID itself rejected the request)
 */
export function ConnectVerifyStep({
  createdKey,
  isNodeRouted,
  onDone,
}: ConnectVerifyStepProps) {
  void isNodeRouted; // kept in props so the parent's data flow stays stable

  const createApiKey = useCreateApiKey();
  const [phase, setPhase] = useState<Phase>("connected");
  const [agentKey, setAgentKey] = useState<ApiKeyCreateResponse | null>(null);
  const [mintError, setMintError] = useState<string | null>(null);
  const [probe, setProbe] = useState<ProbeOutcome | null>(null);

  function triggerMint() {
    console.info("[aha] triggerMint fired", createdKey.id, createdKey.serviceName);
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
          console.info("[aha] mint success", key.key_prefix);
          setAgentKey(key);
          setPhase("key_ready");
        },
        onError: (err) => {
          const msg = err instanceof Error ? err.message : String(err);
          console.warn("[aha] mint failed", msg, err);
          setMintError(msg);
          setPhase("mint_failed");
        },
      },
    );
  }

  async function triggerProbe() {
    if (!agentKey) return;
    console.info("[aha] triggerProbe fired", createdKey.slug);
    setPhase("probing");
    setProbe(null);
    window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_START_EVENT));

    const outcome = await probeAgentKey(createdKey.slug, {
      bearerToken: agentKey.full_key,
    });

    window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_END_EVENT));
    console.info(
      "[aha] probe outcome",
      "agentKeyValid=",
      outcome.agentKeyValid,
      "http=",
      outcome.httpStatus,
      "downstream=",
      outcome.downstreamStatus,
    );
    setProbe(outcome);

    // Success = the Agent Key + scope + route all resolved on NyxID's
    // side. A downstream 401/404/500 is diagnostic, not blocking, so
    // we still transition to probe_success and let the diagnostic
    // copy explain what happened downstream. The one thing that
    // fails the test is a NyxID-layer rejection (bad key, scope
    // violation, network failure) — those land in probe_failed.
    if (outcome.agentKeyValid) {
      setPhase("probe_success");
      window.dispatchEvent(new CustomEvent(FIRST_PROXY_CALL_SUCCEEDED_EVENT));
    } else {
      setPhase("probe_failed");
    }
  }

  // Per-provider probe registry — some services (Codex chat-only,
  // OpenClaw self-hosted, etc.) have no cheap GET endpoint we can
  // reliably test. For those we hide the Test button entirely and
  // hint at manual verification — a "green light" from a probe we
  // don't trust is worse than no light at all.
  const untestable = isKnownUntestable(createdKey.slug);

  return (
    <div className="space-y-4 py-2">
      <Body
        phase={phase}
        createdKey={createdKey}
        agentKey={agentKey}
        mintError={mintError}
        probe={probe}
        untestable={untestable}
      />
      <Footer
        phase={phase}
        untestable={untestable}
        onMint={triggerMint}
        onProbe={triggerProbe}
        onDone={onDone}
      />
    </div>
  );
}

function Body({
  phase,
  createdKey,
  agentKey,
  mintError,
  probe,
  untestable,
}: {
  readonly phase: Phase;
  readonly createdKey: CreatedKey;
  readonly agentKey: ApiKeyCreateResponse | null;
  readonly mintError: string | null;
  readonly probe: ProbeOutcome | null;
  readonly untestable: boolean;
}) {
  if (phase === "connected") {
    return (
      <div className="flex items-start gap-3 rounded-xl border border-border/50 bg-card p-4">
        <KeyRound
          className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground"
          aria-hidden
        />
        <p className="text-[12px] leading-relaxed text-muted-foreground">
          Your AI tools need an{" "}
          <span className="font-medium text-foreground">Agent Key</span> to
          call <span className="font-medium text-foreground">{createdKey.serviceName}</span>{" "}
          through NyxID. NyxID injects your stored credentials server-side —
          your tool never sees the original secret.
        </p>
      </div>
    );
  }

  if (phase === "minting") {
    return (
      <InlineStatus icon={<Spinner />}>
        Creating your Agent Key…
      </InlineStatus>
    );
  }

  if (phase === "mint_failed") {
    return (
      <InlineStatus
        tone="warn"
        icon={<AlertTriangle className="h-4 w-4 text-warning" aria-hidden />}
      >
        {mintError ?? "The server didn't accept the request."}{" "}
        Retry, or create one later at{" "}
        <span className="font-medium">/keys → Agent Keys → Create</span>.
      </InlineStatus>
    );
  }

  // key_ready + probing + probe_success + probe_failed all show the panel.
  // Env snippet renders only when the probe verified the key AND the slug
  // is openai-shaped (compatible with OPENAI_BASE_URL / OPENAI_API_KEY).
  const showOpenAiEnvSnippet =
    phase === "probe_success" && OPENAI_SHAPED_HINTS.test(createdKey.slug);

  return (
    <div className="space-y-3">
      {agentKey && (
        <AgentKeyPanel
          agentKey={agentKey}
          createdKey={createdKey}
          showOpenAiEnvSnippet={showOpenAiEnvSnippet}
        />
      )}

      {phase === "key_ready" && untestable && (
        <InlineStatus
          icon={<KeyRound className="h-4 w-4 text-muted-foreground" aria-hidden />}
        >
          Automatic testing isn't supported for{" "}
          <span className="font-medium text-foreground">
            {createdKey.serviceName}
          </span>{" "}
          — it doesn't expose a cheap status endpoint. Verify by running one
          real call from your AI tool once you've wired the env vars above.
        </InlineStatus>
      )}

      {phase === "probing" && (
        <InlineStatus icon={<Spinner />}>
          Calling {createdKey.serviceName} with your Agent Key — the same
          path your AI tool will take.
        </InlineStatus>
      )}

      {phase === "probe_success" && probe && (
        <InlineStatus
          tone={probe.downstreamStatus === "ok" ? "success" : "warn"}
          icon={
            probe.downstreamStatus === "ok" ? (
              <Check className="h-4 w-4 text-success" aria-hidden />
            ) : (
              <AlertTriangle className="h-4 w-4 text-warning" aria-hidden />
            )
          }
        >
          {probe.diagnostic}
        </InlineStatus>
      )}

      {phase === "probe_failed" && probe && (
        <InlineStatus
          tone="warn"
          icon={<AlertTriangle className="h-4 w-4 text-warning" aria-hidden />}
        >
          {probe.diagnostic}
        </InlineStatus>
      )}
    </div>
  );
}

function Spinner() {
  return (
    <Loader2
      className="h-4 w-4 animate-spin text-muted-foreground"
      aria-hidden
    />
  );
}

function InlineStatus({
  tone = "neutral",
  icon,
  children,
}: {
  readonly tone?: "neutral" | "success" | "warn";
  readonly icon: ReactNode;
  readonly children: ReactNode;
}) {
  const toneClass =
    tone === "success"
      ? "text-foreground"
      : tone === "warn"
        ? "text-foreground"
        : "text-muted-foreground";
  return (
    <div className="flex items-start gap-2">
      <span className="mt-0.5 shrink-0">{icon}</span>
      <p className={`text-[12px] leading-relaxed ${toneClass}`}>{children}</p>
    </div>
  );
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
          Save this now. The full secret is only shown here once. Scoped to{" "}
          <span className="font-medium text-foreground">
            {createdKey.serviceName}
          </span>{" "}
          only, with the <code>proxy</code> scope.
        </p>
      </div>
      <CopyableField label="Agent Key" value={agentKey.full_key} />
      <CopyableField label="Base URL" value={proxyUrl} />
      {showOpenAiEnvSnippet && (
        <>
          <p className="text-[11px] text-muted-foreground pt-1">
            Most OpenAI-compatible tools (openai-python, Cursor, Continue.dev, …)
            read these two env vars.
          </p>
          <CopyableField
            label="Shell env"
            value={`export OPENAI_API_KEY="${agentKey.full_key}"\nexport OPENAI_BASE_URL="${proxyUrl}/v1"`}
          />
        </>
      )}
    </div>
  );
}

type FooterAction = "onMint" | "onProbe" | "onDone";

function Footer({
  phase,
  untestable,
  onMint,
  onProbe,
  onDone,
}: {
  readonly phase: Phase;
  readonly untestable: boolean;
  readonly onMint: () => void;
  readonly onProbe: () => void;
  readonly onDone: () => void;
}) {
  const config = footerConfig(phase, untestable);
  // Dispatch INVOKES the mapped handler. Do NOT refactor back to
  // `(a) => a === "onMint" ? onMint : …` — that returns the function
  // reference but never calls it (silent no-op click). This one bit
  // Calvin twice in a row; the connect-verify-step.test.tsx suite
  // has a REGRESSION test that will fail immediately if it does.
  function dispatch(action: FooterAction) {
    console.info("[aha] footer click", action, "phase=", phase);
    if (action === "onMint") onMint();
    else if (action === "onProbe") onProbe();
    else onDone();
  }

  return (
    <div className="flex items-center justify-end gap-2">
      <Button
        variant="outline"
        size="lg"
        onClick={() => dispatch(config.secondary)}
        disabled={config.secondaryLabel === null || config.busy}
        className={config.secondaryLabel === null ? "invisible" : ""}
        aria-hidden={config.secondaryLabel === null}
      >
        {config.secondaryLabel ?? "placeholder"}
      </Button>
      <Button
        variant="primary"
        size="lg"
        onClick={() => dispatch(config.primary)}
        disabled={config.busy}
        isLoading={config.busy}
      >
        {config.primaryLabel}
      </Button>
    </div>
  );
}

interface FooterConfig {
  readonly secondaryLabel: string | null;
  readonly secondary: FooterAction;
  readonly primaryLabel: string;
  readonly primary: FooterAction;
  readonly busy: boolean;
}

function footerConfig(phase: Phase, untestable: boolean): FooterConfig {
  switch (phase) {
    case "connected":
      return {
        secondaryLabel: "Maybe later",
        secondary: "onDone",
        primaryLabel: "Create Agent Key",
        primary: "onMint",
        busy: false,
      };
    case "minting":
      return {
        secondaryLabel: null,
        secondary: "onDone",
        primaryLabel: "Creating…",
        primary: "onMint",
        busy: true,
      };
    case "mint_failed":
      return {
        secondaryLabel: "Maybe later",
        secondary: "onDone",
        primaryLabel: "Try again",
        primary: "onMint",
        busy: false,
      };
    case "key_ready":
      // Untestable providers (Codex, OpenClaw, Lark bot, etc.) skip
      // the probe entirely — a "green" result we can't trust is worse
      // than no result. Single Done CTA in that case.
      if (untestable) {
        return {
          secondaryLabel: null,
          secondary: "onDone",
          primaryLabel: "Done",
          primary: "onDone",
          busy: false,
        };
      }
      return {
        secondaryLabel: "I'll wire it myself",
        secondary: "onDone",
        primaryLabel: "Test Agent Key",
        primary: "onProbe",
        busy: false,
      };
    case "probing":
      return {
        secondaryLabel: null,
        secondary: "onDone",
        primaryLabel: "Testing…",
        primary: "onProbe",
        busy: true,
      };
    case "probe_success":
      return {
        secondaryLabel: null,
        secondary: "onDone",
        primaryLabel: "Done",
        primary: "onDone",
        busy: false,
      };
    case "probe_failed":
      return {
        secondaryLabel: "Retry test",
        secondary: "onProbe",
        primaryLabel: "Done",
        primary: "onDone",
        busy: false,
      };
  }
}
