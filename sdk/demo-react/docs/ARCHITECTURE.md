# OAuth React Demo

## Structure

```
demo-react/
|-- src/
|   |-- main.tsx
|   |-- App.tsx
|   |-- styles.css
|   `-- vite-env.d.ts
|-- .env.example
|-- index.html
|-- package.json
|-- tsconfig.json
|-- vite.config.ts
`-- README.md
```

## Responsibilities

- `main.tsx`: SDK client bootstrap and provider wiring.
- `App.tsx`: login/callback/userinfo/logout demo flow.
- `.env.example`: runtime config contract for NyxID endpoint and OAuth client.
- `README.md`: runbook for local demo execution.

## Dependency Boundary

- Depends only on `@nyxids/oauth-react` and React runtime.
- No coupling to the main NyxID frontend app code.
