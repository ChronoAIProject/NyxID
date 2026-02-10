# Code Review: Service Proxy Overhaul

**Reviewer:** code-reviewer agent
**Date:** 2026-02-10
**Scope:** 20 files changed, 760 insertions, 200 deletions
**Verdict:** BLOCK - 1 CRITICAL issue must be fixed before merge

---

## CRITICAL Issues (Must Fix)

### CR-C1: Reconnection fails due to unique index conflict

**File:** `backend/src/services/connection_service.rs:84-98`

The `connect_user` function checks for an existing **active** connection, but `disconnect_user` soft-deletes (sets `is_active: false`, leaves record in DB). The unique compound index on `(user_id, service_id)` in `db.rs:202-208` does not distinguish active from inactive records. When a user disconnects and tries to reconnect, the `insert_one` at line 128 will fail with a MongoDB duplicate key error.

**Reproduction:** Connect to a service, disconnect, attempt to reconnect. MongoDB returns `E11000 duplicate key error`.

**Fix:** In `connect_user`, after checking for an active connection, also check for an inactive one and reactivate it instead of inserting:

```rust
// After the active connection check, add:
let inactive = db
    .collection::<UserServiceConnection>(CONNECTIONS)
    .find_one(doc! {
        "user_id": user_id,
        "service_id": service_id,
        "is_active": false,
    })
    .await?;

if let Some(_existing) = inactive {
    // Reactivate and update credentials
    let mut set_doc = doc! {
        "is_active": true,
        "updated_at": mongodb::bson::DateTime::from_chrono(now),
    };
    if let Some(enc) = &credential_encrypted {
        set_doc.insert("credential_encrypted", mongodb::bson::Binary {
            subtype: mongodb::bson::spec::BinarySubtype::Generic,
            bytes: enc.clone(),
        });
    }
    if let Some(ct) = &credential_type {
        set_doc.insert("credential_type", ct.as_str());
    }
    if let Some(label) = credential_label {
        set_doc.insert("credential_label", label);
    }
    db.collection::<UserServiceConnection>(CONNECTIONS)
        .update_one(
            doc! { "user_id": user_id, "service_id": service_id },
            doc! { "$set": set_doc },
        )
        .await?;

    return Ok(ConnectionResult {
        connection_id: _existing.id,
        service_name: service.name,
        connected_at: now,
    });
}
// ... then proceed with insert_one for truly new connections
```

---

## HIGH Issues (Should Fix)

### CR-H1: `credential_label` has no length validation

**File:** `backend/src/services/connection_service.rs:31-32, 121`

The `credential` field is validated for length (max 8192 bytes), but `credential_label` has no length limit. A malicious user could send an arbitrarily large label, consuming storage and potentially causing issues in the UI.

**Fix:** Add length validation in both `connect_user` and `update_credential`:

```rust
if let Some(label) = credential_label {
    if label.len() > 200 {
        return Err(AppError::ValidationError(
            "Credential label must not exceed 200 characters".to_string(),
        ));
    }
}
```

### CR-H2: CredentialDialog shows wrong text for "update" mode

**File:** `frontend/src/components/dashboard/credential-dialog.tsx:16-93`

The `CredentialDialog` component is used for both "connect" and "update" flows (see `connection-grid.tsx:57,210`), but the dialog always shows "Connect to {service.name}" as the title and "Connect" as the button text. When updating a credential, this is misleading.

**Fix:** Add a `mode` prop to `CredentialDialog`:

```tsx
interface CredentialDialogProps {
  readonly service: DownstreamService;
  readonly mode: "connect" | "update";
  readonly onSubmit: (credential: string, label?: string) => void;
  readonly onCancel: () => void;
  readonly isPending: boolean;
}

// Then in the component:
<DialogTitle>
  {mode === "connect" ? `Connect to ${service.name}` : `Update Credential for ${service.name}`}
</DialogTitle>
<Button type="submit" ...>
  {mode === "connect" ? "Connect" : "Update"}
</Button>
```

And pass `mode` from `connection-grid.tsx`:

```tsx
<CredentialDialog
  service={credentialDialog.service}
  mode={credentialDialog.mode}
  ...
/>
```

### CR-H3: Missing `proxy_request_denied` audit log

**File:** `backend/src/services/proxy_service.rs`

The architecture doc (Section 7.5) specifies an audit log for `proxy_request_denied` when a connection or credential is missing. The implementation returns errors at lines 53-56, 68-72, and 77-81 but does not emit audit log entries. This makes it harder to detect unauthorized access attempts.

**Fix:** Add audit logging in the error paths. Since `proxy_service` doesn't have access to the `Database` handle in a fire-and-forget way, consider adding audit calls in the proxy handler that invokes `resolve_proxy_target`, or pass audit context through.

---

## MEDIUM Issues (Recommended Fix)

### CR-M1: MCP proxy type field naming mismatch with backend

**Files:** `mcp-proxy/src/types.ts:43,47-50` vs `backend/src/handlers/mcp.rs:34,23-25`

The backend serializes fields as snake_case (`service_category`, `total_services`, `total_endpoints`) but the TypeScript types use camelCase (`serviceCategory`, `totalServices`, `totalEndpoints`). If the NyxID client does not perform field name conversion, these fields would be `undefined` at runtime.

Currently unused in MCP proxy logic, so no runtime impact, but:
1. Verify the `NyxIdClient` handles the conversion
2. If not, either update TypeScript types to match the backend JSON, or add a mapping layer

Backend JSON:
```json
{ "service_category": "connection", "total_services": 5, "total_endpoints": 20 }
```

TypeScript expects:
```typescript
{ serviceCategory: "connection", totalServices: 5, totalEndpoints: 20 }
```

### CR-M2: Frontend credential input has no max length feedback

