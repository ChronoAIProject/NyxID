---
title: Your first Studio connection
description: Connect GitHub in Studio, scope an Agent Key to that connection and use it to read your GitHub username.
---

## Hosted (recommended for first-time users)

### 1. Sign in

Sign in to NyxID with the account that will own the connection. If your instance requires an invitation to register, use the code supplied by your workspace. You also need a GitHub account you can authorize. [Register with an invitation or recover a first sign-in](/docs/web/getting-started/sign-up#create-an-account)

### 2. Add an AI Service

1. In the NyxBot account menu choose **Open Studio**, then **AI Services**. The page is titled **Services & Credentials**. [Open the services dashboard](/keys?tab=services)
2. Under **External Services**, choose **Connect Service** and select **GitHub OAuth**. For a personal connection, select your own account as owner. For a team connection, select an organization where your role allows you to connect services. [Organization access](/docs/web/guides/organizations)
3. Continue to GitHub, check the GitHub account and requested permissions, and authorize the connection. If Studio asks for a client ID and secret, follow the OAuth app setup below. [Configure a provider app](#self-host)
4. Return to **External Services** and copy the slug from your connected entry. A first GitHub OAuth connection usually has the slug `api-github`. Additional connections can have a suffix.

### 3. Create a NyxID Agent Key

Open **Agent Keys** on the same page. Choose **Create API Key** and give the key a recognizable name, such as `github-profile-check`. Select the same owner as the connection, select the `proxy` scope, leave **Allow all services** off and choose only the GitHub connection. Create the key and save the full value privately when it appears.

The service selection limits which connection the key can use. The example below reads a profile. Other operations on that connection may also be authorized by the key's service permissions, the GitHub credential and the applicable approval policies. For an existing key, inspect **Service Scope** in its details.

### 4. Run the verification curl

In Bash, set the slug to the value you copied. Enter the full **NyxID Agent Key** at the hidden prompt. The GitHub credential is already stored with the connection.

```bash
NYXID_API_ORIGIN='https://nyx-api.chrono-ai.fun'
SERVICE_SLUG='api-github'
read -rs -p 'NyxID Agent Key: ' AGENT_KEY; echo
curl --fail-with-body -sS -H "X-API-Key: $AGENT_KEY" \
  "$NYXID_API_ORIGIN/api/v1/proxy/s/$SERVICE_SLUG/user"
unset AGENT_KEY
```

Look for a GitHub profile containing a `login` field. For the account `octocat`, the result includes `"login":"octocat"`. Studio also offers **Verify key** to inspect the NyxID key itself.

## Self-host

Sign in to your instance's Studio and use its API origin for `NYXID_API_ORIGIN` in the verification command. If your administrator has configured a GitHub connector OAuth app, continue directly to GitHub. Otherwise, select **Use your own OAuth app** when offered, or follow the credential setup shown in **Connect Service**.

Copy the **NyxID callback URL** shown by Studio into the GitHub OAuth app's **Authorization callback URL** field. It uses your API origin and the path `/api/v1/providers/callback`. Enter that app's **Client ID** and **Client Secret** in Studio and continue to GitHub. [Register a GitHub OAuth app](https://docs.github.com/en/apps/oauth-apps/building-oauth-apps/creating-an-oauth-app)

If your workspace does not provide an OAuth app and you cannot register one, ask the administrator to configure it. If your organization permits personal access tokens, you can select the **GitHub API (Personal Access Token)** catalog entry instead, supply a token with the access you need, and repeat the key and verification steps using that connection's own slug.

## Troubleshooting

- **GitHub authorization is unavailable or cancelled.** Confirm the selected provider app and the exact callback URL above if GitHub reports a redirect mismatch. Reopen the connection dialog and authorize the intended account. Check that the entry appears in **External Services** before creating its Agent Key.
- **NyxID asks for approval.** Open **Approvals** and review the account, connection and `GET /user` operation. A waiting request can finish after approval. If the request has already returned `approval_required`, follow its request ID or approval instructions. A denial or expiry ends that attempt. [Review approvals](/docs/web/guides/approvals)
- **NyxID rejects the key.** For a 401, check that you supplied the full NyxID Agent Key. For a 403 about scope or service access, compare the owner, `proxy` scope and selected connection with the exact slug in the URL.
- **GitHub rejects the credential.** Reauthorize the OAuth connection or update its PAT, then repeat the profile read.

## Next

Use the verified connection from [NyxBot](/docs/nyxbot/getting-started/first-chat), [an MCP client](/docs/ai/getting-started/connect-your-agent) or [the CLI](/docs/cli/getting-started/first-connection).
