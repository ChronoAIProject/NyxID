# Contributing to NyxID

Thank you for your interest in contributing to NyxID. This guide covers the development workflow, coding conventions, and quality expectations for the project.

---

## Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Development Workflow](#development-workflow)
- [Architecture Rules](#architecture-rules)
- [Backend Conventions](#backend-conventions)
- [Frontend Conventions](#frontend-conventions)
- [Security Requirements](#security-requirements)
- [Testing](#testing)
- [Commit Messages](#commit-messages)
- [Pull Requests](#pull-requests)
- [Documentation](#documentation)
- [Resources](#resources)

---

## Code of Conduct

Be respectful, constructive, and inclusive. Harassment, discrimination, and bad-faith behavior are not tolerated. We are all here to build something useful together.

---

## Getting Started

### Prerequisites

| Tool       | Version   | Purpose                              |
|------------|-----------|--------------------------------------|
| Rust       | 1.85+     | Backend and node-agent compiler      |
| Node.js    | 20+       | Frontend build tooling               |
| MongoDB    | 8.0       | Primary database                     |
| Docker     | 24+       | Run MongoDB and Mailpit via Compose  |

### Local Setup

```bash
# Clone the repository
git clone https://github.com/ChronoAIProject/NyxID.git
cd NyxID

# Copy environment template and generate an encryption key
cp .env.example .env
openssl rand -hex 32  # Paste output as ENCRYPTION_KEY in .env

# Start infrastructure (MongoDB on 27018, Mailpit on 8025)
docker compose up -d

# Start the backend (auto-generates RSA keys in dev mode)
cargo run

# In a separate terminal, start the frontend
cd frontend && npm install && npm run dev
```

The backend starts on `http://localhost:3001`, frontend on `http://localhost:3000`, and Mailpit web UI on `http://localhost:8025`.

### Verify

```bash
curl http://localhost:3001/health
# {"status":"ok","version":"0.1.0"}
```

---

## Development Workflow

1. **Fork and clone** the repository
2. **Create a feature branch** from `dev`: `git checkout -b feature/your-feature dev`
3. **Make your changes**, following the conventions below
4. **Run tests and lint** before committing (see [Testing](#testing))
5. **Commit** using [conventional commit messages](#commit-messages)
6. **Push** your branch and open a pull request against `dev`

### Branch Naming

| Prefix      | Use Case                        |
|-------------|---------------------------------|
| `feature/`  | New features                    |
| `fix/`      | Bug fixes                       |
| `refactor/` | Code restructuring              |
| `docs/`     | Documentation only              |
| `test/`     | Adding or fixing tests          |
| `chore/`    | Tooling, CI, dependencies       |

---

## Architecture Rules

NyxID follows a strict layered architecture. Understand these boundaries before contributing.

### Backend Layers

```
handlers/ --> services/ --> models/
```

- **models/** -- Plain structs with serde derive macros and a `COLLECTION_NAME` constant. No business logic.
- **services/** -- Business logic. Takes `&mongodb::Database` and `&str` for IDs. No HTTP types.
- **handlers/** -- HTTP layer only. Converts `AuthUser.user_id` (Uuid) to `String` before calling services. Uses **dedicated response structs** -- never serialize model structs directly to API responses.
- **crypto/** -- Cryptographic operations (JWT, AES, password hashing, token generation).
- **mw/** -- Axum middleware (auth extraction, rate limiting, security headers).
- **errors/** -- Centralized error types (`AppError` enum with `thiserror`).

### Adding a New Endpoint

1. Define request/response types in `handlers/<module>.rs`
2. Implement business logic in `services/<module>.rs`
3. Register the route in `routes.rs`
4. Add audit logging where appropriate

---

## Backend Conventions

### MongoDB Model Rules

These are critical -- incorrect serialization silently breaks data persistence:

- **NEVER** use `#[serde(skip_serializing)]` on model fields -- it prevents `insert_one(&struct)` from storing them
- **ALWAYS** use `#[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]` on `DateTime<Utc>` fields
- For `Option<DateTime<Utc>>`, use the custom `bson_datetime::optional` helper (in `models/bson_datetime.rs`)
- IDs are UUID v4 stored as strings in MongoDB `_id` fields
- Each model must have a `const COLLECTION_NAME: &str` constant

### Error Handling

Use the `AppError` enum and `AppResult<T>` type alias:

```rust
fn my_handler() -> AppResult<Json<MyResponse>> {
    // AppResult<T> = Result<T, AppError>
}
```

Error variants map to HTTP status codes and numeric error codes (1000-3002, 7000, 8000-8003). Internal and database errors must never leak details to clients.

### Rust Style

- Use `thiserror` for error types
- Prefer `&str` over `String` in service function parameters
- Keep handler functions thin -- delegate logic to services
- All key material must use `Zeroizing` wrappers
- All `Debug` impls on sensitive types must redact secrets

---

## Frontend Conventions

- **Validation**: Zod schemas in `schemas/` directory, one file per domain
- **Forms**: React Hook Form with `@hookform/resolvers/zod`
- **Server state**: TanStack Query hooks in `hooks/`, one file per domain
- **Client state**: Zustand store (`stores/auth-store.ts`)
- **UI components**: Radix UI + shadcn/ui pattern (`components/ui/`)
- **Routing**: TanStack Router with type-safe route params
- **No `console.log`** in production code

### File Organization

- Many small files over few large files (200-400 lines typical, 800 max)
- Organize by feature/domain, not by type
- Extract reusable components into `components/shared/`

---

## Security Requirements

All contributions must pass these checks before merge:

- [ ] No hardcoded secrets (API keys, passwords, tokens) -- use environment variables
- [ ] All user inputs validated (Zod on frontend, service-level validation on backend)
- [ ] No SQL/NoSQL injection vectors (use parameterized queries)
- [ ] No XSS vectors (sanitized HTML, proper escaping)
- [ ] Authentication and authorization verified on all protected endpoints
- [ ] Error messages do not leak internal details (database errors, stack traces)
- [ ] Sensitive data encrypted at rest (use the existing `crypto/aes.rs` envelope encryption)
- [ ] Rate limiting applied to new endpoints where appropriate

If your change touches authentication, encryption, or credential handling, it will receive extra scrutiny during review.

---

## Testing

### Running Tests

```bash
# Backend unit tests
cargo test

# Backend tests with all features (including KMS providers)
cargo test --all-features

# Node agent tests
cargo test -p nyxid-node

# Frontend tests
cd frontend && npm run test

# Frontend lint
cd frontend && npm run lint

# Frontend type check + build
cd frontend && npm run build
```

### Test Expectations

- All new backend logic should have corresponding unit tests
- All new frontend components should have tests where behavior is non-trivial
- Existing tests must not break -- if a test fails, fix the implementation, not the test (unless the test itself is wrong)
- Frontend linting must pass with zero errors

---

## Commit Messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>: <description>

<optional body>
```

| Type       | Use Case                                   |
|------------|--------------------------------------------|
| `feat`     | New feature                                |
| `fix`      | Bug fix                                    |
| `refactor` | Code restructuring (no behavior change)    |
| `docs`     | Documentation only                         |
| `test`     | Adding or fixing tests                     |
| `chore`    | Tooling, CI, dependencies                  |
| `perf`     | Performance improvement                    |

Keep the subject line under 72 characters. Use the body for context on *why* the change was made, not *what* was changed (the diff shows that).

---

## Pull Requests

### Before Opening a PR

- [ ] All tests pass locally (`cargo test`, `npm run test`, `npm run lint`)
- [ ] No new compiler warnings (`cargo build` is clean)
- [ ] Your branch is rebased on the latest `dev`
- [ ] Commit history is clean (squash fixup commits)

### PR Description

Include:
- **Summary**: What changed and why (1-3 bullet points)
- **Test plan**: How you verified the changes work

### CI Checks

Every pull request runs the following checks automatically via GitHub Actions:

| Job | What it does |
|-----|--------------|
| **Rust Format** | `cargo fmt --check` -- ensures consistent formatting |
| **Rust Clippy** | `cargo clippy --workspace` -- catches common mistakes and enforces idioms |
| **Rust Test** | `cargo test --workspace` with a real MongoDB service -- unit and integration tests |
| **Rust Features** | Builds with `--features aws-kms`, `gcp-kms`, and both -- ensures KMS providers compile |
| **Frontend** | `npm run lint`, `npm run test`, `npm run build` -- lint, test, and type-check |
| **SDK Build** | `npm run build` across all SDK packages -- ensures TypeScript compiles |

All checks must pass before a PR can be merged. If a check fails, fix the issue locally and push -- the workflow re-runs automatically.

### Review Process

- All PRs target the `dev` branch (not `main`)
- At least one approval is required before merge
- All CI checks must pass (see above)
- Security-sensitive changes require explicit security review

---

## Documentation

- Update `docs/API.md` if you add or change API endpoints
- Update `docs/ARCHITECTURE.md` if you add new collections, services, or architectural patterns
- Update `README.md` if you add new features visible to users
- Do **not** create new documentation files unless the feature is significant enough to warrant a standalone guide

### In-Code Documentation

- Add comments only where the logic is non-obvious
- Do not add boilerplate docstrings to every function
- Do not add `// removed` comments for deleted code -- just delete it

---

## Releases

Releases are triggered by pushing a semver tag to `main`:

```bash
git tag v0.2.0
git push origin v0.2.0
```

The release workflow:
1. Runs the full CI suite as a gate
2. Builds and pushes Docker images to GitHub Container Registry (`ghcr.io`)
3. Creates a GitHub Release with an auto-generated changelog

Only maintainers should create release tags. Contributors do not need to worry about releases -- just open PRs against `dev`.

---

## Resources

| Document | Description |
|----------|-------------|
| [README.md](README.md) | Project overview, quick start, features |
| [docs/API.md](docs/API.md) | Full API reference with request/response schemas |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | System architecture, data flows, database schema |
| [docs/DEVELOPER_GUIDE.md](docs/DEVELOPER_GUIDE.md) | In-depth development patterns and conventions |
| [docs/SECURITY.md](docs/SECURITY.md) | Security architecture and threat model |
| [docs/DEPLOYMENT.md](docs/DEPLOYMENT.md) | Production deployment guide |
| [docs/ENCRYPTION_ARCHITECTURE.md](docs/ENCRYPTION_ARCHITECTURE.md) | Envelope encryption and KMS integration |
| [docs/NODE_PROXY.md](docs/NODE_PROXY.md) | Credential node setup and usage |
| [docs/NYXID_NODE.md](docs/NYXID_NODE.md) | Node agent CLI reference |

---

## License

By contributing to NyxID, you agree that your contributions will be licensed under the [MIT License](LICENSE).
