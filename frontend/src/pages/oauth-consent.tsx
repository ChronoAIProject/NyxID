import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { PortalMarkLogo } from "@/components/shared/portal-mark-logo";
import { AlertTriangle, ShieldCheck } from "lucide-react";
import {
  OAUTH_SCOPE_META,
  scopeRiskClass,
  scopeRiskLabel,
} from "@/lib/constants";

function readParam(search: URLSearchParams, key: string): string {
  return search.get(key) ?? "";
}

function parseHost(uri: string): string {
  try {
    return new URL(uri).host;
  } catch {
    return "Unknown";
  }
}

export function OAuthConsentPage() {
  const search = new URLSearchParams(window.location.search);

  const responseType = readParam(search, "response_type");
  const clientId = readParam(search, "client_id");
  const clientName = readParam(search, "client_name") || clientId;
  const redirectUri = readParam(search, "redirect_uri");
  const scope = readParam(search, "scope");
  const state = search.get("state") ?? "";
  const codeChallenge = readParam(search, "code_challenge");
  const codeChallengeMethod = readParam(search, "code_challenge_method");
  const nonce = search.get("nonce") ?? "";
  const prompt = search.get("prompt") ?? "";

  const missing =
    !responseType ||
    !clientId ||
    !redirectUri ||
    !scope ||
    !codeChallenge ||
    !codeChallengeMethod;

  const scopes = scope.split(/\s+/).filter(Boolean);
  const redirectHost = parseHost(redirectUri);

  if (missing) {
    return (
      <div
        className="mx-auto flex min-h-dvh w-full max-w-2xl items-center px-6 py-10"
        style={{ paddingTop: "max(2.5rem, var(--sat))", paddingBottom: "max(2.5rem, var(--sab))" }}
      >
        <Card className="w-full">
          <CardHeader>
            <CardTitle>Invalid consent request</CardTitle>
            <CardDescription>
              Missing required OAuth parameters. Please restart the sign-in flow.
            </CardDescription>
          </CardHeader>
        </Card>
      </div>
    );
  }

  return (
    <div
      className="mx-auto flex min-h-dvh w-full max-w-2xl items-center px-6 py-10"
      style={{ paddingTop: "max(2.5rem, var(--sat))", paddingBottom: "max(2.5rem, var(--sab))" }}
    >
      <Card className="w-full">
        <CardHeader className="space-y-4">
          <div className="flex items-center gap-3">
            <PortalMarkLogo size={26} />
            <p className="logo-wordmark text-xl">NyxID</p>
          </div>
          <CardTitle>Authorize Application</CardTitle>
          <CardDescription>
            <span className="font-medium text-foreground">{clientName}</span>{" "}
            wants to access your account via OAuth.
          </CardDescription>
        </CardHeader>

        <CardContent className="space-y-6">
          <div className="rounded-md border border-border bg-muted px-4 py-3">
            <div className="flex items-center gap-2 text-sm font-medium text-foreground">
              <ShieldCheck className="h-4 w-4 text-primary" />
              App verification details
            </div>
            <div className="mt-2 space-y-1 text-xs text-muted-foreground">
              <p>
                Application:{" "}
                <span className="font-medium text-foreground">{clientName}</span>
              </p>
              <p>
                Redirect host:{" "}
                <span className="font-mono text-foreground">{redirectHost}</span>
              </p>
            </div>
          </div>

          <div className="space-y-2">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">
              Requested scopes
            </p>
            <div className="flex flex-wrap gap-2">
              {scopes.map((item) => (
                <Badge key={item} variant="outline">
                  {item}
                </Badge>
              ))}
            </div>
          </div>

          <div className="space-y-2">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">
              Scope impact
            </p>
            <div className="space-y-2">
              {scopes.map((item) => {
                const meta = OAUTH_SCOPE_META[item] ?? {
                  title: "Custom permission",
                  description:
                    "This app is requesting a non-standard permission.",
                  risk: "medium" as const,
                };
                return (
                  <div
                    key={`meta-${item}`}
                    className="rounded-md border border-border bg-muted/50 px-3 py-2"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <p className="font-mono text-xs text-foreground">{item}</p>
                      <span
                        className={`rounded-full border px-2 py-0.5 text-[10px] font-medium ${scopeRiskClass(meta.risk)}`}
                      >
                        {scopeRiskLabel(meta.risk)}
                      </span>
                    </div>
                    <p className="mt-1 text-xs text-muted-foreground">
                      <span className="font-medium text-foreground">
                        {meta.title}
                      </span>
                      {" - "}
                      {meta.description}
                    </p>
                  </div>
                );
              })}
            </div>
          </div>

          <div className="space-y-1">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">
              Client ID
            </p>
            <p className="font-mono text-xs text-foreground">{clientId}</p>
          </div>

          <div className="space-y-1">
            <p className="text-xs uppercase tracking-wide text-text-tertiary">
              Redirect URI
            </p>
            <p className="break-all font-mono text-xs text-foreground">
              {redirectUri}
            </p>
          </div>

          <div className="rounded-md border border-yellow-500/30 bg-yellow-500/10 px-4 py-3">
            <div className="flex items-start gap-2">
              <AlertTriangle className="mt-0.5 h-4 w-4 text-yellow-300" />
              <p className="text-xs text-yellow-100/90">
                Only continue if you trust this application. You can revoke this
                access later from <strong>Authorized Applications</strong>.
              </p>
            </div>
          </div>

          <form
            method="POST"
            action="/oauth/authorize/decision"
            className="flex flex-wrap items-center gap-3 pt-1"
          >
            <input type="hidden" name="response_type" value={responseType} />
            <input type="hidden" name="client_id" value={clientId} />
            <input type="hidden" name="redirect_uri" value={redirectUri} />
            <input type="hidden" name="scope" value={scope} />
            <input type="hidden" name="state" value={state} />
            <input type="hidden" name="code_challenge" value={codeChallenge} />
            <input
              type="hidden"
              name="code_challenge_method"
              value={codeChallengeMethod}
            />
            <input type="hidden" name="nonce" value={nonce} />
            {prompt && <input type="hidden" name="prompt" value={prompt} />}

            <Button
              type="submit"
              variant="outline"
              name="decision"
              value="deny"
            >
              Deny
            </Button>
            <Button type="submit" name="decision" value="allow">
              Allow
            </Button>
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
