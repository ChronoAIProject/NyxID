import { useEffect, useRef, useState } from "react";
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

/**
 * Slugs that almost always expose `/v1/models` on AI providers. Used to
 * decide whether (a) the bearer probe path should be `v1/models` or `/`
 * and (b) whether the OpenAI-env-var config snippet is worth showing.
 * Mismatched services still get the proxy-URL curl example, just not
 * the env-var snippet (which wouldn't work for non-OpenAI-shaped APIs).
 */
const OPENAI_SHAPED_HINTS =
  /(openai|anthropic|claude|gemini|deepseek|groq|together|mistral|fireworks|perplexity|cohere|xai|grok)/i;

const PROBE_TIMEOUT_MS = 8000;

type Phase =
  | "minting"          // POST /api-keys in flight
  | "mint_failed"      // backend rejected the auto-create
  | "ready_to_probe"   // key minted, about to fire probe
  | "probing"          // bearer-auth fetch in flight
  | "success"          // probe returned 2xx/3xx — the aha
  | "probe_failed"     // bearer auth landed but proxy returned 4xx/5xx
  | "skipped";         // catalog says no creds AND not OpenAI-shaped — show key + config only

interface CreatedKey {
  readonly id: string;
  readonly slug: string;
  readonly serviceName: string;
  /**
   * How the underlying credential got established. Drives whether we
   * probe at all (typed creds + OAuth/device-code → probe; auth_method
   * "none" with a ChatGPT-backed URL like Codex → skip because /v1/models
   * doesn't exist on that backend).
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

function looksOpenAiShaped(slug: string): boolean {
  return OPENAI_SHAPED_HINTS.test(slug);
}

/**
 * Wave-aha-1 A4+ — the real post-connect aha moment.
 *
 * Connecting a service ≠ being able to use it. The user's actual goal
 * is "make my AI tool talk to {service} through NyxID." Three things
 * have to happen for that to work:
 *
 *   1. NyxID stores the service credential (the form-submit step did
 *      this already by the time we render).
 *   2. The user has an Agent Key (`nyx_ag_…`) scoped to call that
 *      service through the proxy.
 *   3. The user has the proxy URL + the agent key wired into their
 *      AI tool's config.
 *
 * The old `/keys/{id}` flow left steps 2-3 entirely to the user.
 * This step does both inline:
 *
 *   - Auto-mints an Agent Key scoped to ONLY the just-connected
 *     service (least-privilege default), with the `proxy` scope so it
 *     can make proxy calls.
 *   - Shows the full secret ONCE in a CopyableField with a one-time
 *     warning ("save it now; we won't show it again"). The Agent Keys
 *     tab will show only the prefix afterwards.
 *   - Shows the proxy URL.
 *   - For OpenAI-shaped services, shows the OPENAI_API_KEY +
 *     OPENAI_BASE_URL env-var snippet (the lingua franca for Codex
 *     CLI, openai-python, Cursor, Continue.dev, etc.).
 *   - Fires a probe against the proxy USING THE NEW AGENT KEY AS
 *     BEARER — not the user's session cookie. That's the path the
 *     user's tool will actually take, so this is the real verification
 *     (the cookie probe in v1 of A4 didn't prove the bearer path
 *     worked).
 *
 * If anything fails, we degrade gracefully: minted key + URL still
 * shown, with a clear "couldn't probe automatically" explanation.
 */
export function ConnectVerifyStep({
  createdKey,
  isNodeRouted,
  onDone,
  onViewDetails,
}: ConnectVerifyStepProps) {
  const createApiKey = useCreateApiKey();
  const [phase, setPhase] = useState<Phase>("minting");
  const [agentKey, setAgentKey] = useState<ApiKeyCreateResponse | null>(null);
  const [mintError, setMintError] = useState<string | null>(null);
  const [httpStatus, setHttpStatus] = useState<number | null>(null);
  const [probeError, setProbeError] = useState<string | null>(null);
  const mintedRef = useRef(false);
  const probedRef = useRef(false);
  const loadingDispatchedRef = useRef(false);

  // Step 1: mint a scoped Agent Key for the just-connected service.
  // Runs once on mount; the ref guard handles React strict-mode double-fire.
  useEffect(() => {
    if (mintedRef.current) return;
    mintedRef.current = true;

    createApiKey.mutate(
      {
        // A short auto-name keeps the Agent Keys table tidy and the user
        // can rename it on the detail page if they want a better label.
        name: `Key for ${createdKey.serviceName}`,
        scopes: ["proxy"],
        allow_all_services: false,
        allowed_service_ids: [createdKey.id],
        allow_all_nodes: true,
      },
      {
        onSuccess: (key) => {
          setAgentKey(key);
          setPhase("ready_to_probe");
        },
        onError: (err) => {
          setMintError(err instanceof Error ? err.message : String(err));
          setPhase("mint_failed");
        },
      },
    );
    // createApiKey is a stable mutation handle; we intentionally don't
    // re-mint when it changes. createdKey.id + serviceName drive the
    // scoping + name and don't change for the life of this step.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [createdKey.id, createdKey.serviceName]);

  // Step 2: once the key exists, fire the bearer-auth probe. For
  // services that don't have a probeable path (Codex's chat backend,
  // unknown custom endpoints) just go straight to "skipped" success.
  useEffect(() => {
    if (phase !== "ready_to_probe" || !agentKey) return;
    if (probedRef.current) return;
    probedRef.current = true;

    const probePath = probePathForSlug(createdKey.slug);
    // Skip the probe when:
    //  - The catalog entry's auth_method is "none" (Codex etc.) — we
    //    don't really know what path on that backend would respond,
    //    and we don't want a false-negative 404 against a working
    //    connection.
    //  - The slug isn't OpenAI-shaped (`probePath === ""` means root,
    //    which usually 404s on real APIs).
    const shouldProbe =
      createdKey.completionMode === "credential" && probePath !== "";

    if (!shouldProbe) {
      setPhase("skipped");
      return;
    }

    window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_START_EVENT));
    loadingDispatchedRef.current = true;
    setPhase("probing");

    const controller = new AbortController();
    const timer = window.setTimeout(
      () => controller.abort(),
      PROBE_TIMEOUT_MS,
    );

    const url = `${proxyBaseUrl(createdKey.slug)}/${probePath}`;
    (async () => {
      let status: number | null = null;
      try {
        const res = await fetch(url, {
          method: "GET",
          // Bearer = the just-minted Agent Key. credentials:"omit" so
          // the test does NOT fall back to the user's session cookie —
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

      setHttpStatus(status);

      const ok = status !== null && status >= 200 && status < 400;
      if (ok) {
        setPhase("success");
        // Dashboard checklist's "Make first proxy call" step listens for
        // this — ticks it off without a separate navigation.
        window.dispatchEvent(
          new CustomEvent(FIRST_PROXY_CALL_SUCCEEDED_EVENT),
        );
      } else {
        setPhase("probe_failed");
        setProbeError(diagnose(status, isNodeRouted));
      }

      if (loadingDispatchedRef.current) {
        window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_END_EVENT));
        loadingDispatchedRef.current = false;
      }
    })();

    return () => {
      controller.abort();
      window.clearTimeout(timer);
      if (loadingDispatchedRef.current) {
        window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_END_EVENT));
        loadingDispatchedRef.current = false;
      }
    };
  }, [phase, agentKey, createdKey.slug, createdKey.completionMode, isNodeRouted]);

