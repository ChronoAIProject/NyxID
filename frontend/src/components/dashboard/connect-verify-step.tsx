import { useEffect, useRef, useState } from "react";
import { Check, ExternalLink, Loader2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  FIRST_PROXY_CALL_SUCCEEDED_EVENT,
  VERIFY_KEY_LOADING_END_EVENT,
  VERIFY_KEY_LOADING_START_EVENT,
} from "@/hooks/use-proxy-onboarding";

/**
 * Slugs that almost always expose `/v1/models` on AI providers. Used to
 * decide whether the probe path should be `v1/models` (OpenAI-shaped API)
 * or `` (treat the downstream's root path). Mirrors the same heuristic
 * VerifyKeyCard uses on the key-detail page.
 */
const OPENAI_SHAPED_HINTS =
  /(openai|anthropic|claude|gemini|deepseek|groq|together|mistral|fireworks|perplexity|cohere|xai|grok)/i;

const PROBE_TIMEOUT_MS = 8000;

type VerifyStatus = "pending" | "success" | "failure";

interface CreatedKey {
  readonly id: string;
  readonly slug: string;
  readonly serviceName: string;
}

interface ConnectVerifyStepProps {
  readonly createdKey: CreatedKey;
  readonly isNodeRouted: boolean;
  /**
   * When false (catalog entries with `auth_method=none`, e.g.
   * `llm-openai-codex` whose base_url is the ChatGPT backend), there's
   * literally no credential to test, so the probe would either time out
   * or come back with a misleading 4xx. Skip it and show a "no creds
   * needed" success state instead.
   */
  readonly requiresCredential: boolean;
  readonly onDone: () => void;
  readonly onViewDetails: () => void;
}

function probePathForSlug(slug: string): string {
  return OPENAI_SHAPED_HINTS.test(slug) ? "v1/models" : "";
}

/**
 * Wave-aha-1 A4 — inline post-connect verification step.
 *
 * Auto-fires a single real proxy call against the just-connected service
 * using the user's session cookie. The user sees their first 200 (or a
 * precise failure diagnosis) inside the dialog instead of having to
 * navigate to /keys/{id} → API Usage → Verify panel.
 *
 * Unlike `<VerifyKeyCard>` (which verifies an Agent Key's *scope* by
 * asking the user to paste a key), this step verifies the *service
 * credential* itself: the user just provided their OpenAI key, did
 * NyxID's broker actually call OpenAI with it? That's the question
 * most blocking the aha moment.
 */