**File:** `frontend/src/components/dashboard/credential-dialog.tsx:53-59`

The backend enforces a max credential length of 8192 bytes (`connection_service.rs:13`), but the credential `<Input>` element has no `maxLength` attribute or visual feedback about limits. Users who paste very long credentials would get an opaque server error.

**Fix:** Add `maxLength={8192}` to the credential Input element and a hint:

```tsx
<Input
  id="credential"
  type="password"
  placeholder={inputConfig.placeholder}
  value={credential}
  onChange={(e) => setCredential(e.target.value)}
  maxLength={8192}
  autoComplete="off"
/>
<p className="text-xs text-muted-foreground">Max 8192 characters</p>
```

### CR-M3: Error messages in connection-grid are generic

**File:** `frontend/src/components/dashboard/connection-grid.tsx:49-54, 85-91, 98-100`

All catch blocks in `connection-grid.tsx` show generic error messages like "Failed to connect to service" without surfacing the backend error message. Compare with `service-list.tsx:64-69` which checks for `ApiError` and shows the specific message.

**Fix:** Follow the pattern in `service-list.tsx`:

```tsx
} catch (error) {
  if (error instanceof ApiError) {
    toast.error(error.message);
  } else {
    toast.error("Failed to connect to service");
  }
}
```

### CR-M4: Tool search results unbounded

**File:** `mcp-proxy/src/tools.ts:175-201`

`handleToolSearch` returns all matching tools without a limit. With many services, a broad search query could return hundreds of results, overwhelming the MCP client.

**Fix:** Add a max results limit:

```typescript
const MAX_SEARCH_RESULTS = 25;
const matches = tools.filter(/* ... */).slice(0, MAX_SEARCH_RESULTS);

const suffix = tools.filter(/* ... */).length > MAX_SEARCH_RESULTS
  ? `\n\n(Showing first ${MAX_SEARCH_RESULTS} of ${total} matches. Refine your query for more specific results.)`
  : '';
```

### CR-M5: `update_credential` does not clear stale `credential_label`

**File:** `backend/src/services/connection_service.rs:184-186`

When updating a credential, if `credential_label` is `None`, the existing label is preserved due to the `if let Some(label)` pattern. This means a user cannot remove a label without explicitly passing an empty string. Consider whether the intended behavior is to clear the label when not provided, or require explicit null.

Document the expected behavior at minimum.

---

## LOW Issues (Nice to Have)

### CR-L1: `service_category` form default not set explicitly

**File:** `frontend/src/pages/service-list.tsx:48-56`

The `useForm` `defaultValues` doesn't include `service_category`. The select element uses `field.value ?? "connection"` for display, but the underlying form value is `undefined` until changed. This works because the backend defaults to "connection" when absent, but explicit defaults are clearer:

```tsx
defaultValues: {
  name: "",
  description: "",
  base_url: "",
  auth_type: "api_key",
  service_category: "connection", // Add explicit default
},
```

### CR-L2: Frontend mutation return type mismatch

**File:** `frontend/src/hooks/use-services.ts:161-164`

`useConnectService` declares its return type as `Promise<UserServiceConnection>`, but the backend's `ConnectResponse` only has `{ service_id, service_name, connected_at }`. The return value is never used (query invalidation handles refresh), so no runtime issue, but the type is inaccurate.

**Fix:** Define a `ConnectResponse` type or use `Promise<unknown>`.

### CR-L3: `CONNECTABLE_AUTH_TYPES` defined but unused

**File:** `frontend/src/schemas/services.ts:21-26`

`CONNECTABLE_AUTH_TYPES` is defined but never imported or used anywhere in the codebase.

**Fix:** Remove if not needed, or add a comment documenting its intended future use.

### CR-L4: Consider trimming credential whitespace

**File:** `frontend/src/components/dashboard/credential-dialog.tsx:34`

The credential validation `credential.length > 0` would pass for whitespace-only strings. While the backend would store the encrypted whitespace, it likely wouldn't be a valid credential for any service.

**Fix:** Trim before checking:

```tsx
const trimmed = credential.trim();
if (trimmed.length > 0) {
  onSubmit(trimmed, label.length > 0 ? label : undefined);
}
```

---

## Positive Observations

1. **Clean architecture separation** - The new `connection_service.rs` properly extracts business logic from handlers, enabling reuse by proxy and MCP handlers.

2. **Comprehensive input validation** - Backend validates service categories, credential lengths, slug formats, base URLs (SSRF protection), and auth type compatibility.

3. **Proper credential security** - Credentials encrypted with AES, cleared on disconnect (set to Null), never exposed in API responses, password input type in frontend, autoComplete disabled.

4. **Immutable patterns throughout** - Frontend code consistently uses readonly types, spread operators, and functional patterns with no mutations.

5. **Good audit trail** - All connection lifecycle events are logged asynchronously.

6. **Backward compatibility** - `serde(default)` on new model fields ensures existing MongoDB documents deserialize correctly without migration.

7. **Proper error propagation** - Rust code uses `?` operator throughout with no `unwrap()` on fallible operations.

8. **Accessibility** - Delete button has `sr-only` text, form labels associated with inputs via `htmlFor`.

9. **File sizes** - All files are within the 200-400 line guideline (largest is `services.rs` at 743 lines, pre-existing).

10. **Type safety** - Both Rust and TypeScript use strict typing with no `any` types in frontend code.

---

## Summary

| Priority | Count | Status |
|----------|-------|--------|
| CRITICAL | 1     | Must fix |
| HIGH     | 3     | Should fix |
| MEDIUM   | 5     | Recommended |
| LOW      | 4     | Nice to have |

**Verdict:** BLOCK until CR-C1 (reconnection failure) is resolved. HIGH issues should also be addressed before merge.
