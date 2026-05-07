use std::io::{IsTerminal, Write};

use anyhow::{Context, Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OutputFormat, ServiceCommands};
use crate::commands::lark_permission::print_permission_block;
use crate::org_resolver::resolve_org_id;

/// Parse one or more `--default-header NAME=VALUE[:overridable]` flag values
/// into a JSON-shaped list that the backend `validate_headers` helper will
/// then normalize and persist. NyxID#356.
///
/// - The separator is the *first* `=` in the raw argument, so values may
///   themselves contain `=` characters.
/// - The optional `:overridable` suffix must be the literal tail of the
///   trimmed value. Anything else after the final `:` is treated as part
///   of the value (e.g. URLs with ports still round-trip).
pub(crate) fn parse_default_headers(raw: &[String]) -> Result<Vec<serde_json::Value>> {
    let mut out = Vec::with_capacity(raw.len());
    for entry in raw {
        let (name, rest) = entry.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("--default-header must be in NAME=VALUE form (got: {entry})")
        })?;

        let name = name.trim().to_string();
        if name.is_empty() {
            bail!("--default-header has empty name: {entry}");
        }

        // Detect `:overridable` / `:sensitive` suffix flags. Keep the
        // check conservative — require the suffix to be exactly one of
        // the known tokens so values containing colons (URLs, scoped
        // tokens, etc.) are not clipped. Multiple flags can be chained
        // via repeated `:` suffixes.
        let mut value = rest.to_string();
        let mut overridable = false;
        let mut sensitive = false;
        loop {
            if let Some(stripped) = value.strip_suffix(":overridable") {
                overridable = true;
                value = stripped.to_string();
            } else if let Some(stripped) = value.strip_suffix(":sensitive") {
                sensitive = true;
                value = stripped.to_string();
            } else {
                break;
            }
        }

        out.push(serde_json::json!({
            "name": name,
            "value": value,
            "overridable": overridable,
            "sensitive": sensitive,
        }));
    }
    Ok(out)
}

fn validate_service_slug(slug: &str) -> Result<()> {
    if slug.is_empty() || slug.len() > 80 {
        bail!("Service slug must be between 1 and 80 characters");
    }

    let valid = slug
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !slug.starts_with('-')
        && !slug.ends_with('-')
        && !slug.contains("--");
    if !valid {
        bail!(
            "Service slug must contain only lowercase letters, digits, and single hyphens (no leading, trailing, or consecutive hyphens)"
        );
    }

    Ok(())
}

// Backend `UpdateUserServiceRequest::node_id` semantics:
//   ""        -> clear node_id (switch to direct routing)
//   Some(id)  -> set node_id
//   null/None -> leave unchanged
// Sending `null` for --direct silently leaves stale node routing in place.
fn build_route_body(direct: bool, node: Option<&str>) -> Result<Value> {
    let node_value = if direct {
        Value::String(String::new())
    } else if let Some(node_id) = node {
        Value::String(node_id.to_string())
    } else {
        bail!("Specify --node <NODE_ID> or --direct");
    };
    Ok(serde_json::json!({ "node_id": node_value }))
}

struct AddSshBody<'a> {
    label: &'a str,
    host: &'a str,
    port: u16,
    cert_auth: bool,
    ssh_auth_mode: &'a str,
    principals: &'a str,
    ttl: u32,
    via_node: &'a str,
    target_org_id: Option<&'a str>,
}

fn build_add_ssh_body(input: AddSshBody<'_>) -> serde_json::Map<String, Value> {
    let mut body = serde_json::Map::new();
    body.insert("label".into(), Value::String(input.label.to_string()));
    body.insert("ssh_host".into(), Value::String(input.host.to_string()));
    body.insert("ssh_port".into(), serde_json::json!(input.port));
    body.insert("ssh_certificate_auth".into(), Value::Bool(input.cert_auth));
    body.insert(
        "ssh_auth_mode".into(),
        Value::String(input.ssh_auth_mode.to_string()),
    );
    body.insert(
        "ssh_principals".into(),
        Value::String(input.principals.to_string()),
    );
    body.insert(
        "ssh_certificate_ttl_minutes".into(),
        serde_json::json!(input.ttl),
    );
    body.insert("node_id".into(), Value::String(input.via_node.to_string()));
    if let Some(org_id) = input.target_org_id {
        body.insert("target_org_id".into(), Value::String(org_id.to_string()));
    }
    body
}

async fn resolve_user_service_for_ssh_mode(
    api: &mut ApiClient,
    id_or_slug: &str,
) -> Result<(String, String, Option<String>, Option<String>, Option<u16>)> {
    let direct = api.get_value(&format!("/keys/{id_or_slug}")).await.ok();
    let service = match direct {
        Some(value) => value,
        None => {
            let resp: Value = api.get("/keys").await?;
            resp.get("keys")
                .and_then(|v| v.as_array())
                .and_then(|items| {
                    items.iter().find(|item| {
                        item.get("id").and_then(|v| v.as_str()) == Some(id_or_slug)
                            || item.get("_id").and_then(|v| v.as_str()) == Some(id_or_slug)
                            || item.get("slug").and_then(|v| v.as_str()) == Some(id_or_slug)
                            || item.get("service_slug").and_then(|v| v.as_str()) == Some(id_or_slug)
                    })
                })
                .cloned()
                .with_context(|| format!("Could not find SSH service '{id_or_slug}'"))?
        }
    };

    let id = service
        .get("id")
        .or_else(|| service.get("_id"))
        .and_then(|v| v.as_str())
        .with_context(|| format!("Service '{id_or_slug}' response did not include an id"))?
        .to_string();
    let slug = service
        .get("slug")
        .or_else(|| service.get("service_slug"))
        .and_then(|v| v.as_str())
        .unwrap_or(id_or_slug)
        .to_string();
    let principal_hint = service
        .get("ssh_allowed_principals")
        .and_then(|v| v.as_array())
        .and_then(|items| items.first())
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let host = service
        .get("ssh_host")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let port = service
        .get("ssh_port")
        .and_then(|v| v.as_u64())
        .and_then(|v| u16::try_from(v).ok());

    Ok((id, slug, principal_hint, host, port))
}

fn home_assistant_ws_frame_rules() -> Value {
    serde_json::json!([{
        "trigger": {
            "json_field_equals": {
                "path": "$.type",
                "value": "auth_required"
            }
        },
        "template": "{\"type\":\"auth\",\"access_token\":\"${credential}\"}",
        "frame_kind": "text",
        "consume_trigger": true,
        "direction": "downstream"
    }])
}

fn build_ws_frame_injections_body(preset: Option<&str>, clear: bool) -> Result<Option<Value>> {
    if clear {
        return Ok(Some(Value::Array(Vec::new())));
    }

    let Some(name) = preset else {
        return Ok(None);
    };

    match name {
        "home-assistant" => Ok(Some(home_assistant_ws_frame_rules())),
        other => bail!("Unsupported --ws-frame-preset '{other}'. Supported value: home-assistant"),
    }
}

