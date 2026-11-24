---
title: Your first NyxBot chat
description: Ask NyxBot to read the username of your connected GitHub account and check its answer in the conversation.
---

You need a NyxID account with NyxBot enabled and a GitHub account you can authorize. [Sign up or sign in](/docs/web/getting-started/sign-up).

1. Open **NyxBot** and choose **New chat** next to your agent. You can use your personal assistant for this example.
2. Ask “Help me connect my GitHub account.” Open the connection link NyxBot offers. On the hosted form, check the service and owner, then choose **Approve connection**. Complete GitHub authorization using the account whose username you want to check and return to the form. If the link expires or you cancel, request a fresh link in the same chat. You can also use **Open Studio** in the account menu and [connect GitHub there](/docs/web/getting-started/first-connection#connect-a-service).
3. Return to the chat and send this message:

   > Use my connected GitHub service to read my authenticated GitHub profile. What is the login returned by GitHub?

4. Open the reply in the same conversation. The tool activity should show a GitHub read, and the answer should include the GitHub username you authorized. For an account named `octocat`, the expected `login` is `octocat`.

## When NyxBot needs your help

If an approval card appears, review the service, account and requested operation. Approve the profile read if it matches your request. Return to the conversation to see its result, or repeat the question if that turn ended.

If the assistant cannot see GitHub, open Studio and check the connection and its owner. A specialist is useful for a recurring job that needs its own instructions and a limited set of services. If you're chatting with one, check that its **Services** include the intended GitHub connection, then retry in that specialist's chat. [Approvals](/docs/web/guides/approvals) · [NyxBot CLI reference](/docs/cli/reference/nyxbot)
