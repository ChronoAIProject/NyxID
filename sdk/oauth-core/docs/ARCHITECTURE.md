# OAuth Core Package

## Structure

```
oauth-core/
|-- src/index.ts
|-- package.json
|-- tsconfig.json
`-- README.md
```

## Responsibilities

- `src/index.ts`: PKCE generation, authorization URL creation, callback exchange, token persistence, and userinfo retrieval.
- `package.json`: npm publish metadata and build scripts.
- `tsconfig.json`: declaration and JavaScript emission for distribution.

## Dependency Boundary

- No runtime dependency on React.
- Browser APIs only (`crypto`, `URL`, `fetch`, `Storage`), with injectable storage/fetch for testing.