pub async fn run(command: ServiceCommands) -> Result<()> {
    match command {
        ServiceCommands::Add {
            slug,
            custom,
            custom_slug,
            oauth,
            device_code,
            via_node,
            endpoint_url,
            label,
            auth_method,
            auth_key_name,
            credential,
            credential_env,
            scopes,
            org,
            openapi_spec_url,
            ws_frame_preset,
            ws_frame_clear,
            terminal,
            no_wait,
            auth,
        } => {
            // Wizard dispatch (docs/CLI_WIZARD_V2.md §3.1): route to
            // the browser flow when the invocation isn't "scripted-
            // complete". Flags compatible with prefill (slug, label,
            // via-node, endpoint-url, `--org`, plus `--custom`
            // definitional fields per issue #414) just seed the form.
            // Flags that declare a specific scripted flow (--credential,
            // --credential-env, --oauth, --device-code, --output json)
            // fall through to the existing non-interactive path so
            // scripted behavior for existing users is unchanged.
            //
            // Issue #414 — `--custom` and its companion flags
            // (`--auth-method`, `--auth-key-name`, `--slug` in the
            // override sense) are *not* scripted markers; they're
            // definitional values for the wizard's custom-service
            // form. The exception: when `--auth-method` /
            // `--auth-key-name` / `--slug` are passed WITHOUT
            // `--custom`, they're acting as *overrides* on a catalog
            // entry — that's the existing scripted-override use case
            // and stays scripted.
            //
            // `--org` used to be treated as an advanced scripted-only
            // flag. It now behaves like the api-key wizard: the CLI
            // resolves the slug/name to a canonical org owner id before
            // dispatch and the browser pre-selects that owner.
            //
            // Headless contexts (SSH sessions, no local display on
            // Linux, AI-agent bash tool) NO LONGER fall through to the
            // stdin-prompt path by default — `run_ai_key_wizard` picks
            // the remote-pairing transport for them (prints a code +
            // URL the user opens on another device). Set
            // `NYXID_NO_WIZARD=1` to restore the pre-wizard stdin
            // prompt path for CI jobs or scripts that rely on it.
            let mut api = if org.is_some() {
                Some(ApiClient::from_auth(&auth)?)
            } else {
                None
            };
            let org = match org {
                Some(raw) => Some(
                    resolve_org_id(api.as_mut().expect("api initialized for org"), &raw)
                        .await
                        .with_context(|| format!("Could not resolve org '{raw}'"))?,
                ),
                None => None,
            };
            let interactive_output = matches!(auth.output, OutputFormat::Table);
            let explicit_scripted = is_explicit_scripted(
                credential.is_some(),
                credential_env.is_some(),
                oauth,
                device_code,
                custom,
                custom_slug.is_some(),
                auth_method.is_some(),
                auth_key_name.is_some(),
                !scopes.is_empty(),
                openapi_spec_url.is_some(),
                ws_frame_preset.is_some(),
                ws_frame_clear,
            );
            // `--no-wait` forces the pairing variant even when a local
            // browser is available and wins over `--output json`
            // (agent wrappers use JSON specifically to automate the
            // pairing handoff). Only `--terminal` and explicit
            // scripted flags override it.
            if !explicit_scripted
                && !terminal
                && (no_wait || (interactive_output && crate::wizard::is_browser_flow_eligible()))
            {
                let prefill = crate::wizard::WizardPrefill {
                    slug: slug.clone(),
                    label: label.clone(),
                    via_node: via_node.clone(),
                    org: org.clone(),
                    endpoint_url: endpoint_url.clone(),
                    // Issue #414 — definitional fields for custom mode.
                    // When `--custom` is set, the SPA skips the catalog
                    // grid and renders a custom-service form pre-
                    // populated with auth_method / auth_key_name /
                    // custom_slug (all optional, with sensible defaults
                    // applied SPA-side).
                    custom,
                    custom_slug: custom_slug.clone(),
                    auth_method: auth_method.clone(),
                    auth_key_name: auth_key_name.clone(),
                };
                return crate::wizard::run_ai_key_wizard(&auth, prefill, no_wait).await;
            }

            let mut api = match api {
                Some(api) => api,
                None => ApiClient::from_auth(&auth)?,
            };

            // Resolve `--via-node <ID_OR_NAME>` to a node ID up-front so that
            // node names shown by `nyxid node list` and in the docs (e.g.
            // `--via-node my-laptop`) work the same as node UUIDs.
            let via_node = match via_node {
                Some(raw) => Some(
                    crate::commands::node::resolve_node_id(&mut api, &raw)
                        .await
                        .with_context(|| format!("Could not resolve node '{raw}'"))?,
                ),
                None => None,
            };
            // Normalize --scope inputs: split each entry on comma/whitespace so
            // users can write `--scope a,b --scope "c d"` or `--scope a --scope b`.
            let additional_scopes: Vec<String> = scopes
                .iter()
                .flat_map(|raw| {
                    raw.split(|c: char| c == ',' || c.is_whitespace())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .collect();

            // `--scope` is forwarded only on the OAuth and device-code flows.
            // Other paths accept the flag for symmetry (e.g. a user adding a
            // custom endpoint that they will later connect via OAuth), but
            // we warn that the scopes are not recorded anywhere yet.
            let scope_flow_unsupported = !additional_scopes.is_empty() && !oauth && !device_code;
            if scope_flow_unsupported {
                if custom {
                    eprintln!(
                        "warning: --scope has no effect on --custom endpoints \
                         (custom endpoints use direct credentials, not OAuth). \
                         Use --oauth or --device-code with a catalog slug to request additional scopes."
                    );
                } else {
                    bail!(
                        "--scope is only supported with --oauth or --device-code \
                         (use one of those flags, or --custom to create a direct-credential endpoint)"
                    );
                }
            }

            if let Some(ref custom_slug) = custom_slug {
                validate_service_slug(custom_slug)?;
            }

            let ws_frame_injections =
                build_ws_frame_injections_body(ws_frame_preset.as_deref(), ws_frame_clear)?;

            // OAuth flow
            if oauth {
                return run_oauth_add(
                    &mut api,
                    slug,
                    CatalogAddFlowOptions {
                        custom_slug: custom_slug.as_deref(),
                        via_node: via_node.as_deref(),
                        additional_scopes: &additional_scopes,
                        target_org_id: org.as_deref(),
                        openapi_spec_url: openapi_spec_url.as_deref(),
                        ws_frame_injections: ws_frame_injections.as_ref(),
                    },
                    &auth,
                )
                .await;
            }

            // Device code flow
            if device_code {
                return run_device_code_add(
                    &mut api,
                    slug,
                    CatalogAddFlowOptions {
                        custom_slug: custom_slug.as_deref(),
                        via_node: via_node.as_deref(),
                        additional_scopes: &additional_scopes,
                        target_org_id: org.as_deref(),
                        openapi_spec_url: openapi_spec_url.as_deref(),
                        ws_frame_injections: ws_frame_injections.as_ref(),
                    },
                    &auth,
                )
                .await;
            }

            let mut body = serde_json::Map::new();
            // Effective auth method + key name used to generate a
            // context-aware credential prompt (e.g. "Enter app_secret:" for
            // Lark bot body auth instead of a generic "API key" label).
            // Assigned in both the `custom` and catalog branches below.
            let effective_auth_method: String;
            let effective_auth_key_name: String;
            // Token exchange credential field specs from the catalog entry
            // (only populated in the catalog branch for `token_exchange`
            // services). Drives the dynamic multi-field credential prompt.
            let mut catalog_token_exchange_fields: Option<Vec<TokenExchangeField>> = None;

            if custom {
                let label_val = match label {
                    Some(l) => l,
                    None => prompt_line("Label: ", "label")?,
                };
                let endpoint = match endpoint_url {
                    Some(u) => u,
                    None => prompt_line("Endpoint URL: ", "endpoint-url")?,
                };
                let method = match auth_method {
                    Some(m) => m,
                    None => prompt_line_default(
                        "Auth method [bearer/header/query/path/basic/body/bot_bearer/none]: ",
                        "bearer",
                        "auth-method",
                    )?,
                };
                // Default key name depends on method; `body` wants `app_secret`,
                // header wants `X-API-Key`, etc. `bot_bearer` is a fixed
                // `Authorization: Bot <token>` format so we don't prompt.
                let key_name = if requires_auth_key_name_prompt(&method, auth_key_name.is_some()) {
                    match auth_key_name {
                        Some(k) => k,
                        None => {
                            let default_key = default_auth_key_name(&method);
                            let label = if method == "body" {
                                "Body field name: "
                            } else {
                                "Auth key name: "
                            };
                            prompt_line_default(
                                &format!("{label}[{default_key}] "),
                                default_key,
                                "auth-key-name",
                            )?
                        }
                    }
                } else {
                    auth_key_name.unwrap_or_else(|| "Authorization".to_string())
                };

                body.insert("label".into(), Value::String(label_val));
                if let Some(ref custom_slug) = custom_slug {
                    body.insert("slug".into(), Value::String(custom_slug.clone()));
                }
                body.insert("endpoint_url".into(), Value::String(endpoint));
                body.insert("auth_method".into(), Value::String(method.clone()));
                body.insert("auth_key_name".into(), Value::String(key_name.clone()));

                effective_auth_method = method;
                effective_auth_key_name = key_name;
            } else {
                let slug = slug.ok_or_else(|| {
                    anyhow::anyhow!("Provide a catalog slug or use --custom for a custom endpoint")
                })?;
                body.insert("service_slug".into(), Value::String(slug.clone()));
                if let Some(ref custom_slug) = custom_slug {
                    body.insert("slug".into(), Value::String(custom_slug.clone()));
                }

                // Fetch catalog entry to validate slug and pull defaults
                // (name, auth method, auth key name, token_exchange credential
                // fields) so later prompts can adapt to what the service
                // actually expects.
                let catalog_entry = api.get_value(&format!("/catalog/{slug}")).await;
                let (catalog_name, catalog_auth_method, catalog_auth_key_name) =
                    match &catalog_entry {
                        Ok(entry) => {
                            catalog_token_exchange_fields = parse_token_exchange_fields(
                                &entry["token_exchange_credential_fields"],
                            );
                            (
                                entry["name"].as_str().map(|s| s.to_string()),
                                entry["auth_method"].as_str().map(|s| s.to_string()),
                                entry["auth_key_name"].as_str().map(|s| s.to_string()),
                            )
                        }
                        Err(_) => (None, None, None),
                    };

                if catalog_name.is_none() {
                    eprintln!("Service '{slug}' not found in catalog.");
                    eprintln!();
                    eprintln!("Available options:");
                    eprintln!("  nyxid catalog list              # browse available services");
                    eprintln!("  nyxid service add --custom      # add a custom endpoint");
                    eprintln!("  nyxid service add-ssh            # add an SSH service");
                    bail!(
                        "Catalog service '{slug}' not found. Use --custom for a custom endpoint."
                    );
                }

                let lbl = label.unwrap_or_else(|| catalog_name.unwrap_or_else(|| slug.clone()));
                body.insert("label".into(), Value::String(lbl));

                if let Some(url) = endpoint_url {
                    body.insert("endpoint_url".into(), Value::String(url));
                }
                if let Some(ref method) = auth_method {
                    body.insert("auth_method".into(), Value::String(method.clone()));
                }
                if let Some(ref key_name) = auth_key_name {
                    body.insert("auth_key_name".into(), Value::String(key_name.clone()));
                }

                // Effective values: flag override wins, otherwise inherit
                // from the catalog entry we just fetched.
                effective_auth_method = auth_method
                    .clone()
                    .or(catalog_auth_method)
                    .unwrap_or_default();
                effective_auth_key_name = auth_key_name
                    .clone()
                    .or(catalog_auth_key_name)
                    .unwrap_or_default();
            }

            if let Some(ref node) = via_node {
                body.insert("node_id".into(), Value::String(node.clone()));
            }

            // Resolve credential: --credential flag > --credential-env > interactive prompt
            // Skip if routing through a node (credentials live on the node)
            if requires_credential_prompt(&effective_auth_method, body.contains_key("node_id")) {
                let token_exchange_fields = if !custom && effective_auth_method == "token_exchange"
                {
                    catalog_token_exchange_fields.clone()
                } else {
                    None
                };
                let cred_value = if let Some(c) = credential {
                    c
                } else if let Some(env_var) = &credential_env {
                    std::env::var(env_var)
                        .with_context(|| format!("Environment variable {env_var} not set"))?
                } else if let Some(fields) = token_exchange_fields.as_ref() {
                    // Declarative token_exchange services advertise their
                    // credential fields via the catalog. Prompt for each
                    // field in order and compose a JSON object so the
                    // proxy can parse it back out at request time.
                    prompt_token_exchange_credential(fields)?
                } else {
                    let prompt =
                        credential_prompt_label(&effective_auth_method, &effective_auth_key_name);
                    prompt_password(&prompt, "credential")?
                };
                if cred_value.is_empty() {
                    bail!(
                        "Credential is required. Pass --credential, --credential-env, or enter interactively."
                    );
                }
                body.insert("credential".into(), Value::String(cred_value));
            }

            // Org-scoped creation: the caller must be an admin of the target
            // org. The backend enforces this via resolve_owner_access.
            if let Some(ref org_id) = org {
                body.insert("target_org_id".into(), Value::String(org_id.clone()));
            }

            // Forward three-state openapi_spec_url verbatim. Backend treats
            // absent as "inherit catalog default", empty string as "opt out",
            // and a non-empty URL as an explicit override.
            if let Some(ref url) = openapi_spec_url {
                body.insert("openapi_spec_url".into(), Value::String(url.clone()));
            }
            if let Some(ref rules) = ws_frame_injections {
                body.insert("ws_frame_injections".into(), rules.clone());
            }

            let result: Value = api.post("/keys", &body).await?;
            print_add_result(&api, &result, auth.output)?;
            if let Some(node_id) = via_node.as_deref() {
                let result_slug = result["slug"]
                    .as_str()
                    .or(result["service_slug"].as_str())
                    .unwrap_or("-");
                eprintln!();
                eprintln!("Next step: configure the credential on node {node_id}.");
                if custom {
                    eprintln!(
                        "  Run `nyxid node credentials add ... --service {result_slug}` on that node."
                    );
                } else {
                    eprintln!(
                        "  Run `nyxid node credentials setup --service {result_slug}` on that node."
                    );
                }
            }
            Ok(())
        }

        ServiceCommands::AddSsh {
            label,
            host,
            port,
            cert_auth,
            node_key,
            principals,
            ttl,
            via_node,
            org,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;

            // Accept either a node ID or a node name (from `nyxid node list`).
            let via_node = crate::commands::node::resolve_node_id(&mut api, &via_node)
                .await
                .with_context(|| format!("Could not resolve node '{via_node}'"))?;

            let org = match org {
                Some(raw) => Some(
                    resolve_org_id(&mut api, &raw)
                        .await
                        .with_context(|| format!("Could not resolve org '{raw}'"))?,
                ),
                None => None,
            };

            let principals_str = principals.as_deref().unwrap_or("");
            let principal_list: Vec<&str> = principals_str
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();

            let body = build_add_ssh_body(AddSshBody {
                label: &label,
                host: &host,
                port,
                cert_auth,
                ssh_auth_mode: if node_key {
                    "node_key"
                } else if cert_auth {
                    "cert"
                } else {
                    "proxy_only"
                },
                principals: principals_str,
                ttl,
                via_node: &via_node,
                target_org_id: org.as_deref(),
            });

            let result: Value = api.post("/keys", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let slug = result["slug"]
                        .as_str()
                        .or(result["service_slug"].as_str())
                        .unwrap_or("-");
                    let svc_id = result["id"]
                        .as_str()
                        .or(result["_id"].as_str())
                        .unwrap_or("-");

                    eprintln!("SSH service added!");
                    eprintln!();
                    eprintln!("Slug:       {slug}");
                    eprintln!("ID:         {svc_id}");
                    eprintln!("Host:       {host}:{port}");
                    eprintln!("Node:       {via_node}");
                    if let Some(ref org_id) = org {
                        eprintln!("Org:        {org_id}");
                    }
                    if node_key {
                        eprintln!();
                        eprintln!("SSH node-key setup:");
                        eprintln!(
                            "  nyxid node ssh-credentials add --service {slug} --principal {} \\",
                            principal_list.first().unwrap_or(&"ubuntu")
                        );
                        eprintln!("    --key-file ~/.ssh/id_ed25519 --host {host} --port {port}");
                    } else if cert_auth {
                        eprintln!();
                        eprintln!("SSH Setup Instructions:");
                        eprintln!("  1. Download the CA public key:");
                        eprintln!("     nyxid ssh issue-cert --service-id {svc_id} \\");
                        eprintln!("       --public-key-file ~/.ssh/id_ed25519.pub \\");
                        eprintln!(
                            "       --principal {} \\",
                            principal_list.first().unwrap_or(&"ubuntu")
                        );
                        eprintln!("       --certificate-file ~/.ssh/id_ed25519-cert.pub \\");
                        eprintln!("       --ca-public-key-file /tmp/nyxid_ca.pub");
                        eprintln!();
                        eprintln!("  2. On the target server, add to /etc/ssh/sshd_config:");
                        eprintln!("     TrustedUserCAKeys /etc/ssh/nyxid_ca.pub");
                        eprintln!();
                        eprintln!("  3. Copy the CA public key to the server:");
                        eprintln!("     scp /tmp/nyxid_ca.pub {host}:/etc/ssh/nyxid_ca.pub");
                        eprintln!();
                        eprintln!("  4. Restart sshd on the target server");
                    }
                }
            }
            Ok(())
        }

        ServiceCommands::ConvertSsh {
            slug,
            to_node_key,
            to_cert,
            to_proxy_only,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let mode = match (to_node_key, to_cert, to_proxy_only) {
                (true, false, false) => "node_key",
                (false, true, false) => "cert",
                (false, false, true) => "proxy_only",
                _ => bail!("Specify exactly one of --to-node-key, --to-cert, or --to-proxy-only"),
            };
            let (service_id, service_slug, principal_hint, host, port) =
                resolve_user_service_for_ssh_mode(&mut api, &slug).await?;
            let body = serde_json::json!({ "mode": mode });
            let result: Value = api
                .patch(&format!("/user-services/{service_id}/ssh-auth-mode"), &body)
                .await?;

            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
                OutputFormat::Table => {
                    eprintln!("SSH auth mode updated.");
                    eprintln!("Service: {service_slug}");
                    eprintln!("Mode:    {mode}");
                    match mode {
                        "node_key" => {
                            eprintln!();
                            eprintln!("Next step on the node:");
                            eprintln!(
                                "  nyxid node ssh-credentials add --service {service_slug} --principal {} \\",
                                principal_hint.as_deref().unwrap_or("<principal>")
                            );
                            eprintln!(
                                "    --key-file ~/.ssh/id_ed25519 --host {} --port {}",
                                host.as_deref().unwrap_or("<host>"),
                                port.unwrap_or(22)
                            );
                        }
                        "cert" | "proxy_only" => {
                            eprintln!();
                            eprintln!("Node-local SSH keys for this service are now stale.");
                            eprintln!("Prune them on the node with:");
                            eprintln!("  nyxid node ssh-credentials prune --stale");
                        }
                        _ => {}
                    }
                }
            }
            Ok(())
        }

        ServiceCommands::List { auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let resp: Value = api.get("/keys").await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&resp)?);
                }
                OutputFormat::Table => {
                    let items = resp.get("keys").and_then(|v| v.as_array());
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No services configured.");
                            eprintln!(
                                "Use `nyxid service add <slug>` or `nyxid catalog list` to get started."
                            );
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header(["ID", "Slug", "Label", "Endpoint", "Status", "Node"]);

                        for svc in items {
                            let id = svc["id"].as_str().or(svc["_id"].as_str()).unwrap_or("-");
                            let slug = svc["slug"]
                                .as_str()
                                .or(svc["service_slug"].as_str())
                                .unwrap_or("-");
                            let label = svc["label"].as_str().unwrap_or("-");
                            let endpoint = svc["endpoint_url"].as_str().unwrap_or("-");
                            let status = svc["status"].as_str().unwrap_or("active");
                            let node = svc["node_id"].as_str().unwrap_or("--");
                            table.add_row([id, slug, label, endpoint, status, node]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        ServiceCommands::Show { id, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let svc: Value = api.get(&format!("/keys/{id}")).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&svc)?);
                }
                OutputFormat::Table => {
                    let name = svc["label"]
                        .as_str()
                        .or(svc["name"].as_str())
                        .unwrap_or("-");
                    let slug = svc["slug"]
                        .as_str()
                        .or(svc["service_slug"].as_str())
                        .unwrap_or("-");
                    let svc_id = svc["id"].as_str().or(svc["_id"].as_str()).unwrap_or(&id);
                    let status = svc["status"].as_str().unwrap_or("active");
                    let endpoint = svc["endpoint_url"].as_str().unwrap_or("-");
                    let auth_method = svc["auth_method"].as_str().unwrap_or("-");
                    let auth_key = svc["auth_key_name"].as_str().unwrap_or("-");
                    let node = svc["node_id"].as_str().unwrap_or("-- (direct)");
                    let svc_type = svc["service_type"].as_str().unwrap_or("http");
                    let is_active = svc["is_active"].as_bool().unwrap_or(true);

                    eprintln!("Service: {name} ({slug})");
                    eprintln!("ID:         {svc_id}");
                    eprintln!("Type:       {svc_type}");
                    eprintln!("Status:     {status}");
                    eprintln!("Active:     {is_active}");
                    eprintln!();
                    eprintln!("Endpoint:   {endpoint}");
                    eprintln!("Auth:       {auth_method} / {auth_key}");
                    eprintln!("Node:       {node}");
                    if let Some(spec) = svc["openapi_spec_url"].as_str() {
                        eprintln!("OpenAPI:    {spec}");
                    }

                    // SSH-specific fields
                    if svc_type == "ssh" {
                        let ssh_host = svc["ssh_host"].as_str().unwrap_or("-");
                        let ssh_port = svc["ssh_port"].as_u64().unwrap_or(22);
                        let cert_auth = svc["ssh_ca_public_key"].is_string();
                        let ttl = svc["ssh_certificate_ttl_minutes"].as_u64().unwrap_or(30);
                        let principals = svc["ssh_allowed_principals"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|v| v.as_str())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_else(|| "-".to_string());

                        eprintln!();
                        eprintln!("SSH Host:       {ssh_host}:{ssh_port}");
                        eprintln!("Cert Auth:      {cert_auth}");
                        eprintln!("Principals:     {principals}");
                        eprintln!("Cert TTL:       {ttl} min");
                    }

                    eprintln!();
                    eprintln!("Proxy URL:  {}/api/v1/proxy/s/{slug}/", api.base_url_root());
                    print_permission_block(&svc);
                }
            }
            Ok(())
        }

        ServiceCommands::Delete { id, yes, auth } => {
            if !yes {
                eprint!("Delete service {id}? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let mut api = ApiClient::from_auth(&auth)?;
            api.delete_empty(&format!("/keys/{id}")).await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
                ),
                OutputFormat::Table => eprintln!("Service deleted."),
            }
            Ok(())
        }

        ServiceCommands::Update {
            id,
            label,
            endpoint_url,
            openapi_spec_url,
            node_id,
            no_node,
            active,
            inactive,
            default_header,
            clear_default_headers,
            ws_frame_preset,
            ws_frame_clear,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;

            let mut body = serde_json::Map::new();
            let ws_frame_injections =
                build_ws_frame_injections_body(ws_frame_preset.as_deref(), ws_frame_clear)?;
            let has_ws_frame_update = ws_frame_injections.is_some();

            if let Some(l) = label {
                body.insert("label".into(), Value::String(l));
            }
            if let Some(url) = endpoint_url {
                body.insert("endpoint_url".into(), Value::String(url));
            }
            // PUT /keys/{id} treats empty string as "clear" for openapi_spec_url.
            if let Some(url) = openapi_spec_url {
                body.insert("openapi_spec_url".into(), Value::String(url));
            }
            if no_node {
                body.insert("node_id".into(), Value::String(String::new()));
            } else if let Some(nid) = node_id {
                // Accept either a node ID or a node name (from `nyxid node list`).
                let resolved = crate::commands::node::resolve_node_id(&mut api, &nid)
                    .await
                    .with_context(|| format!("Could not resolve node '{nid}'"))?;
                body.insert("node_id".into(), Value::String(resolved));
            }
            if active {
                body.insert("is_active".into(), Value::Bool(true));
            } else if inactive {
                body.insert("is_active".into(), Value::Bool(false));
            }

            // NyxID#356: default_request_headers.
            //   --clear-default-headers     -> null (clears)
            //   any --default-header flags  -> replace with parsed list
            //   neither                     -> field omitted (no change)
            if clear_default_headers {
                body.insert("default_request_headers".into(), Value::Null);
            } else if !default_header.is_empty() {
                let parsed = parse_default_headers(&default_header)?;
                body.insert(
                    "default_request_headers".into(),
                    serde_json::to_value(parsed)?,
                );
            }

            let mut result: Value = if body.is_empty() && has_ws_frame_update {
                api.get(&format!("/keys/{id}")).await?
            } else {
                api.put(&format!("/keys/{id}"), &Value::Object(body))
                    .await?
            };

            if let Some(rules) = ws_frame_injections {
                let service_id = result["user_service_id"]
                    .as_str()
                    .or(result["id"].as_str())
                    .or(result["_id"].as_str())
                    .ok_or_else(|| anyhow::anyhow!("Could not resolve user_service_id"))?
                    .to_string();
                let body = serde_json::json!({ "ws_frame_injections": rules });
                let _: Value = api
                    .put(&format!("/user-services/{service_id}"), &body)
                    .await?;
                result = api.get(&format!("/keys/{id}")).await?;
            }

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Service updated.");
                    print_permission_block(&result);
                }
            }
            Ok(())
        }

        ServiceCommands::RotateCredential {
            id,
            credential_env,
            credential,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;

            // Fetch service to get the external api_key_id
            let svc: Value = api.get(&format!("/keys/{id}")).await?;
            let api_key_id = svc["api_key_id"]
                .as_str()
                .or(svc["external_key_id"].as_str())
                .ok_or_else(|| anyhow::anyhow!("No external credential found for this service"))?;
            if svc["credential_type"].as_str() == Some("node_managed") {
                bail!("This service is node-managed. Update the credential on the node instead.");
            }

            let credential = if let Some(c) = credential {
                c
            } else if let Some(env_var) = &credential_env {
                std::env::var(env_var)
                    .with_context(|| format!("Environment variable {env_var} not set"))?
            } else {
                prompt_password("New credential: ", "credential")?
            };
            if credential.is_empty() {
                bail!("Credential is required");
            }

            let body = serde_json::json!({ "credential": credential });
            let _: Value = api
                .put(&format!("/api-keys/external/{api_key_id}"), &body)
                .await?;

            eprintln!("Credential rotated for service {id}.");
            Ok(())
        }

        ServiceCommands::Route {
            id,
            node,
            direct,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;

            // Accept either a node ID or a node name (from `nyxid node list`).
            let node = match node {
                Some(raw) => Some(
                    crate::commands::node::resolve_node_id(&mut api, &raw)
                        .await
                        .with_context(|| format!("Could not resolve node '{raw}'"))?,
                ),
                None => None,
            };

            let svc: Value = api.get(&format!("/keys/{id}")).await?;
            let service_id = svc["user_service_id"]
                .as_str()
                .or(svc["id"].as_str())
                .or(svc["_id"].as_str())
                .ok_or_else(|| anyhow::anyhow!("Could not resolve user_service_id"))?;

            let body = build_route_body(direct, node.as_deref())?;
            let _: Value = api
                .put(&format!("/user-services/{service_id}"), &body)
                .await?;

            if direct {
                eprintln!("Service {id} set to direct routing.");
            } else {
                eprintln!("Service {id} routed through node.");
            }
            Ok(())
        }

        ServiceCommands::Credentials {
            slug,
            client_id_env,
            client_id,
            client_secret_env,
            client_secret,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;

            // Fetch catalog entry to get provider_id
            let catalog: Value = api.get(&format!("/catalog/{slug}")).await?;
            let provider_id = catalog["provider_config_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Could not find provider_id for slug: {slug}"))?;

            let client_id = if let Some(c) = client_id {
                c
            } else if let Some(env_var) = &client_id_env {
                std::env::var(env_var)
                    .with_context(|| format!("Environment variable {env_var} not set"))?
            } else {
                let mut input = String::new();
                eprint!("OAuth client ID: ");
                std::io::stderr().flush()?;
                std::io::stdin().read_line(&mut input)?;
                input.trim().to_string()
            };
            if client_id.is_empty() {
                bail!("Client ID is required");
            }

            let client_secret = if let Some(c) = client_secret {
                c
            } else if let Some(env_var) = &client_secret_env {
                std::env::var(env_var)
                    .with_context(|| format!("Environment variable {env_var} not set"))?
            } else {
                prompt_password("OAuth client secret: ", "client-secret")?
            };
            if client_secret.is_empty() {
                bail!("Client secret is required");
            }

            let body = serde_json::json!({
                "client_id": client_id,
                "client_secret": client_secret,
            });

            let result: Value = api
                .put(&format!("/providers/{provider_id}/credentials"), &body)
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("OAuth credentials set for {slug}.");
                }
            }
            Ok(())
        }
    }
}

