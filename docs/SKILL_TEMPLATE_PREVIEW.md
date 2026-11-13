# Skill template frontend preview

Branch: `preview/skill-template-loadout`, based on `feat/nyx-skill-template`.
This runs the actual NyxID frontend with the production API and account data.

For immediate visual inspection without signing in, open
<http://127.0.0.1:4321/skill-preview.html>. This development entry renders the
real `SkillTemplateCard` component with historical example outlines: the
`phone-front-desk-sg` sample loadout and 19 reusable skills. Choose any member
to see its outline; the `skill` query parameter preserves a direct link to it.
These are sample excerpts, not backend-verified catalog results or skills
attached to your account. The page does not run skills or change account data.
The normal production build does not include this HTML entry.

From `frontend/`:

```sh
npm ci
npm run preview:skills
```

Open <http://127.0.0.1:4321/assistant>. Sign in using **Continue with the NyxID
app**, then approve the displayed device login in your production session.
Hosted social OAuth callbacks return to the hosted application; the device
login flow establishes the browser session on localhost through the proxy.

Open the agent's settings, then **Skills → Find skills**, and select a skill
and a published version. The real outline card is returned by production when
`assistant:skill-runs` is enabled for your user. An attached skill's **Choose
version** action opens the same view. Skill packages must contain valid NYX
template sections for an outline to appear.

The launcher defaults to `https://nyxid.ai` for both `BACKEND_URL` and
`FRONTEND_URL`. These environment variables and `NYXID_PREVIEW_PORT` can be
overridden. It uses the existing Vite API proxy and cookie rewriting. Existing
account permissions and backend feature flags apply, including to writes made
through the app.

The current feature adds outline cards in Find skills and NyxBot's outline
reading guidance. Chat Start cards, combined skillset execution and live run
checklists still require integration; this branch does not add them.