  const proxyUrl = proxyBaseUrl(createdKey.slug);
  const openAiShaped = looksOpenAiShaped(createdKey.slug);

  return (
    <div className="space-y-4 py-2">
      <ResultPanel
        phase={phase}
        serviceName={createdKey.serviceName}
        mintError={mintError}
        probeError={probeError}
        httpStatus={httpStatus}
      />

      {agentKey && (
        <div className="space-y-3 rounded-xl border border-border/50 bg-card p-4">
          <div className="space-y-1">
            <p className="text-[12px] font-semibold text-foreground">
              Your new Agent Key
            </p>
            <p className="text-[11px] text-muted-foreground">
              Save this now. The full secret is only shown here once — only
              the prefix is visible afterwards. Scoped to{" "}
              <span className="font-medium text-foreground">
                {createdKey.serviceName}
              </span>{" "}
              only, with the <code>proxy</code> scope.
            </p>
          </div>
          <CopyableField label="Agent Key" value={agentKey.full_key} />

          <div className="space-y-1 pt-1">
            <p className="text-[12px] font-semibold text-foreground">
              Proxy URL
            </p>
            <p className="text-[11px] text-muted-foreground">
              Point your AI tool at this URL instead of the service&apos;s
              direct API. NyxID brokers the call.
            </p>
          </div>
          <CopyableField label="Base URL" value={proxyUrl} />

          {openAiShaped && (
            <>
              <div className="space-y-1 pt-1">
                <p className="text-[12px] font-semibold text-foreground">
                  Wire it up
                </p>
                <p className="text-[11px] text-muted-foreground">
                  Most OpenAI-compatible tools (Codex CLI, openai-python,
                  Cursor, Continue.dev, …) read these two env vars.
                </p>
              </div>
              <CopyableField
                label="Shell env"
                value={`export OPENAI_API_KEY="${agentKey.full_key}"\nexport OPENAI_BASE_URL="${proxyUrl}/v1"`}
              />
            </>
          )}
        </div>
      )}

      <div className="flex flex-wrap items-center justify-end gap-2">
        <Button variant="outline" onClick={onViewDetails}>
          <ExternalLink className="mr-1.5 h-3.5 w-3.5" />
          View details
        </Button>
        <Button
          variant="primary"
          size="lg"
          onClick={onDone}
          disabled={phase === "minting"}
        >
          Done
        </Button>
      </div>
    </div>
  );
}