// ---- OAuth add flow (I21) ----

async fn run_oauth_add(
    api: &mut ApiClient,
    slug: Option<String>,
    options: CatalogAddFlowOptions<'_>,
    auth: &crate::cli::AuthArgs,
) -> Result<()> {
    let slug = slug.ok_or_else(|| anyhow::anyhow!("Catalog slug is required for --oauth"))?;

    // Fetch catalog to get provider info and create a placeholder unified key first.
    let catalog: Value = api.get(&format!("/catalog/{slug}")).await?;
    let label = catalog["name"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| slug.clone());

    let mut key_body = serde_json::Map::new();
    key_body.insert("service_slug".into(), Value::String(slug.clone()));
    key_body.insert("label".into(), Value::String(label));
    if let Some(custom_slug) = options.custom_slug {
        key_body.insert("slug".into(), Value::String(custom_slug.to_string()));
    }
    if let Some(node_id) = options.via_node {
        key_body.insert("node_id".into(), Value::String(node_id.to_string()));
    }
    if let Some(org_id) = options.target_org_id {
        key_body.insert("target_org_id".into(), Value::String(org_id.to_string()));
    }
    // Forward the three-state spec URL as-is: `None` omits the field so the
    // catalog default applies; `Some("")` opts out; `Some(url)` overrides.
    if let Some(url) = options.openapi_spec_url {
        key_body.insert("openapi_spec_url".into(), Value::String(url.to_string()));
    }
    if let Some(rules) = options.ws_frame_injections {
        key_body.insert("ws_frame_injections".into(), rules.clone());
    }
    let key_result: Value = api.post("/keys", &Value::Object(key_body)).await?;
    let key_id = key_result["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Created key response did not include an id"))?;

    if options.via_node.is_some() {
        // For node-routed services the OAuth flow runs on the node itself, so
        // we can't kick off the browser here. Print a copy-paste friendly
        // next-step command that includes any --scope values the caller
        // passed, so the node-side `credentials setup` picks up the same
        // extra scopes (see `cli/src/node/agent.rs::cmd_credentials_setup`).
        print_add_result(api, &key_result, auth.output)?;
        eprintln!();
        eprintln!("Next step: run this on the node that owns the credential:");
        let scope_suffix = format_scope_suffix(options.additional_scopes);
        eprintln!("  nyxid node credentials setup --service {slug}{scope_suffix}");
        return Ok(());
    }

    let provider_id = catalog["provider_config_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No provider found for slug: {slug}"))?;
    let redirect_path = format!("/keys/{key_id}");
    let mut initiate_path = format!(
        "/providers/{provider_id}/connect/oauth?redirect_path={}",
        urlencoding::encode(&redirect_path)
    );
    if !options.additional_scopes.is_empty() {
        initiate_path.push_str(&format!(
            "&scope={}",
            urlencoding::encode(&options.additional_scopes.join(","))
        ));
    }
    // Org-targeted OAuth: the provider token must be stored under the org's
    // user_id so `sync_provider_token_to_api_keys` picks up the placeholder
    // UserApiKey we just created under the same org id. Without this query
    // param, the token would land on the admin's personal scope and the
    // org-owned UserApiKey would stay pending_auth forever.
    if let Some(org_id) = options.target_org_id {
        initiate_path.push_str(&format!("&target_org_id={}", urlencoding::encode(org_id)));
    }
    let initiate: Value = api.get(&initiate_path).await?;
    let authorization_url = initiate["authorization_url"].as_str().ok_or_else(|| {
        anyhow::anyhow!("OAuth initiate response did not include authorization_url")
    })?;

    eprintln!("Opening browser for OAuth authorization...");
    eprintln!();
    eprintln!("If the browser does not open, visit:");
    eprintln!("  {authorization_url}");
    eprintln!();

    let _ = open::that(authorization_url);

    let final_key = wait_for_authorized_key(api, key_id).await?;
    print_add_result(api, &final_key, auth.output)?;

    Ok(())
}

