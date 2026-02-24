# SDK Workspace

## Structure

```
sdk/
|-- package.json
|-- oauth-core/
|   |-- src/index.ts
|   |-- package.json
|   |-- tsconfig.json
|   `-- README.md
|-- oauth-react/
|   |-- src/index.tsx
|   |-- package.json
|   |-- tsconfig.json
|   `-- README.md
`-- demo-react/
    |-- src/main.tsx
    |-- src/App.tsx
    |-- package.json
    |-- .env.example
    `-- README.md
```

## Responsibilities

- `sdk/package.json`: workspace root, unified build and publish-precheck scripts.
- `oauth-core`: protocol and PKCE primitives, token and callback handling.
- `oauth-react`: React bindings layered on top of core primitives.
- `demo-react`: runnable reference client showing end-to-end OAuth redirect flow.

## Dependency Boundary

- `oauth-react` depends on `@nyxid/oauth-core` via workspace linkage.
- `demo-react` depends on `@nyxid/oauth-react` via workspace linkage.
- SDK workspace remains independent from app runtime code in `frontend/`.