export function ConnectVerifyStep({
  createdKey,
  isNodeRouted,
  requiresCredential,
  onDone,
  onViewDetails,
}: ConnectVerifyStepProps) {
  // Default to "success" with a no-creds note when there's nothing to
  // test. Skipping the probe entirely is the only honest UX — running a
  // probe against an `auth_method=none` service would either time out
  // or land a 4xx from a path that doesn't exist on that backend, and
  // the user would see misleading "Credential rejected" copy for a
  // service that literally has no credential.
  const [status, setStatus] = useState<VerifyStatus>(
    requiresCredential ? "pending" : "success",
  );
  const [httpStatus, setHttpStatus] = useState<number | null>(null);
  const [errorHint, setErrorHint] = useState<string | null>(null);
  const firedRef = useRef(false);
  const loadingDispatchedRef = useRef(false);

  // Single-shot probe on mount, only for services with real credentials.
  // The guard avoids React-strict-mode double-fire and any other
  // rerender re-triggering the call.
  useEffect(() => {
    if (!requiresCredential) return;
    if (firedRef.current) return;
    firedRef.current = true;

    window.dispatchEvent(new CustomEvent(VERIFY_KEY_LOADING_START_EVENT));
    loadingDispatchedRef.current = true;

    const controller = new AbortController();
    const timer = window.setTimeout(
      () => controller.abort(),
      PROBE_TIMEOUT_MS,
    );

    const url = `/api/v1/proxy/s/${encodeURIComponent(createdKey.slug)}/${probePathForSlug(createdKey.slug)}`;

    (async () => {
      let probeStatus: number | null = null;
      try {
        const res = await fetch(url, {
          method: "GET",
          // Use the user's session cookie — this verifies the SERVICE
          // credential the user just submitted, not any agent key scope.
          credentials: "include",
          headers: { "Content-Type": "application/json" },
          signal: controller.signal,
        });
        probeStatus = res.status;
      } catch {
        probeStatus = null;
      } finally {
        window.clearTimeout(timer);
      }

      setHttpStatus(probeStatus);

      const ok =
        probeStatus !== null && probeStatus >= 200 && probeStatus < 400;
      if (ok) {
        setStatus("success");
        // Notify the dashboard checklist that the first proxy call landed,
        // so the "Make first proxy call" step ticks off too.
        window.dispatchEvent(
          new CustomEvent(FIRST_PROXY_CALL_SUCCEEDED_EVENT),
        );
      } else {
        setStatus("failure");
        setErrorHint(diagnose(probeStatus, isNodeRouted));
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
  }, [createdKey.slug, isNodeRouted, requiresCredential]);

  return (
    <div className="space-y-5 py-2">
      <div className="rounded-xl border border-border/50 bg-card p-5">
        {status === "pending" && (
          <div className="flex items-center gap-3">
            <Loader2 className="h-5 w-5 animate-spin text-muted-foreground" />
            <div className="space-y-0.5">
              <p className="text-[13px] font-semibold text-foreground">
                Testing connection to {createdKey.serviceName}…
              </p>
              <p className="text-[12px] text-muted-foreground">
                NyxID is calling the downstream once to make sure your
                credential works.
              </p>
            </div>
          </div>
        )}

        {status === "success" && (
          <div className="flex items-start gap-3">
            <span className="mt-0.5 inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-success/20">
              <Check className="h-4 w-4 text-success" aria-hidden />
            </span>
            <div className="space-y-1">
              <p className="text-[13px] font-semibold text-foreground">
                {requiresCredential
                  ? `Verified — your ${createdKey.serviceName} credential works`
                  : `${createdKey.serviceName} connected — ready to use`}
              </p>
              <p className="text-[12px] text-muted-foreground">
                {requiresCredential
                  ? `NyxID can now broker calls to ${createdKey.serviceName} on your behalf. Your AI agents and downstream tools can use this connection without ever seeing the raw key.`
                  : `${createdKey.serviceName} doesn't require credentials — NyxID can route calls to it immediately.`}
              </p>
              {requiresCredential && httpStatus !== null && (
                <p className="text-[11px] text-muted-foreground">
                  Probe returned HTTP {String(httpStatus)}.
                </p>
              )}
            </div>
          </div>
        )}

        {status === "failure" && (
          <div className="flex items-start gap-3">
            <span className="mt-0.5 inline-flex h-6 w-6 shrink-0 items-center justify-center rounded-full bg-warning/20">
              <X className="h-4 w-4 text-warning" aria-hidden />
            </span>
            <div className="space-y-1.5">
              <p className="text-[13px] font-semibold text-foreground">
                {createdKey.serviceName} connected, but the test call didn&apos;t succeed
              </p>
              <p className="text-[12px] text-muted-foreground">
                {errorHint ??
                  "We couldn't reach the downstream. You can still continue — open the service details to debug."}
              </p>
              {httpStatus !== null && (
                <p className="text-[11px] text-muted-foreground">
                  Probe returned HTTP {String(httpStatus)}.
                </p>
              )}
            </div>
          </div>
        )}
      </div>

      <div className="flex flex-wrap items-center justify-end gap-2">
        <Button variant="outline" onClick={onViewDetails}>
          <ExternalLink className="mr-1.5 h-3.5 w-3.5" />
          View details
        </Button>
        <Button variant="primary" size="lg" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  );
}

/**
 * Turn the probe's HTTP status into a one-sentence hint the user can act
 * on. Kept narrow on purpose — vague "something went wrong" copy is
 * exactly what makes the dashboard feel like a black box. If the status
 * doesn't match a known pattern, fall back to the "open details to
 * debug" message; better silence than wrong guesses.
 */
function diagnose(
  status: number | null,
  isNodeRouted: boolean,
): string {
  if (status === null) {
    return "The probe timed out or was blocked by the browser. The service was created — open the details to retry from the Verify panel.";
  }
  if (status === 401 || status === 403) {
    return "The downstream rejected the credential (HTTP " +
      String(status) +
      "). Most likely the API key was typed incorrectly, has been revoked, or doesn't have the scope this probe needs. Try the Verify panel on the detail page with the exact key you used.";
  }
  if (status === 404) {
    if (isNodeRouted) {
      return "The proxy didn't find a route — node-routed services need the node agent online to respond. Start the node agent and retry from the Verify panel.";
    }
    return "The proxy returned 404 — the slug isn't registered as a user-service for this account, or the downstream doesn't expose this probe path. Open the details to inspect.";
  }
  if (status === 429) {
    return "The downstream is rate-limiting (HTTP 429). Your credential works — retry in a minute or two.";
  }
  if (status >= 500) {
    return "The downstream returned a server error (HTTP " +
      String(status) +
      "). NyxID's part of the path is fine; the upstream service is having issues right now.";
  }
  return "The probe returned an unexpected HTTP " +
    String(status) +
    ". Open the details to debug from the Verify panel.";
}