// ---- Device code add flow (I22) ----

fn parse_device_code_deadline(initiate: &Value) -> std::time::Instant {
    use chrono::DateTime;

    if let Some(s) = initiate["expires_at"].as_str()
        && let Ok(at) = DateTime::parse_from_rfc3339(s)
    {
        let secs = (at.timestamp() - chrono::Utc::now().timestamp()).max(0) as u64;
        return std::time::Instant::now() + std::time::Duration::from_secs(secs);
    }

    if let Some(secs) = initiate["expires_in"]
        .as_u64()
        .or_else(|| initiate["expires_in"].as_str().and_then(|s| s.parse().ok()))
    {
        return std::time::Instant::now() + std::time::Duration::from_secs(secs);
    }

    std::time::Instant::now() + std::time::Duration::from_secs(15 * 60)
}

async fn run_device_code_add(
    api: &mut ApiClient,
    slug: Option<String>,
    options: CatalogAddFlowOptions<'_>,
    auth: &crate::cli::AuthArgs,
) -> Result<()> {
    let slug = slug.ok_or_else(|| anyhow::anyhow!("Catalog slug is required for --device-code"))?;

    // Fetch catalog to get provider info and create a placeholder unified key first.
    let catalog: Value = api.get(&format!("/catalog/{slug}")).await?;
    let label = catalog["name"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| slug.clone());
    let mut key_body = serde_json::Map::new();
    key_body.insert("service_slug".into(), Value::String(slug.clone()));
    key_body.insert("label".into(), Value::String(label));
    if let Some(custom_slug) = options.custom_slug {
        key_body.insert("slug".into(), Value::String(custom_slug.to_string()));
    }
    if let Some(node_id) = options.via_node {
        key_body.insert("node_id".into(), Value::String(node_id.to_string()));
    }
    if let Some(org_id) = options.target_org_id {
        key_body.insert("target_org_id".into(), Value::String(org_id.to_string()));
    }
    if let Some(url) = options.openapi_spec_url {
        key_body.insert("openapi_spec_url".into(), Value::String(url.to_string()));
    }
    if let Some(rules) = options.ws_frame_injections {
        key_body.insert("ws_frame_injections".into(), rules.clone());
    }
    let key_result: Value = api.post("/keys", &Value::Object(key_body)).await?;
    let key_id = key_result["id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Created key response did not include an id"))?;

    if options.via_node.is_some() {
        print_add_result(api, &key_result, auth.output)?;
        eprintln!();
        eprintln!("Next step: run this on the node that owns the credential:");
        let scope_suffix = format_scope_suffix(options.additional_scopes);
        eprintln!("  nyxid node credentials setup --service {slug}{scope_suffix}");
        return Ok(());
    }

    let provider_id = catalog["provider_config_id"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No provider found for slug: {slug}"))?;

    // Initiate device code flow. Include `target_org_id` when present so the
    // provider token lands under the org's user_id (see the OAuth branch
    // above for the invariant).
    let mut initiate_path = format!("/providers/{provider_id}/connect/device-code/initiate");
    let mut first_param = true;
    let mut append = |path: &mut String, key: &str, val: &str| {
        path.push(if first_param { '?' } else { '&' });
        first_param = false;
        path.push_str(key);
        path.push('=');
        path.push_str(&urlencoding::encode(val));
    };
    if !options.additional_scopes.is_empty() {
        append(
            &mut initiate_path,
            "scope",
            &options.additional_scopes.join(","),
        );
    }
    if let Some(org_id) = options.target_org_id {
        append(&mut initiate_path, "target_org_id", org_id);
    }
    let initiate: Value = api.post(&initiate_path, &serde_json::json!({})).await?;

    let user_code = initiate["user_code"].as_str().unwrap_or("-");
    let verification_uri = initiate["verification_uri"]
        .as_str()
        .or(initiate["verification_url"].as_str())
        .unwrap_or("-");
    let state = initiate["state"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Device code initiate response did not include state"))?;
    let mut interval = initiate["interval"]
        .as_u64()
        .or_else(|| initiate["interval"].as_str().and_then(|s| s.parse().ok()))
        .unwrap_or(5);
    let deadline = parse_device_code_deadline(&initiate);

    eprintln!("Device Code Authorization");
    eprintln!();
    eprintln!("  Code: {user_code}");
    eprintln!("  URL:  {verification_uri}");
    eprintln!();
    eprintln!("Enter the code at the URL above, then wait for authorization...");

    let _ = open::that(verification_uri);

    // Poll for completion
    let poll_body = serde_json::json!({ "state": state });
    let poll_path = format!("/providers/{provider_id}/connect/device-code/poll");
    let mut consecutive_poll_errors = 0_u8;

    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_secs(interval)).await;

        match api.post::<Value, _>(&poll_path, &poll_body).await {
            Ok(result) => {
                consecutive_poll_errors = 0;
                let status = result["status"].as_str().unwrap_or("");
                if status == "complete"
                    || status == "authorized"
                    || result["access_token"].is_string()
                {
                    eprintln!("Authorization successful!");
                    eprintln!();
                    let key_result: Value = api.get(&format!("/keys/{key_id}")).await?;

                    match auth.output {
                        crate::cli::OutputFormat::Json => {
                            println!("{}", serde_json::to_string_pretty(&key_result)?);
                        }
                        crate::cli::OutputFormat::Table => {
                            print_add_result(api, &key_result, auth.output)?;
                        }
                    }
                    return Ok(());
                }
                if status == "expired" {
                    bail!("Device code authentication expired before completion");
                }
                if status == "denied" {
                    bail!("Device code authentication was denied");
                }
                if status == "slow_down" {
                    interval = result["interval"]
                        .as_u64()
                        .or_else(|| result["interval"].as_str().and_then(|s| s.parse().ok()))
                        .unwrap_or(interval + 5);
                }
                // Still pending, continue polling
                eprint!(".");
                std::io::stderr().flush()?;
            }
            Err(_) => {
                consecutive_poll_errors += 1;
                eprint!(".");
                std::io::stderr().flush()?;
                if consecutive_poll_errors >= 30 {
                    eprintln!();
                    bail!("device code polling failed repeatedly — check your network and re-run");
                }
            }
        }
    }

    eprintln!();
    bail!(
        "Device code authorization timed out (the code may have expired or the request was denied).\n\
         Re-run the command to start a new authorization."
    );
}

