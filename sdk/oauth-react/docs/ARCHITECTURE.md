# OAuth React Package

## Structure

```
oauth-react/
|-- src/index.tsx
|-- package.json
|-- tsconfig.json
`-- README.md
```

## Responsibilities

- `src/index.tsx`: React context/provider/hook wrapper around the core OAuth client.
- `package.json`: publish metadata, peer dependency boundaries, and build scripts.
- `tsconfig.json`: TS emit configuration for npm distribution.

## Dependency Boundary

- Depends on `@nyxid/oauth-core` for protocol logic.
- Exposes React-only API surface (`NyxIDProvider`, `useNyxID`, `createNyxClient`).
