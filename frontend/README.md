# NyxID frontend

## Credits page scope

The Credits revision is limited to `/admin/credits` and its allocation controls.
Keep the existing `Credit grants`, `Free allowances`, and `Schedules` tabs, as well
as the existing `/billing` page, wallet/top-up UI, and navigation. The combined
user credit-summary API is additive and does not replace the user Billing UI.

## Local preview readiness

Use the same backend revision as the frontend when reviewing the Credits work.
`npm run preview` now checks the configured backend before building and starting
Vite. It stops when either Credits summary route is missing, an API responds with
HTML, or configured social login cannot return to the local cookie host and URL.
The server binds only to loopback, uses a strict port, and builds the current source
so an old `dist` directory cannot silently become the preview.

```sh
BACKEND_URL=http://127.0.0.1:3001 FRONTEND_URL=http://127.0.0.1:3000 npm run preview
```

`BACKEND_URL` selects the API server. `FRONTEND_URL` must match the origin trusted
by that backend for CSRF; Vite rewrites proxied Origin/Referer to this value. The
preview defaults to `http://127.0.0.1:3000`; `--host localhost` and `--port 4614`
are supported. Use `npm run preview:check` to run only the read-only checks and
`npm run test:preview` for their regression suite.

A passing preflight establishes configuration compatibility, not authenticated
acceptance. Before sharing a preview, verify sign-in, authenticated admin and user
credit summaries, recipient/service lookup, one-time and recurring allocation,
policy persistence, and isolated reclaim through that exact frontend proxy. Run
financial mutation checks against an isolated local database. The remaining
production deployment and external billing integrations need their own acceptance.

Production social login cannot use localhost's state cookie: its provider callback
returns to the configured production API domain. Cookie/Origin rewriting alone
does not bridge that login. The new summary endpoints, service caps, and pending
reclaim also require the matching backend deployment; an old grants endpoint
accepting writes is insufficient. The Credits page disables changes while its
validated summary is unavailable, including after a failed cached refresh.

## React + TypeScript + Vite

This template provides a minimal setup to get React working in Vite with HMR and some ESLint rules.

Currently, two official plugins are available:

- [@vitejs/plugin-react](https://github.com/vitejs/vite-plugin-react/blob/main/packages/plugin-react) uses [Babel](https://babeljs.io/) (or [oxc](https://oxc.rs) when used in [rolldown-vite](https://vite.dev/guide/rolldown)) for Fast Refresh
- [@vitejs/plugin-react-swc](https://github.com/vitejs/vite-plugin-react/blob/main/packages/plugin-react-swc) uses [SWC](https://swc.rs/) for Fast Refresh

## React Compiler

The React Compiler is not enabled on this template because of its impact on dev & build performances. To add it, see [this documentation](https://react.dev/learn/react-compiler/installation).

## Expanding the ESLint configuration

If you are developing a production application, we recommend updating the configuration to enable type-aware lint rules:

```js
export default defineConfig([
  globalIgnores(['dist']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      // Other configs...

      // Remove tseslint.configs.recommended and replace with this
      tseslint.configs.recommendedTypeChecked,
      // Alternatively, use this for stricter rules
      tseslint.configs.strictTypeChecked,
      // Optionally, add this for stylistic rules
      tseslint.configs.stylisticTypeChecked,

      // Other configs...
    ],
    languageOptions: {
      parserOptions: {
        project: ['./tsconfig.node.json', './tsconfig.app.json'],
        tsconfigRootDir: import.meta.dirname,
      },
      // other options...
    },
  },
])
```

You can also install [eslint-plugin-react-x](https://github.com/Rel1cx/eslint-react/tree/main/packages/plugins/eslint-plugin-react-x) and [eslint-plugin-react-dom](https://github.com/Rel1cx/eslint-react/tree/main/packages/plugins/eslint-plugin-react-dom) for React-specific lint rules:

```js
// eslint.config.js
import reactX from 'eslint-plugin-react-x'
import reactDom from 'eslint-plugin-react-dom'

export default defineConfig([
  globalIgnores(['dist']),
  {
    files: ['**/*.{ts,tsx}'],
    extends: [
      // Other configs...
      // Enable lint rules for React
      reactX.configs['recommended-typescript'],
      // Enable lint rules for React DOM
      reactDom.configs.recommended,
    ],
    languageOptions: {
      parserOptions: {
        project: ['./tsconfig.node.json', './tsconfig.app.json'],
        tsconfigRootDir: import.meta.dirname,
      },
      // other options...
    },
  },
])
```