async fn wait_for_authorized_key(api: &mut ApiClient, key_id: &str) -> Result<Value> {
    const MAX_ATTEMPTS: usize = 150;
    const POLL_INTERVAL_SECS: u64 = 2;

    eprintln!("Waiting for authorization to complete...");

    for _ in 0..MAX_ATTEMPTS {
        tokio::time::sleep(std::time::Duration::from_secs(POLL_INTERVAL_SECS)).await;
        let key: Value = api.get(&format!("/keys/{key_id}")).await?;
        let status = key["status"].as_str().unwrap_or("");
        match status {
            "pending_auth" => {
                eprint!(".");
                std::io::stderr().flush()?;
            }
            "active" => {
                eprintln!();
                return Ok(key);
            }
            other => {
                eprintln!();
                bail!("OAuth flow finished with unexpected key status: {other}");
            }
        }
    }

    eprintln!();
    bail!("Timed out waiting for OAuth authorization to complete")
}

/// Format user-supplied extra scopes as a trailing ` --scope "a,b,c"` suffix
/// for a copy-paste friendly next-step hint shown after `service add --oauth
/// --via-node`. Returns an empty string when there are no extras, so the
/// pre-existing hint format is unchanged in the common case.
fn format_scope_suffix(additional_scopes: &[String]) -> String {
    if additional_scopes.is_empty() {
        String::new()
    } else {
        format!(" --scope \"{}\"", additional_scopes.join(","))
    }
}

