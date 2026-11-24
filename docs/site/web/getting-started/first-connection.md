---
title: Your first Studio connection
description: Connect GitHub from Studio, restrict an Agent Key to that connection and verify it with a read-only request.
---

You need a NyxID account and a GitHub account you can authorize. [Sign up or sign in](/docs/web/getting-started/sign-up).

## Connect a service

1. In NyxBot's account menu choose **Open Studio**. Open the [services dashboard](/keys?tab=services). In the Studio sidebar this entry is **AI Services**. The page is titled **Services & Credentials**.
2. Under **External Services**, choose **Connect Service**. Select **GitHub OAuth** from the catalog. Follow the authorization prompt and confirm the GitHub account.
3. Return to **External Services** and find the connected entry. Record its slug, for example `api-github`. A second connection can have a different slug.
4. For a personal example use your own connection. For team use, check the organization that owns the connection and your role there. Choose the same owner when creating the key. [Organization access](/docs/web/guides/organizations)

## Create a scoped Agent Key

1. Open **Agent Keys** on the same page. Choose **Create API Key**. Give it a recognizable name, such as `github-profile-check`.
2. Set the owner, select the `proxy` scope, leave **Allow all services** off and select only your GitHub connection. Create the key and save the full value privately when it appears. For an existing key, open its details and check **Service Scope**.
3. GitHub authorization belongs to the service connection. The new Agent Key authenticates your client to NyxID and grants access to the selected connection. Its preview in Studio only identifies it.

## Verify the connection

In Bash, set the slug to the value from **External Services** and paste the full Agent Key at the hidden prompt. For self-hosting, replace the hosted API URL with your instance's API origin.

```bash
SERVICE_SLUG='api-github'
read -rs -p 'NyxID Agent Key: ' AGENT_KEY; echo
curl --fail-with-body -sS -H "X-API-Key: $AGENT_KEY" \
  "https://nyx-api.chrono-ai.fun/api/v1/proxy/s/$SERVICE_SLUG/user"
unset AGENT_KEY
```

GitHub returns your profile, including a `login` such as `"octocat"`. A NyxID 403 means you should check the key's `proxy` scope, owner and allowed service. If GitHub rejects the credential, reauthorize the connection and retry. You can inspect the key separately in Studio under **Verify key**.

Use the same connection with [NyxBot](/docs/nyxbot/getting-started/first-chat) or [an MCP client](/docs/ai/getting-started/connect-your-agent).
