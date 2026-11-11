import { translateUi, useUiTranslation } from "@/i18n/use-ui-translation";

import { useCallback } from "react";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { Copy } from "lucide-react";
import { toast } from "sonner";
import {
  OAUTH_SCOPE_META,
  scopeRiskBadgeVariant,
  scopeRiskLabel,
} from "@/lib/constants";
import {
  INTEGRATION_GUIDE_TABS,
  INTEGRATION_GUIDE_TAB_DEFAULT,
  parseTab,
} from "@/lib/url-tabs";
import { PageHeader } from "@/components/shared/page-header";

function CodeBlock({
  label,
  code,
}: {
  readonly label: string;
  readonly code: string;
}) {
  const { t } = useUiTranslation();
  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(code);
      toast.success(translateUi("Copied to clipboard"));
    } catch {
      toast.error(translateUi("Failed to copy"));
    }
  }, [code]);

  return (
    <div className="space-y-2">
      <Badge variant="secondary" className="text-12">
        {label}
      </Badge>
      <div className="relative">
        <pre className="whitespace-pre-wrap break-all rounded-lg border border-border bg-muted px-4 py-3.5 pr-12 min-h-[44px] font-mono text-14 text-foreground">
          {code}
        </pre>
        <Button
          variant="ghost"
          size="icon"
          className="absolute right-2 top-2 h-8 w-8 text-text-tertiary hover:text-foreground"
          onClick={() => void handleCopy()}
          aria-label={t("Copy")}
        >
          <Copy className="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>
  );
}

function StepCard({
  index,
  title,
  children,
}: {
  readonly index: number;
  readonly title: string;
  readonly children: React.ReactNode;
}) {
  useUiTranslation();
  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-3">
          <span className="inline-flex h-6 w-6 items-center justify-center rounded-full bg-primary type-secondary-strong text-primary-foreground">
            {index}
          </span>
          {title}
        </CardTitle>
      </CardHeader>
      <CardContent>{children}</CardContent>
    </Card>
  );
}

const reactInstall = `npm install @nyxid/oauth-react @nyxid/oauth-core`;
const reactProvider = `import { NyxIDProvider, createNyxClient } from "@nyxid/oauth-react";

const nyxClient = createNyxClient({
  baseUrl: "https://your-nyxid-domain.com",
  clientId: "your-client-id",
  redirectUri: window.location.origin + "/auth/callback",
});

root.render(
  <NyxIDProvider client={nyxClient}>
    <App />
  </NyxIDProvider>,
);`;
const reactLogin = `import { useNyxID } from "@nyxid/oauth-react";

export function LoginButton() {
  const { loginWithRedirect } = useNyxID();
  return <button onClick={() => void loginWithRedirect()}>Sign in</button>;
}`;

const coreInstall = `npm install @nyxid/oauth-core`;
const corePkce = `import { NyxIDClient } from "@nyxid/oauth-core";

const client = new NyxIDClient({
  baseUrl: "https://your-nyxid-domain.com",
  clientId: "your-client-id",
  redirectUri: "https://your-app.com/callback",
});

await client.loginWithRedirect();`;
const coreCallback = `const tokens = await client.handleRedirectCallback(window.location.href);
const profile = await client.getUserInfo(tokens.accessToken);`;

const rawAuthorize = `GET /oauth/authorize?
  response_type=code&
  client_id=your-client-id&
  redirect_uri=https://your-app.com/callback&
  scope=openid profile email&
  code_challenge=...&
  code_challenge_method=S256&
  state=...`;
const rawToken = `POST /oauth/token
Content-Type: application/x-www-form-urlencoded

grant_type=authorization_code&
code=AUTH_CODE&
redirect_uri=https://your-app.com/callback&
client_id=your-client-id&
code_verifier=...`;
const rawUserInfo = `GET /api/v1/users/me
Authorization: Bearer ACCESS_TOKEN`;

export function IntegrationGuidePage() {
  const { t } = useUiTranslation();
  const searchParams = useRouterState({ select: (s) => s.location.search as Record<string, unknown> });
  const navigate = useNavigate();
  const currentTab = parseTab(
    searchParams.tab,
    INTEGRATION_GUIDE_TABS,
    INTEGRATION_GUIDE_TAB_DEFAULT,
  );

  function handleTabChange(value: string) {
    void navigate({ to: "/integration-guide", search: { tab: value }, replace: true });
  }

  return (
    <div className="space-y-8">
      <PageHeader
        title={t("Integration & SDK Guide")}
        description={t("Use the official React or Core SDK (recommended), or integrate with raw OAuth endpoints for custom flows.")}
      />

      <Tabs value={currentTab} onValueChange={handleTabChange} className="space-y-6">
        <TabsList>
          <TabsTrigger value="react">{t("React SDK")}</TabsTrigger>
          <TabsTrigger value="core">{t("Core SDK")}</TabsTrigger>
          <TabsTrigger value="raw">{t("Raw API")}</TabsTrigger>
        </TabsList>

        <TabsContent value="react" className="space-y-4">
          <StepCard index={1} title={t("Install React SDK")}>
            <CodeBlock label={t("Install")} code={reactInstall} />
          </StepCard>
          <StepCard index={2} title={t("Wrap your app with provider")}>
            <CodeBlock label={t("Provider setup")} code={reactProvider} />
          </StepCard>
          <StepCard index={3} title={t("Trigger login flow")}>
            <CodeBlock label={t("Login button")} code={reactLogin} />
          </StepCard>
        </TabsContent>

        <TabsContent value="core" className="space-y-4">
          <StepCard index={1} title={t("Install Core SDK")}>
            <CodeBlock label={t("Install")} code={coreInstall} />
          </StepCard>
          <StepCard index={2} title={t("Create client and start PKCE flow")}>
            <CodeBlock label={t("Client + redirect")} code={corePkce} />
          </StepCard>
          <StepCard index={3} title={t("Handle callback and load user")}>
            <CodeBlock label={t("Callback handling")} code={coreCallback} />
          </StepCard>
        </TabsContent>

        <TabsContent value="raw" className="space-y-4">
          <StepCard index={1} title={t("Redirect user to authorize endpoint")}>
            <CodeBlock label={"GET /oauth/authorize"} code={rawAuthorize} />
          </StepCard>
          <StepCard index={2} title={t("Exchange authorization code for tokens")}>
            <CodeBlock label={"POST /oauth/token"} code={rawToken} />
          </StepCard>
          <StepCard index={3} title={t("Use access token to call APIs")}>
            <CodeBlock label={"GET /api/v1/users/me"} code={rawUserInfo} />
          </StepCard>
        </TabsContent>
      </Tabs>

      <Card>
        <CardHeader>
          <CardTitle>{t("Scope Reference")}</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          {Object.entries(OAUTH_SCOPE_META).map(([scope, meta]) => (
            <div
              key={scope}
              className="flex items-center gap-4 rounded-lg border border-border bg-muted/50 px-3 py-2.5"
            >
              <div className="min-w-0 flex-1">
                <p className="text-14 text-foreground break-all">{scope}</p>
                <p className="text-13 text-muted-foreground">
                  <span className="font-medium text-foreground">
                    {meta.title}
                  </span>
                  {" - "}
                  {meta.description}
                </p>
              </div>
              <Badge
                variant={scopeRiskBadgeVariant(meta.risk)}
                className="shrink-0"
              >
                {scopeRiskLabel(meta.risk)}
              </Badge>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}