fn print_add_result(api: &ApiClient, result: &Value, output: OutputFormat) -> Result<()> {
    match output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(result)?);
        }
        OutputFormat::Table => {
            let slug = result["slug"]
                .as_str()
                .or(result["service_slug"].as_str())
                .unwrap_or("-");
            let endpoint = result["endpoint_url"].as_str().unwrap_or("-");
            let status = result["status"].as_str().unwrap_or("active");

            eprintln!("Service added successfully!");
            eprintln!();
            eprintln!("Slug:      {slug}");
            eprintln!("Endpoint:  {endpoint}");
            eprintln!("Status:    {status}");
            eprintln!();
            eprintln!("Proxy URL: {}/api/v1/proxy/s/{slug}/", api.base_url_root());
            print_permission_block(result);
        }
    }
    Ok(())
}

fn ensure_stdin_is_tty(flag: &str) -> Result<()> {
    if std::io::stdin().is_terminal() {
        Ok(())
    } else {
        bail!("stdin is not a TTY; pass --{flag} or run from an interactive shell");
    }
}

fn prompt_line(prompt: &str, flag: &str) -> Result<String> {
    ensure_stdin_is_tty(flag)?;
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        bail!("Input is required");
    }
    Ok(trimmed)
}

fn prompt_line_default(prompt: &str, default: &str, flag: &str) -> Result<String> {
    ensure_stdin_is_tty(flag)?;
    eprint!("{prompt}");
    std::io::stderr().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn prompt_password(prompt: &str, flag: &str) -> Result<String> {
    ensure_stdin_is_tty(flag)?;
    Ok(rpassword::prompt_password(prompt)?)
}

/// Returns true when the CLI is running somewhere we can't reasonably
/// open a local browser for the wizard. In those cases `service add`
/// falls through to the pre-wizard rpassword path so SSH / CI / remote
/// dev users keep the old in-terminal credential prompt.
///
/// Checks, in order:
/// - `NYXID_NO_WIZARD` set to anything → explicit opt-out
/// - `SSH_CONNECTION` / `SSH_TTY` set → SSH session (no local display)
/// - Linux-only: both `DISPLAY` and `WAYLAND_DISPLAY` unset → no X/Wayland
///
/// We intentionally skip this detection on macOS / Windows when not in
/// SSH — there's always a GUI available, so the wizard should run.
/// Retained (behind `#[cfg(test)]`) because the headless-detection
/// tests at the bottom of this file still exercise the exact same
/// logic that `wizard::is_wizard_eligible` encodes canonically. The
/// tests guard against a regression where the two predicates drift;
/// if `is_wizard_eligible` changes rules, these tests fail first.
#[cfg(test)]
fn is_headless_environment() -> bool {
    if std::env::var_os("NYXID_NO_WIZARD").is_some() {
        return true;
    }
    if std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some() {
        return true;
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return true;
        }
    }
    false
}

fn requires_auth_key_name_prompt(method: &str, auth_key_name_provided: bool) -> bool {
    !auth_key_name_provided && method != "bot_bearer" && method != "none"
}

fn requires_credential_prompt(method: &str, has_node: bool) -> bool {
    !has_node && method != "none"
}

/// Issue #414: should `service add` route to the wizard, or fall
/// through to the legacy scripted path? `true` = scripted (the
/// existing user-facing semantics, where the CLI either reads from
/// stdin or prints "Next step:" instructions).
///
/// Caller hands in pre-resolved booleans for each gating flag —
/// pure inputs → pure boolean keeps this unit-testable.
///
/// The nuance is `auth_method` / `auth_key_name` / `custom_slug`:
/// these are *override* flags on a catalog entry (existing scripted
/// use case), but *definitional* values for `--custom` (issue #414's
/// new wizard form). They only mark the call as scripted when used
/// WITHOUT `--custom`.
//
// One bool per CLI flag is the simplest mapping for this decision —
// extracting a struct would just relocate the same set of fields and
// obscure which flag drives which arm. Allow the long signature.
#[allow(clippy::too_many_arguments)]
fn is_explicit_scripted(
    has_credential: bool,
    has_credential_env: bool,
    oauth: bool,
    device_code: bool,
    custom: bool,
    has_custom_slug: bool,
    has_auth_method: bool,
    has_auth_key_name: bool,
    has_scopes: bool,
    has_openapi_spec_url: bool,
    has_ws_frame_preset: bool,
    ws_frame_clear: bool,
) -> bool {
    has_credential
        || has_credential_env
        || oauth
        || device_code
        || (has_auth_method && !custom)
        || (has_auth_key_name && !custom)
        || (has_custom_slug && !custom)
        || has_scopes
        || has_openapi_spec_url
        || has_ws_frame_preset
        || ws_frame_clear
}

/// Default auth key name for a given auth method. Mirrors the frontend
/// defaults in `add-key-dialog.tsx` so CLI and UI stay in sync.
fn default_auth_key_name(method: &str) -> &'static str {
    match method {
        "header" => "X-API-Key",
        "query" => "key",
        "path" => "bot",
        "body" => "app_secret",
        _ => "Authorization",
    }
}

/// Derive a credential input prompt from the auth method and key name so
/// users know what value they're entering (e.g. "Enter app_secret:" for
/// Lark body auth instead of a generic "API key/credential" label).
fn credential_prompt_label(auth_method: &str, auth_key_name: &str) -> String {
    match auth_method {
        "bot_bearer" => "Enter bot token: ".to_string(),
        "basic" => "Enter username:password: ".to_string(),
        "body" => {
            let field = auth_key_name.trim();
            if field.is_empty() {
                "Enter credential: ".to_string()
            } else {
                format!("Enter {field}: ")
            }
        }
        _ => "Enter API key/credential: ".to_string(),
    }
}

/// Credential field metadata pulled from the catalog entry. Drives the
/// dynamic multi-field prompt for `token_exchange` services so the CLI
/// doesn't need per-provider knowledge.
#[derive(Debug, Clone)]
struct TokenExchangeField {
    name: String,
    label: String,
    placeholder: Option<String>,
    secret: bool,
}

struct CatalogAddFlowOptions<'a> {
    custom_slug: Option<&'a str>,
    via_node: Option<&'a str>,
    additional_scopes: &'a [String],
    target_org_id: Option<&'a str>,
    openapi_spec_url: Option<&'a str>,
    ws_frame_injections: Option<&'a Value>,
}

/// Parse the `token_exchange_credential_fields` array out of a raw catalog
/// JSON response. Returns `None` when the field is missing / null / not an
/// array, and filters out malformed entries.
fn parse_token_exchange_fields(value: &Value) -> Option<Vec<TokenExchangeField>> {
    let arr = value.as_array()?;
    if arr.is_empty() {
        return None;
    }
    let fields: Vec<TokenExchangeField> = arr
        .iter()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?.to_string();
            let label = item
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or(&name)
                .to_string();
            let placeholder = item
                .get("placeholder")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let secret = item
                .get("secret")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            Some(TokenExchangeField {
                name,
                label,
                placeholder,
                secret,
            })
        })
        .collect();
    if fields.is_empty() {
        None
    } else {
        Some(fields)
    }
}

