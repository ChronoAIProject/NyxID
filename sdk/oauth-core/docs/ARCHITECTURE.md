# OAuth Core Package

## Structure

```
oauth-core/
|-- src/index.ts
|-- src/services.ts
|-- test/services.test.ts
|-- package.json
|-- tsconfig.json
`-- README.md
```

## Responsibilities

- `src/index.ts`: PKCE generation, authorization URL creation, callback exchange, token persistence, and userinfo retrieval.
- `src/services.ts`: authenticated connect-link orchestration and raw-response service proxy requests.
- `test/services.test.ts`: injected-fetch contract tests for agent-facing operations.
- `package.json`: npm publish metadata and build scripts.
- `tsconfig.json`: declaration and JavaScript emission for distribution.

## Dependency Boundary

- No runtime dependency on React.
- Web platform APIs only (`crypto`, `URL`, `fetch`, `Storage`), with injectable storage/fetch for testing.
