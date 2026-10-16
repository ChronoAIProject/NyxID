# Website A source migration

The approved product HTML is an external design input with SHA-256
`5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33`.
The original review materials remain untouched and are not a production runtime
or a build dependency. Production builds compile the committed React source.

From `frontend`, using a supported Node runtime and `npm ci`:

```sh
node scripts/website-a/generate.mjs /absolute/path/to/nyxid-site-preview.html
```

The migration checks the exact baseline hash. Its isolated VM evaluates only
baseline template/data declarations at development time; it never runs in the
web app. The generated output uses native JSX, local brand assets, isolated
locale resources, pathname routes and explicitly implemented React interactions.
No HTML iframe, script, `eval` or raw HTML insertion ships to the website.

Files in `src/features/website-a/generated` are separated by locale and route for
lazy loading. The generator contains the explicit production adaptations:
auth/query mapping, disabled newsletter, Discord contact, documentation/legal
links, illustrative story disclosure and excluded internal review guides. Change
the generator when updating those adaptations, then regenerate and run the
checks in `docs/plans/website-a-delivery.md`. The runtime interactions, auth link
helpers, public route policy, legacy blog compatibility and workflow CSS are
maintained normally outside the generated directory.

The baseline's five-workflow embedded film is migrated to locale JSON and native
React, with accessible selection and playback controls. Its animation timing and
responsive arrangement are implemented in `interactions.tsx` / `workflow-film.css`.
All public CSS selectors and animation names are scoped to avoid changing the app.

The approved A + v4.1 enhancement is generated through native `HeroSection`,
`HeroTitle`, `VisualLink` and precise `RevealFrame` placements. Source adapters,
scoped styles and lifecycle tests live in `visuals/`; the full React Bits license
and pinned-source hashes are retained there. The static 24-card tool wall remains
unchanged. Public pages use an independent dark theme and never change saved app
preferences. See `docs/plans/website-v4.1-delivery.md` for current verification and
the unresolved UnicornStudio branding requirement.

Contact uses a native `ContactSection` adapter while retaining the original
Discord card and scoped layout selectors. Its form and scripted FAQ live in
`src/features/website-a/contact.tsx`; validation is in
`src/schemas/website-contact.ts`. Delivery integration is pending an owner-provided
recipient/endpoint. Never restore the prototype's fake success branch or replace
the whole interface when a backend is unavailable. Contact-specific shrink/error
and disclosure styles are in `contact.css`; the original layout/breakpoint
styles remain in `website-a.css`.