/// Interactive prompt for `token_exchange` services. Walks the
/// catalog-declared credential fields in order, prompting for each one
/// (visible input for public fields like `app_id`, hidden input for
/// `secret: true` fields like `app_secret`), then returns a compact JSON
/// object ready for the `credential` field on the create-key API.
///
/// For non-interactive flows, callers can pass the same JSON via
/// `--credential` or `--credential-env`.
fn prompt_token_exchange_credential(fields: &[TokenExchangeField]) -> Result<String> {
    eprintln!("Provider credentials (stored encrypted, never exposed):");
    let mut payload = serde_json::Map::with_capacity(fields.len());
    for field in fields {
        let prompt_label = match &field.placeholder {
            Some(ph) => format!("{} (e.g. {ph}): ", field.label),
            None => format!("{}: ", field.label),
        };
        let value = if field.secret {
            prompt_password(&prompt_label, "credential")?
        } else {
            prompt_line(&prompt_label, "credential")?
        };
        let trimmed = value.trim();
        if trimmed.is_empty() {
            bail!("{} is required", field.name);
        }
        payload.insert(field.name.clone(), Value::String(trimmed.to_string()));
    }
    Ok(Value::Object(payload).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_auth_skips_auth_key_name_and_credential_prompts() {
        assert!(!requires_auth_key_name_prompt("none", false));
        assert!(!requires_credential_prompt("none", false));
    }

    #[test]
    fn bearer_without_overrides_requires_auth_key_name_and_credential_prompts() {
        assert!(requires_auth_key_name_prompt("bearer", false));
        assert!(requires_credential_prompt("bearer", false));
    }

    #[test]
    fn bot_bearer_skips_auth_key_name_prompt_but_still_requires_credential() {
        assert!(!requires_auth_key_name_prompt("bot_bearer", false));
        assert!(requires_credential_prompt("bot_bearer", false));
    }

    #[test]
    fn node_routing_skips_credential_prompt_regardless_of_method() {
        assert!(!requires_credential_prompt("bearer", true));
        assert!(!requires_credential_prompt("none", true));
    }

    #[test]
    fn parse_device_code_deadline_uses_expires_at() {
        let future = chrono::Utc::now() + chrono::Duration::seconds(600);
        let v = serde_json::json!({ "expires_at": future.to_rfc3339() });
        let deadline = parse_device_code_deadline(&v);
        let secs = deadline.duration_since(std::time::Instant::now()).as_secs();
        assert!((550..=650).contains(&secs), "got {secs}s");
    }

    #[test]
    fn parse_device_code_deadline_uses_expires_in() {
        let v = serde_json::json!({ "expires_in": 600 });
        let deadline = parse_device_code_deadline(&v);
        let secs = deadline.duration_since(std::time::Instant::now()).as_secs();
        assert!((550..=650).contains(&secs), "got {secs}s");
    }

    #[test]
    fn parse_device_code_deadline_falls_back_to_default() {
        let v = serde_json::json!({});
        let deadline = parse_device_code_deadline(&v);
        let secs = deadline.duration_since(std::time::Instant::now()).as_secs();
        assert!((850..=900).contains(&secs), "got {secs}s");
    }

    // ── Issue #414: explicit_scripted dispatch nuance ─────────────────

    /// Test helper: build a clean "no flags" baseline so each test
    /// can flip the one bit it cares about without restating thirteen
    /// `false` arguments.
    #[allow(clippy::too_many_arguments)]
    fn scripted(
        custom: bool,
        has_credential: bool,
        has_credential_env: bool,
        oauth: bool,
        device_code: bool,
        has_custom_slug: bool,
        has_auth_method: bool,
        has_auth_key_name: bool,
    ) -> bool {
        is_explicit_scripted(
            has_credential,
            has_credential_env,
            oauth,
            device_code,
            custom,
            has_custom_slug,
            has_auth_method,
            has_auth_key_name,
            false, // has_scopes
            false, // has_openapi_spec_url
            false, // has_ws_frame_preset
            false, // ws_frame_clear
        )
    }

    #[test]
    fn issue_414_no_flags_routes_to_wizard() {
        // `nyxid service add` with no flags falls through to the
        // wizard (catalog grid). Existing behavior, preserved.
        assert!(!scripted(
            false, false, false, false, false, false, false, false
        ));
    }

    #[test]
    fn issue_414_custom_alone_routes_to_wizard() {
        // `service add --custom` — was scripted (stdin prompts) before
        // issue #414, now routes to wizard so user lands on the
        // custom-service form.
        assert!(!scripted(
            true, false, false, false, false, false, false, false
        ));
    }

    #[test]
    fn issue_414_custom_with_definitional_flags_routes_to_wizard() {
        // The exact issue repro: `--custom --auth-method bearer
        // --auth-key-name Authorization --slug X`. All three flags
        // are *definitional* values for the wizard form when paired
        // with `--custom`.
        assert!(!scripted(
            true, // custom
            false, false, false, false, true, // has_custom_slug
            true, // has_auth_method
            true, // has_auth_key_name
        ));
    }

    #[test]
    fn issue_414_credential_always_keeps_scripted() {
        // `--credential foo` is the canonical "I'm scripting this"
        // marker — must stay scripted regardless of --custom. The
        // existing scripted-with-credential users shouldn't see any
        // behavior change.
        assert!(scripted(
            false, true, false, false, false, false, false, false
        ));
        assert!(scripted(
            true, true, false, false, false, false, false, false
        ));
    }

    #[test]
    fn issue_414_credential_env_always_keeps_scripted() {
        assert!(scripted(
            false, false, true, false, false, false, false, false
        ));
        assert!(scripted(
            true, false, true, false, false, false, false, false
        ));
    }

    #[test]
    fn issue_414_oauth_and_device_code_always_keep_scripted() {
        assert!(scripted(
            false, false, false, true, false, false, false, false
        ));
        assert!(scripted(
            false, false, false, false, true, false, false, false
        ));
    }

    #[test]
    fn issue_414_auth_method_alone_is_a_catalog_override_and_stays_scripted() {
        // `service add openai-chat --auth-method bearer` is the
        // existing "override the catalog entry's auth method" case.
        // Has been scripted forever; must stay scripted (no --custom
        // means it's an override, not a definitional value).
        assert!(scripted(
            false, false, false, false, false, false, true, false
        ));
    }

    #[test]
    fn issue_414_auth_key_name_alone_is_a_catalog_override_and_stays_scripted() {
        assert!(scripted(
            false, false, false, false, false, false, false, true
        ));
    }

    #[test]
    fn issue_414_custom_slug_alone_is_a_catalog_override_and_stays_scripted() {
        // `service add openai-chat --slug my-openai` — slug override
        // on a catalog entry. Stays scripted.
        assert!(scripted(
            false, false, false, false, false, true, false, false
        ));
    }

    #[test]
    fn org_scope_alone_routes_to_wizard() {
        // `--org` is resolved before this heuristic and then carried
        // as wizard prefill. It no longer forces the legacy scripted
        // terminal path.
        assert!(!scripted(
            false, false, false, false, false, false, false, false
        ));
    }

    #[test]
    fn issue_414_advanced_flags_keep_scripted() {
        // --scope / --openapi-spec-url / --ws-frame-preset /
        // --ws-frame-clear are advanced flags we don't want to expose
        // in the wizard form. They keep their existing scripted
        // semantics.
        for (scopes, spec_url, ws_preset, ws_clear) in [
            (true, false, false, false),
            (false, true, false, false),
            (false, false, true, false),
            (false, false, false, true),
        ] {
            assert!(
                is_explicit_scripted(
                    false, false, false, false, false, false, false, false, scopes, spec_url,
                    ws_preset, ws_clear,
                ),
                "expected scripted with scopes={scopes} spec_url={spec_url} \
                 ws_preset={ws_preset} ws_clear={ws_clear}",
            );
        }
    }

    #[test]
    fn default_auth_key_name_maps_known_methods() {
        assert_eq!(default_auth_key_name("bearer"), "Authorization");
        assert_eq!(default_auth_key_name("bot_bearer"), "Authorization");
        assert_eq!(default_auth_key_name("header"), "X-API-Key");
        assert_eq!(default_auth_key_name("query"), "key");
        assert_eq!(default_auth_key_name("path"), "bot");
        assert_eq!(default_auth_key_name("body"), "app_secret");
        assert_eq!(default_auth_key_name("unknown"), "Authorization");
    }

    #[test]
    fn credential_prompt_reflects_auth_method() {
        assert_eq!(
            credential_prompt_label("bearer", "Authorization"),
            "Enter API key/credential: "
        );
        assert_eq!(
            credential_prompt_label("bot_bearer", "Authorization"),
            "Enter bot token: "
        );
        assert_eq!(
            credential_prompt_label("basic", "Authorization"),
            "Enter username:password: "
        );
    }

    #[test]
    fn credential_prompt_body_uses_key_name() {
        assert_eq!(
            credential_prompt_label("body", "app_secret"),
            "Enter app_secret: "
        );
        assert_eq!(
            credential_prompt_label("body", "client_secret"),
            "Enter client_secret: "
        );
    }

    #[test]
    fn credential_prompt_body_without_key_name_falls_back() {
        assert_eq!(credential_prompt_label("body", ""), "Enter credential: ");
        assert_eq!(credential_prompt_label("body", "   "), "Enter credential: ");
    }

    #[test]
    fn custom_service_slug_validator_accepts_valid_slugs() {
        for slug in ["custom-service", "api2", "service-2026"] {
            validate_service_slug(slug)
                .unwrap_or_else(|e| panic!("expected '{slug}' to validate: {e}"));
        }
    }

    #[test]
    fn custom_service_slug_validator_rejects_invalid_slugs() {
        let too_long = "a".repeat(81);
        for slug in [
            "",
            "Bad-Slug",
            "-leading",
            "trailing-",
            "double--hyphen",
            "with_underscore",
            too_long.as_str(),
        ] {
            assert!(
                validate_service_slug(slug).is_err(),
                "expected '{slug}' to be rejected"
            );
        }
    }

    #[test]
    fn token_exchange_credential_json_shape() {
        // Round-trip check: the payload we build for a token_exchange
        // service must parse as a JSON object with the declared fields
        // so the backend's parse_credential succeeds.
        let sample = serde_json::json!({
            "app_id": "cli_test",
            "app_secret": "secret_value",
        })
        .to_string();
        let parsed: serde_json::Value = serde_json::from_str(&sample).unwrap();
        assert_eq!(parsed["app_id"], "cli_test");
        assert_eq!(parsed["app_secret"], "secret_value");
    }

    #[test]
    fn parse_token_exchange_fields_extracts_declared_fields() {
        let value = serde_json::json!([
            {
                "name": "app_id",
                "label": "App ID",
                "placeholder": "cli_xxx",
                "secret": false,
            },
            {
                "name": "app_secret",
                "label": "App Secret",
                "secret": true,
            },
        ]);
        let fields = parse_token_exchange_fields(&value).expect("fields");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].name, "app_id");
        assert_eq!(fields[0].placeholder.as_deref(), Some("cli_xxx"));
        assert!(!fields[0].secret);
        assert_eq!(fields[1].name, "app_secret");
        assert!(fields[1].secret);
        assert!(fields[1].placeholder.is_none());
    }

    #[test]
    fn parse_token_exchange_fields_returns_none_for_missing_or_empty() {
        assert!(parse_token_exchange_fields(&serde_json::Value::Null).is_none());
        assert!(parse_token_exchange_fields(&serde_json::json!([])).is_none());
        assert!(parse_token_exchange_fields(&serde_json::json!("not an array")).is_none());
    }

    #[test]
    fn add_ssh_body_with_org_includes_ssh_fields_and_target_org_id() {
        let body = Value::Object(build_add_ssh_body(AddSshBody {
            label: "prod-bastion",
            host: "bastion.internal",
            port: 2222,
            cert_auth: true,
            ssh_auth_mode: "cert",
            principals: "ubuntu,admin",
            ttl: 45,
            via_node: "node-123",
            target_org_id: Some("org-user-id"),
        }));

        assert_eq!(
            body,
            serde_json::json!({
                "label": "prod-bastion",
                "ssh_host": "bastion.internal",
                "ssh_port": 2222,
                "ssh_certificate_auth": true,
                "ssh_auth_mode": "cert",
                "ssh_principals": "ubuntu,admin",
                "ssh_certificate_ttl_minutes": 45,
                "node_id": "node-123",
                "target_org_id": "org-user-id",
            })
        );
    }

    #[test]
    fn add_ssh_body_without_org_omits_target_org_id() {
        let body = Value::Object(build_add_ssh_body(AddSshBody {
            label: "prod-bastion",
            host: "bastion.internal",
            port: 22,
            cert_auth: false,
            ssh_auth_mode: "proxy_only",
            principals: "",
            ttl: 30,
            via_node: "node-123",
            target_org_id: None,
        }));

        assert_eq!(
            body,
            serde_json::json!({
                "label": "prod-bastion",
                "ssh_host": "bastion.internal",
                "ssh_port": 22,
                "ssh_certificate_auth": false,
                "ssh_auth_mode": "proxy_only",
                "ssh_principals": "",
                "ssh_certificate_ttl_minutes": 30,
                "node_id": "node-123",
            })
        );
    }

    // Regression for issue #327: --direct must send "" so the backend
    // actually clears node_id. null is interpreted as "leave unchanged".
    #[test]
    fn route_direct_sends_empty_string_to_clear_node_id() {
        let body = build_route_body(true, None).unwrap();
        assert_eq!(body, serde_json::json!({ "node_id": "" }));
    }

    #[test]
    fn route_direct_sends_empty_string_even_when_node_is_provided() {
        let body = build_route_body(true, Some("node-a")).unwrap();
        assert_eq!(body, serde_json::json!({ "node_id": "" }));
    }

    #[test]
    fn route_via_node_sends_node_id() {
        let body = build_route_body(false, Some("node-a")).unwrap();
        assert_eq!(body, serde_json::json!({ "node_id": "node-a" }));
    }

    #[test]
    fn route_without_node_or_direct_errors() {
        assert!(build_route_body(false, None).is_err());
    }

    // Headless detection is env-sensitive, so these tests stash and
    // restore the relevant variables. We can't run them in parallel with
    // other env-touching tests, but the helper is pure enough that
    // serialising inside a single test is fine.
    #[test]
    fn headless_env_flags_recognised() {
        struct Guard {
            keys: Vec<&'static str>,
            original: Vec<(&'static str, Option<std::ffi::OsString>)>,
        }
        impl Guard {
            fn new(keys: &[&'static str]) -> Self {
                let original = keys
                    .iter()
                    .map(|k| (*k, std::env::var_os(k)))
                    .collect::<Vec<_>>();
                for k in keys {
                    // SAFETY: tests run single-threaded via the helper
                    // itself calling is_headless_environment(); we
                    // restore originals on drop.
                    unsafe {
                        std::env::remove_var(k);
                    }
                }
                Guard {
                    keys: keys.to_vec(),
                    original,
                }
            }
            fn set(&self, k: &str, v: &str) {
                unsafe {
                    std::env::set_var(k, v);
                }
            }
            fn unset(&self, k: &str) {
                unsafe {
                    std::env::remove_var(k);
                }
            }
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                for (k, v) in &self.original {
                    unsafe {
                        match v {
                            Some(val) => std::env::set_var(k, val),
                            None => std::env::remove_var(k),
                        }
                    }
                }
                let _ = &self.keys;
            }
        }

        let guard = Guard::new(&[
            "NYXID_NO_WIZARD",
            "SSH_CONNECTION",
            "SSH_TTY",
            "DISPLAY",
            "WAYLAND_DISPLAY",
        ]);

        // Explicit opt-out wins.
        guard.set("NYXID_NO_WIZARD", "1");
        assert!(is_headless_environment());
        guard.unset("NYXID_NO_WIZARD");

        // SSH session markers.
        guard.set("SSH_CONNECTION", "10.0.0.1 22 10.0.0.2 34567");
        assert!(is_headless_environment());
        guard.unset("SSH_CONNECTION");

        guard.set("SSH_TTY", "/dev/pts/0");
        assert!(is_headless_environment());
        guard.unset("SSH_TTY");

        // Linux-only: no display at all means headless. On non-Linux
        // we don't gate on display vars (macOS always has a GUI, Windows
        // similar), so just assert the fallback.
        #[cfg(target_os = "linux")]
        {
            assert!(is_headless_environment());
            guard.set("DISPLAY", ":0");
            assert!(!is_headless_environment());
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert!(!is_headless_environment());
        }
    }

    #[test]
    fn parse_default_header_simple_pair() {
        let parsed =
            parse_default_headers(&["x-openclaw-scopes=operator.read".to_string()]).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0]["name"], "x-openclaw-scopes");
        assert_eq!(parsed[0]["value"], "operator.read");
        assert_eq!(parsed[0]["overridable"], serde_json::Value::Bool(false));
        assert_eq!(parsed[0]["sensitive"], serde_json::Value::Bool(false));
    }

    #[test]
    fn parse_default_header_with_overridable_suffix() {
        let parsed = parse_default_headers(&["x-api-version=v2:overridable".to_string()]).unwrap();
        assert_eq!(parsed[0]["value"], "v2");
        assert_eq!(parsed[0]["overridable"], serde_json::Value::Bool(true));
    }

    #[test]
    fn parse_default_header_value_may_contain_equals_and_commas() {
        let parsed = parse_default_headers(&["x-scopes=a,b=c,d".to_string()]).unwrap();
        assert_eq!(parsed[0]["value"], "a,b=c,d");
    }

    #[test]
    fn parse_default_header_value_may_contain_url_with_port() {
        // Colons inside the value must survive; the suffix detector only
        // strips `:overridable` / `:sensitive` exact tails.
        let parsed =
            parse_default_headers(&["x-origin=https://example.com:8443".to_string()]).unwrap();
        assert_eq!(parsed[0]["value"], "https://example.com:8443");
        assert_eq!(parsed[0]["overridable"], serde_json::Value::Bool(false));
    }

    #[test]
    fn parse_default_header_rejects_missing_equals() {
        assert!(parse_default_headers(&["x-no-value".to_string()]).is_err());
    }

    #[test]
    fn parse_default_header_rejects_empty_name() {
        assert!(parse_default_headers(&["=v".to_string()]).is_err());
    }

    #[test]
    fn parse_default_header_accepts_empty_value() {
        // Backend will validate final shape; the parser itself does not
        // reject empty values — admin might legitimately want to emit an
        // empty header.
        let parsed = parse_default_headers(&["x-empty=".to_string()]).unwrap();
        assert_eq!(parsed[0]["value"], "");
    }

    #[test]
    fn ws_frame_home_assistant_preset_matches_backend_shape() {
        let payload = build_ws_frame_injections_body(Some("home-assistant"), false)
            .unwrap()
            .expect("preset should produce rules");

        assert_eq!(payload[0]["trigger"]["json_field_equals"]["path"], "$.type");
        assert_eq!(
            payload[0]["trigger"]["json_field_equals"]["value"],
            "auth_required"
        );
        assert_eq!(
            payload[0]["template"],
            "{\"type\":\"auth\",\"access_token\":\"${credential}\"}"
        );
        assert_eq!(payload[0]["frame_kind"], "text");
        assert_eq!(payload[0]["consume_trigger"], true);
        assert_eq!(payload[0]["direction"], "downstream");
    }

    #[test]
    fn ws_frame_clear_builds_empty_rule_list() {
        let payload = build_ws_frame_injections_body(None, true)
            .unwrap()
            .expect("clear should produce an explicit payload");

        assert_eq!(payload.as_array().map(Vec::len), Some(0));
    }

    #[test]
    fn ws_frame_unknown_preset_is_rejected() {
        assert!(build_ws_frame_injections_body(Some("other"), false).is_err());
    }
}