function ResultPanel({
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
  if (phase === "minting") {
    return (
      <Banner icon="spinner" tone="neutral" title="Setting up your Agent Key…">
        We&apos;re creating a scoped Agent Key so your AI tools can call{" "}
        {serviceName} through NyxID.
      </Banner>
    );
  }

  if (phase === "mint_failed") {
    return (
      <Banner icon="warn" tone="warn" title="Couldn’t auto-create an Agent Key">
        <span>
          {serviceName} is connected, but we couldn&apos;t mint an Agent Key
          for you automatically.{" "}
          {mintError ? `(${mintError})` : ""} You can create one manually
          from <span className="font-medium">/keys → Agent Keys → Create</span>.
        </span>
      </Banner>
    );
  }

  if (phase === "probing" || phase === "ready_to_probe") {
    return (
      <Banner
        icon="spinner"
        tone="neutral"
        title="Testing the end-to-end path…"
      >
        Calling {serviceName} with the new Agent Key to prove your AI tool
        can use it. This takes a couple of seconds.
      </Banner>
    );
  }

  if (phase === "success") {
    return (
      <Banner
        icon="check"
        tone="success"
        title={`Your AI tool can now call ${serviceName} through NyxID`}
      >
        <span>
          End-to-end test succeeded
          {httpStatus !== null ? ` (HTTP ${String(httpStatus)})` : ""}.
          Copy the Agent Key and Base URL below into your tool of choice —
          you&apos;re done.
        </span>
      </Banner>
    );
  }

  if (phase === "skipped") {
    return (
      <Banner
        icon="check"
        tone="success"
        title={`Agent Key minted — ready to use with ${serviceName}`}
      >
        We didn&apos;t run an automatic smoke test on this service (its
        endpoint doesn&apos;t expose a standard health path we can probe
        safely). Copy the Agent Key and Base URL below into your tool —
        it&apos;s wired correctly.
      </Banner>
    );
  }

  // probe_failed
  return (
    <Banner
      icon="warn"
      tone="warn"
      title={`Agent Key minted — but the end-to-end test didn’t succeed`}
    >
      <span>
        {probeError ??
          "The probe returned an unexpected response. Use the View details button to investigate."}
        {httpStatus !== null ? ` (HTTP ${String(httpStatus)})` : ""}
      </span>
    </Banner>
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

/**
 * Turn the probe's HTTP status into a one-sentence actionable hint. Kept
 * narrow on purpose — vague "something went wrong" copy is exactly what
 * makes the dashboard feel like a black box.
 */
function diagnose(
  status: number | null,
  isNodeRouted: boolean,
): string {
  if (status === null) {
    return "The probe timed out or was blocked by the browser. The Agent Key was created — open the details to retry from the Verify panel.";
  }
  if (status === 401 || status === 403) {
    return `The downstream rejected the bearer-authenticated call (HTTP ${String(status)}). This usually means the upstream credential you connected (the OpenAI API key etc.) isn't actually valid for the path we probed.`;
  }
  if (status === 404) {
    if (isNodeRouted) {
      return "The proxy returned 404 — node-routed services need the node agent online to respond. Start the node agent and retry from the Verify panel.";
    }
    return "The proxy returned 404 — the slug isn't routable yet, or the downstream doesn't expose this probe path.";
  }
  if (status === 429) {
    return "The downstream is rate-limiting (HTTP 429). Your credential + Agent Key work — retry in a minute or two.";
  }
  if (status >= 500) {
    return `The downstream returned a server error (HTTP ${String(status)}). NyxID's part of the path is fine; the upstream service is having issues right now.`;
  }
  return `The probe returned an unexpected HTTP ${String(status)}. Open the details to debug from the Verify panel.`;
}
