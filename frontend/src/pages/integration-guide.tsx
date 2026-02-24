import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  OAUTH_SCOPE_META,
  scopeRiskClass,
  scopeRiskLabel,
} from "@/lib/constants";

function CodeBlock({
  label,
  code,
}: {
  readonly label: string;
  readonly code: string;
}) {
  return (
    <div className="space-y-2">
      <Badge variant="outline" className="text-[10px]">
        {label}
      </Badge>
      <pre className="overflow-x-auto rounded-lg border border-border bg-muted px-4 py-3 font-mono text-xs leading-relaxed text-foreground">
        {code}
      </pre>
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
  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="flex items-center gap-3 text-base">
          <span className="inline-flex h-6 w-6 items-center justify-center rounded-full bg-primary text-xs font-semibold text-primary-foreground">
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
  return (
    <div className="space-y-8">
      <div className="space-y-2">
        <h2 className="font-display text-5xl font-normal tracking-tight">
          Integration Guide
        </h2>
        <p className="text-muted-foreground">
          Use the official React or Core SDK (recommended), or integrate with
          raw OAuth endpoints for custom flows.
        </p>
      </div>

      <Tabs defaultValue="react" className="space-y-6">
        <TabsList>
          <TabsTrigger value="react">React SDK</TabsTrigger>
          <TabsTrigger value="core">Core SDK</TabsTrigger>
          <TabsTrigger value="raw">Raw API</TabsTrigger>
        </TabsList>

        <TabsContent value="react" className="space-y-4">
          <StepCard index={1} title="Install React SDK">
            <CodeBlock label="Install" code={reactInstall} />
          </StepCard>
          <StepCard index={2} title="Wrap your app with provider">
            <CodeBlock label="Provider setup" code={reactProvider} />
          </StepCard>
          <StepCard index={3} title="Trigger login flow">
            <CodeBlock label="Login button" code={reactLogin} />
          </StepCard>
        </TabsContent>

        <TabsContent value="core" className="space-y-4">
          <StepCard index={1} title="Install Core SDK">
            <CodeBlock label="Install" code={coreInstall} />
          </StepCard>
          <StepCard index={2} title="Create client and start PKCE flow">
            <CodeBlock label="Client + redirect" code={corePkce} />
          </StepCard>
          <StepCard index={3} title="Handle callback and load user">
            <CodeBlock label="Callback handling" code={coreCallback} />
          </StepCard>
        </TabsContent>

        <TabsContent value="raw" className="space-y-4">
          <StepCard index={1} title="Redirect user to authorize endpoint">
            <CodeBlock label="GET /oauth/authorize" code={rawAuthorize} />
          </StepCard>
          <StepCard index={2} title="Exchange authorization code for tokens">
            <CodeBlock label="POST /oauth/token" code={rawToken} />
          </StepCard>
          <StepCard index={3} title="Use access token to call APIs">
            <CodeBlock label="GET /api/v1/users/me" code={rawUserInfo} />
          </StepCard>
        </TabsContent>
      </Tabs>

      <Card>
        <CardHeader>
          <CardTitle>Scope Reference</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          {Object.entries(OAUTH_SCOPE_META).map(([scope, meta]) => (
            <div
              key={scope}
              className="rounded-md border border-border bg-muted/50 px-3 py-2"
            >
              <div className="flex items-center justify-between gap-2">
                <p className="font-mono text-xs text-foreground">{scope}</p>
                <span
                  className={`rounded-full border px-2 py-0.5 text-[10px] font-medium ${scopeRiskClass(meta.risk)}`}
                >
                  {scopeRiskLabel(meta.risk)}
                </span>
              </div>
              <p className="mt-1 text-xs text-muted-foreground">
                <span className="font-medium text-foreground">{meta.title}</span>
                {" - "}
                {meta.description}
              </p>
            </div>
          ))}
        </CardContent>
      </Card>
    </div>
  );
}
