//! Executable production task inventory. Module walker follows the JWT verifier inventory.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    str::FromStr,
};

use proc_macro2::{Delimiter, TokenStream, TokenTree};

type Inventory = BTreeMap<(String, String, String), usize>;

#[derive(Debug, PartialEq)]
enum Tok {
    Ident(String),
    Punct(char),
    Open(Delimiter),
    Close,
    Literal,
}

fn flatten(stream: TokenStream, out: &mut Vec<Tok>) {
    for tree in stream {
        match tree {
            TokenTree::Ident(ident) => out.push(Tok::Ident(ident.to_string())),
            TokenTree::Punct(punct) => out.push(Tok::Punct(punct.as_char())),
            TokenTree::Literal(_) => out.push(Tok::Literal),
            TokenTree::Group(group) => {
                out.push(Tok::Open(group.delimiter()));
                flatten(group.stream(), out);
                out.push(Tok::Close);
            }
        }
    }
}

fn ident(tok: Option<&Tok>, name: &str) -> bool {
    matches!(tok, Some(Tok::Ident(found)) if found == name)
}

fn punct(tok: Option<&Tok>, ch: char) -> bool {
    matches!(tok, Some(Tok::Punct(found)) if *found == ch)
}

/// Scan all spawn spellings, including aliases and JoinSet methods. Tracking
/// wrappers are recognized by their receiver, never just by a function name.
fn scan(stream: TokenStream, file: &str, function: &str, found: &mut Inventory) {
    let mut toks = Vec::new();
    flatten(stream, &mut toks);
    for index in 0..toks.len() {
        let Tok::Ident(name) = &toks[index] else {
            continue;
        };
        if ident(index.checked_sub(1).and_then(|i| toks.get(i)), "fn") {
            continue;
        }
        if matches!(
            name.as_str(),
            "spawn" | "spawn_blocking" | "spawn_local" | "spawn_on" | "spawn_local_on"
        ) {
            assert!(
                !ident(toks.get(index + 1), "as"),
                "renamed spawn imports/casts hide calls from the shutdown inventory; keep the qualified spelling"
            );
        }
        let call = matches!(toks.get(index + 1), Some(Tok::Open(Delimiter::Parenthesis)));
        let generic_call = punct(toks.get(index + 1), ':')
            && punct(toks.get(index + 2), ':')
            && punct(toks.get(index + 3), '<');
        if !call && !generic_call {
            continue;
        }
        let primitive = match name.as_str() {
            "new" if ident(toks.get(index.wrapping_sub(3)), "Sse") => "sse",
            "from_stream" => "body_stream",
            "spawn"
            | "spawn_blocking"
            | "spawn_local"
            | "spawn_on"
            | "spawn_local_on"
            | "on_upgrade"
            | "spawn_worker"
            | "spawn_cleanup"
            | "spawn_joined"
            | "spawn_worker_joined"
            | "spawn_completion_worker" => name.as_str(),
            _ => continue,
        };
        let receiver_shutdown = punct(toks.get(index.wrapping_sub(1)), '.')
            && (ident(toks.get(index.wrapping_sub(2)), "shutdown")
                || (ident(toks.get(index.wrapping_sub(6)), "shutdown")
                    && ident(toks.get(index.wrapping_sub(4)), "clone")));
        let process_receiver = index >= 9
            && punct(toks.get(index - 1), '.')
            && matches!(toks.get(index - 2), Some(Tok::Close))
            && matches!(toks.get(index - 3), Some(Tok::Open(Delimiter::Parenthesis)))
            && ident(toks.get(index - 4), "process")
            && ident(toks.get(index - 7), "shutdown");
        let tracked_upgrade = primitive == "on_upgrade"
            && {
                let mut depth = 0;
                toks[index+1..].iter().take_while(|tok| {
                match tok { Tok::Open(_) => depth+=1, Tok::Close => depth-=1, _=>{} }
                depth > 0
            }).any(|tok| matches!(tok, Tok::Ident(id) if id == "upgrade_callback" || id == "upgrade_completion_callback"))
            };
        let primitive = if receiver_shutdown || process_receiver || tracked_upgrade {
            "tracked".to_owned()
        } else if matches!(primitive, "sse" | "body_stream") {
            primitive.to_owned()
        } else if punct(toks.get(index.wrapping_sub(1)), '.') {
            format!("method::{name}")
        } else {
            // Preserve the callee path: replacing a tracked helper with a raw
            // Tokio spawn in the same function must not reuse its exception.
            let mut path = vec![name.clone()];
            let mut at = index;
            while at >= 3 && punct(toks.get(at - 1), ':') && punct(toks.get(at - 2), ':') {
                let Some(Tok::Ident(part)) = toks.get(at - 3) else {
                    break;
                };
                path.push(part.clone());
                at -= 3;
            }
            path.reverse();
            path.join("::")
        };
        *found
            .entry((file.to_owned(), function.to_owned(), primitive))
            .or_default() += 1;
    }
}

struct Walk<'a> {
    root: &'a Path,
    production: Inventory,
    reached: BTreeSet<PathBuf>,
}

struct Context<'a> {
    file: &'a str,
    file_dir: &'a Path,
    /// Directory holding files of `mod child;` declarations at this level.
    child_dir: PathBuf,
    /// Outside any inline module block (where `#[path]` is file-relative).
    top_level: bool,
    test_only: bool,
}

struct Item {
    tokens: Vec<TokenTree>,
    cfg_test: bool,
    path_attr: Option<String>,
}

fn is_brace(tree: &TokenTree) -> bool {
    matches!(tree, TokenTree::Group(group) if group.delimiter() == Delimiter::Brace)
}

fn is_punct(tree: &TokenTree, ch: char) -> bool {
    matches!(tree, TokenTree::Punct(punct) if punct.as_char() == ch)
}

fn ident_at(tokens: &[TokenTree], index: usize) -> Option<String> {
    match tokens.get(index) {
        Some(TokenTree::Ident(ident)) => Some(ident.to_string()),
        _ => None,
    }
}

/// Split one item level (file, inline module, impl or trait body) into items.
/// An item ends at `;`, or at a brace group unless it is a `use`, `const`,
/// `static`, `type` or `let` item (so `use a::{b, c};` and
/// `const X: T = S { .. };` stay one item).
fn items(stream: TokenStream) -> Vec<Item> {
    let mut out = Vec::new();
    let mut current = Item {
        tokens: Vec::new(),
        cfg_test: false,
        path_attr: None,
    };
    let mut trees = stream.into_iter().peekable();
    while let Some(tree) = trees.next() {
        if current.tokens.is_empty() && is_punct(&tree, '#') {
            let inner = trees.peek().is_some_and(|next| is_punct(next, '!'));
            if inner {
                trees.next();
            }
            if let Some(TokenTree::Group(attr)) = trees.next() {
                let text = attr.stream().to_string().replace(' ', "");
                if !inner && text == "cfg(test)" {
                    current.cfg_test = true;
                } else if let Some(path) = text.strip_prefix("path=") {
                    current.path_attr = Some(path.trim_matches('"').to_owned());
                }
            }
            continue;
        }
        let ends = is_punct(&tree, ';')
            || (is_brace(&tree)
                && !keyword(&current.tokens).is_some_and(|(_, keyword)| {
                    matches!(
                        keyword.as_str(),
                        "use" | "extern" | "const" | "static" | "type" | "let"
                    )
                }));
        current.tokens.push(tree);
        if ends {
            out.push(std::mem::replace(
                &mut current,
                Item {
                    tokens: Vec::new(),
                    cfg_test: false,
                    path_attr: None,
                },
            ));
        }
    }
    if !current.tokens.is_empty() {
        out.push(current);
    }
    out
}

impl Walk<'_> {
    fn file(&mut self, path: &Path, child_dir: PathBuf, test_only: bool) {
        assert!(
            self.reached.insert(path.to_owned()),
            "module file reached twice: {}",
            path.display()
        );
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let stream = TokenStream::from_str(&source)
            .unwrap_or_else(|error| panic!("lex {}: {error}", path.display()));
        let file = path
            .strip_prefix(self.root)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        let context = Context {
            file: &file,
            file_dir: path.parent().unwrap(),
            child_dir,
            top_level: true,
            test_only,
        };
        self.level(stream, &context);
    }

    fn level(&mut self, stream: TokenStream, context: &Context<'_>) {
        for item in items(stream) {
            let test_only = context.test_only || item.cfg_test;
            let tokens = &item.tokens;
            let Some((index, keyword)) = keyword(tokens) else {
                continue;
            };
            let inner = |child_dir: PathBuf| Context {
                child_dir,
                top_level: false,
                test_only,
                ..*context
            };
            match keyword.as_str() {
                "use" if !test_only => scan(
                    tokens.iter().cloned().collect(),
                    context.file,
                    "<imports>",
                    &mut self.production,
                ),
                "use" | "extern" => {}
                "mod" => {
                    let name = ident_at(tokens, index + 1).expect("module name");
                    match tokens.last() {
                        Some(TokenTree::Group(body)) if is_brace(tokens.last().unwrap()) => {
                            assert!(
                                item.path_attr.is_none(),
                                "inline #[path] module in {}",
                                context.file
                            );
                            self.level(body.stream(), &inner(context.child_dir.join(&name)));
                        }
                        _ => {
                            let path = match &item.path_attr {
                                Some(relative) => {
                                    assert!(
                                        context.top_level,
                                        "#[path] inside inline module in {}",
                                        context.file
                                    );
                                    context.file_dir.join(relative)
                                }
                                None => {
                                    let flat = context.child_dir.join(format!("{name}.rs"));
                                    if flat.exists() {
                                        flat
                                    } else {
                                        context.child_dir.join(&name).join("mod.rs")
                                    }
                                }
                            };
                            let nested = if item.path_attr.is_some()
                                || path.file_name().is_some_and(|name| name == "mod.rs")
                            {
                                path.parent().unwrap().to_owned()
                            } else {
                                path.with_extension("")
                            };
                            self.file(&path, nested, test_only);
                        }
                    }
                }
                "impl" | "trait" => {
                    if let Some(TokenTree::Group(body)) = tokens.last() {
                        self.level(body.stream(), &inner(context.child_dir.clone()));
                    }
                }
                "<item>" if ident_at(tokens, index).as_deref() == Some("include") => {
                    // `include!("x.rs")` splices another file into this module.
                    let Some(TokenTree::Group(arguments)) = tokens.get(index + 2) else {
                        panic!("unexpected include! form in {}", context.file);
                    };
                    let relative = arguments.stream().to_string();
                    let path = context.file_dir.join(relative.trim_matches('"'));
                    self.file(&path, context.child_dir.clone(), test_only);
                }
                _ if test_only => {}
                "fn" => {
                    let name = ident_at(tokens, index + 1).expect("function name");
                    scan(
                        tokens.iter().cloned().collect(),
                        context.file,
                        &name,
                        &mut self.production,
                    );
                }
                _ => scan(
                    tokens.iter().cloned().collect(),
                    context.file,
                    "<module>",
                    &mut self.production,
                ),
            }
        }
    }
}

/// The defining keyword of an item, skipping visibility and qualifiers.
fn keyword(tokens: &[TokenTree]) -> Option<(usize, String)> {
    const KEYWORDS: [&str; 14] = [
        "fn",
        "mod",
        "impl",
        "trait",
        "use",
        "extern",
        "struct",
        "enum",
        "union",
        "type",
        "static",
        "const",
        "macro_rules",
        "let",
    ];
    for (index, tree) in tokens.iter().enumerate() {
        let TokenTree::Ident(ident) = tree else {
            continue;
        };
        let ident = ident.to_string();
        if ident == "const"
            && ident_at(tokens, index + 1)
                .is_some_and(|next| matches!(next.as_str(), "fn" | "unsafe" | "async" | "extern"))
        {
            continue;
        }
        if ident == "extern" && ident_at(tokens, index + 1).as_deref() != Some("crate") {
            continue;
        }
        if KEYWORDS.contains(&ident.as_str()) {
            return Some((index, ident));
        }
        if !matches!(
            ident.as_str(),
            "pub" | "crate" | "unsafe" | "async" | "default" | "auto"
        ) {
            // Macro invocation or other module-level expression item.
            return Some((index, "<item>".to_owned()));
        }
    }
    None
}

fn walk(root: &Path) -> (Inventory, BTreeSet<PathBuf>) {
    let mut walk = Walk {
        root,
        production: Inventory::new(),
        reached: BTreeSet::new(),
    };
    walk.file(&root.join("main.rs"), root.to_owned(), false);
    (walk.production, walk.reached)
}

fn all_sources(directory: &Path, out: &mut BTreeSet<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            all_sources(&path, out);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.insert(path);
        }
    }
}

// Each exception is keyed by enclosing function AND count: adding a second
// raw spawn beside an approved one still fails. Reasons describe ownership.
const ALLOWLIST: &[(&str, &str, &str, usize, &str)] = &[
    (
        "handlers/assistant_cma_agents.rs",
        "relay_events",
        "body_stream",
        1,
        "Subscription (b): dedicated remote observer, durable cursor; existing Reconnect action resumes without executing work.",
    ),
    (
        "handlers/assistant_direct.rs",
        "attach_in_flight_permit",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/assistant_invocations.rs",
        "create",
        "crate::turn_trace::spawn",
        1,
        "Trace helper preserves context then enters shutdown Runtime (completion lane for cleanup); physical implementation independently inventoried.",
    ),
    (
        "handlers/assistant_nyxagent.rs",
        "checking_notice",
        "crate::turn_trace::spawn",
        1,
        "Trace helper preserves context then enters shutdown Runtime (completion lane for cleanup); physical implementation independently inventoried.",
    ),
    (
        "handlers/assistant_nyxagent.rs",
        "run_turn",
        "super::assistant_titles::spawn",
        1,
        "Title helper delegates to the tracked title service, also inventoried.",
    ),
    (
        "handlers/assistant_nyxagent.rs",
        "sse_response",
        "sse",
        1,
        "Turn SSE is work (a); only live adds the recoverable-subscription marker (b), using ready/resync and polling.",
    ),
    (
        "handlers/assistant_platform_models.rs",
        "execute",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/assistant_team.rs",
        "spawn_sweeps",
        "crate::services::assistant_notification_service::spawn_worker",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "handlers/assistant_team.rs",
        "spawn_sweeps",
        "crate::services::nyxbot_gateway_release_service::spawn_worker",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "handlers/assistant_titles.rs",
        "spawn",
        "titles::spawn",
        1,
        "Title helper enters shutdown Runtime in assistant_title_service.rs; its physical spawn is independently inventoried.",
    ),
    (
        "handlers/call_adapters.rs",
        "completions",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/channel_connect_links.rs",
        "managed_complete",
        "sse",
        1,
        "Work (a): finite managed-provider setup progress and terminal result, not an idle subscription.",
    ),
    (
        "handlers/channel_managed.rs",
        "complete",
        "sse",
        1,
        "Work (a): finite managed-provider setup progress and terminal result, not an idle subscription.",
    ),
    (
        "handlers/llm_gateway.rs",
        "build_filtered_response",
        "body_stream",
        2,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/llm_gateway.rs",
        "build_filtered_response",
        "crate::turn_trace::spawn_cleanup",
        1,
        "Trace helper preserves context then enters shutdown Runtime (completion lane for cleanup); physical implementation independently inventoried.",
    ),
    (
        "handlers/llm_gateway.rs",
        "build_translated_sse_response",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/llm_gateway.rs",
        "build_translated_sse_response",
        "crate::turn_trace::spawn_cleanup",
        1,
        "Trace helper preserves context then enters shutdown Runtime (completion lane for cleanup); physical implementation independently inventoried.",
    ),
    (
        "handlers/machine_gateway.rs",
        "authorize_and_execute",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/mcp_transport.rs",
        "content_result_with_notifications_and_diagnostic",
        "sse",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/mcp_transport.rs",
        "mcp_get",
        "sse",
        2,
        "Subscription (b): durable notification outbox resumes Last-Event-ID; the stateless variant is already empty/finite.",
    ),
    (
        "handlers/node_ws.rs",
        "handle_node_connection",
        "tokio::spawn",
        1,
        "Writer is scoped to the tracked node connection, which explicitly aborts and awaits this handle before releasing its owner fence.",
    ),
    (
        "handlers/node_ws.rs",
        "ws_handler",
        "method::on_upgrade",
        1,
        "Node upgrade reserves shutdown.work before callback, then holds node_socket through reader/writer teardown; handover sends close1001.",
    ),
    (
        "handlers/nyxbot.rs",
        "relay_callback",
        "crate::turn_trace::spawn",
        1,
        "Trace helper preserves context then enters shutdown Runtime (completion lane for cleanup); physical implementation independently inventoried.",
    ),
    (
        "handlers/nyxbot.rs",
        "responses_with_producers_impl",
        "crate::turn_trace::spawn",
        1,
        "Trace helper preserves context then enters shutdown Runtime (completion lane for cleanup); physical implementation independently inventoried.",
    ),
    (
        "handlers/nyxbot.rs",
        "sse",
        "sse",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/nyxbot_native_responses.rs",
        "accept",
        "crate::turn_trace::spawn",
        1,
        "Trace helper preserves context then enters shutdown Runtime (completion lane for cleanup); physical implementation independently inventoried.",
    ),
    (
        "handlers/nyxbot_native_responses.rs",
        "stream",
        "sse",
        1,
        "Work (a): native replay owns an Observer and stream delivery barrier; finish before ownership handover.",
    ),
    (
        "handlers/platform_model_gateway.rs",
        "execute_with_runtime",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/proxy.rs",
        "execute_resolved_proxy_inner",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/proxy.rs",
        "gate_pool_response",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/service_pool_attempt.rs",
        "direct_response",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/service_pool_attempt.rs",
        "drop",
        "crate::turn_trace::spawn_cleanup",
        1,
        "Trace helper preserves context then enters shutdown Runtime (completion lane for cleanup); physical implementation independently inventoried.",
    ),
    (
        "handlers/service_pool_attempt.rs",
        "node_response",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/service_pool_attempt.rs",
        "response_body",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "handlers/service_pool_attempt.rs",
        "response_body",
        "crate::turn_trace::spawn",
        1,
        "Trace helper preserves context then enters shutdown Runtime (completion lane for cleanup); physical implementation independently inventoried.",
    ),
    (
        "main.rs",
        "server_run",
        "handlers::machine_update::spawn",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "main.rs",
        "server_run",
        "handlers::trigger_scheduler::spawn",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "main.rs",
        "server_run",
        "services::assistant_agent_learning::spawn",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "main.rs",
        "server_run",
        "services::assistant_upload_retention::spawn",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "main.rs",
        "server_run",
        "services::billing::account_reconciliation::spawn",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "main.rs",
        "server_run",
        "services::billing::exact_migration::spawn",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "main.rs",
        "server_run",
        "services::billing::usage_rollup::spawn_worker",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "main.rs",
        "server_run",
        "services::machine_access_service::spawn",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "main.rs",
        "server_run",
        "services::personal_erasure_service::spawn_worker",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "main.rs",
        "server_run",
        "services::remote_machine_manager::spawn",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "main.rs",
        "server_run",
        "services::telemetry_erasure_service::spawn_worker",
        1,
        "Supervisor helper; its implementation enters the shutdown runtime and is independently scanned here.",
    ),
    (
        "services/api_docs_service.rs",
        "parse_spec_body_with_permits",
        "tokio::task::spawn_blocking",
        1,
        "Awaited CPU/parsing/hash work under its request or worker; no independent durable ownership or external effect. Hard deadline bounds runtime teardown.",
    ),
    (
        "services/assistant_nyxagent_runtime.rs",
        "execute_nyxagent_turn",
        "super::assistant_title_service::spawn",
        1,
        "Title helper enters shutdown Runtime; its physical spawn is independently inventoried.",
    ),
    (
        "services/assistant_service_execution/receipts.rs",
        "finish_http",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "services/attachment_extraction.rs",
        "extract_deadline",
        "method::spawn",
        1,
        "Extraction subprocess owned/awaited by its request with kill_on_drop; no independent database claim or provider effect.",
    ),
    (
        "services/attachment_extraction.rs",
        "extract_deadline",
        "tokio::task::spawn_blocking",
        1,
        "Awaited CPU/parsing/hash work under its request or worker; no independent durable ownership or external effect. Hard deadline bounds runtime teardown.",
    ),
    (
        "services/attachment_extraction.rs",
        "worker",
        "tokio::task::spawn_blocking",
        1,
        "Awaited CPU/parsing/hash work under its request or worker; no independent durable ownership or external effect. Hard deadline bounds runtime teardown.",
    ),
    (
        "services/auth_service.rs",
        "change_password",
        "tokio::task::spawn_blocking",
        1,
        "Awaited CPU/parsing/hash work under its request or worker; no independent durable ownership or external effect. Hard deadline bounds runtime teardown.",
    ),
    (
        "services/auth_service.rs",
        "reset_password",
        "tokio::task::spawn_blocking",
        1,
        "Awaited CPU/parsing/hash work under its request or worker; no independent durable ownership or external effect. Hard deadline bounds runtime teardown.",
    ),
    (
        "services/chatgpt_translator.rs",
        "send_to_chatgpt_with_api_url",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "services/execution_permission_dispatch/machine.rs",
        "delivery",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "services/execution_permission_dispatch/proposals/v2/delivery.rs",
        "complete_response",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "services/generated_artifacts/acquisition.rs",
        "acquire",
        "tokio::task::spawn_blocking",
        1,
        "Awaited CPU/parsing/hash work under its request or worker; no independent durable ownership or external effect. Hard deadline bounds runtime teardown.",
    ),
    (
        "services/machine_gateway_service.rs",
        "stream_upload",
        "body_stream",
        1,
        "Work (a): outgoing upload body is owned by the tracked request, not an independent subscription.",
    ),
    (
        "services/mfa_service.rs",
        "hash_recovery_codes",
        "tokio::task::spawn_blocking",
        1,
        "Awaited CPU/parsing/hash work under its request or worker; no independent durable ownership or external effect. Hard deadline bounds runtime teardown.",
    ),
    (
        "services/mfa_service.rs",
        "matching_recovery_hash",
        "tokio::task::spawn_blocking",
        1,
        "Awaited CPU/parsing/hash work under its request or worker; no independent durable ownership or external effect. Hard deadline bounds runtime teardown.",
    ),
    (
        "services/node_dispatch.rs",
        "proxy_result_response",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "services/platform_model_execution/adapters.rs",
        "parse_with_budget",
        "tokio::task::spawn_blocking",
        1,
        "Awaited CPU/parsing/hash work under its request or worker; no independent durable ownership or external effect. Hard deadline bounds runtime teardown.",
    ),
    (
        "services/platform_personal_turn_service.rs",
        "start_personal_turn",
        "super::assistant_title_service::spawn",
        1,
        "Title helper enters shutdown Runtime; its physical spawn is independently inventoried.",
    ),
    (
        "services/platform_public_proxy_service.rs",
        "dispatch_nyxagent",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "services/provider_token_exchange_service.rs",
        "spawn_expiry_worker",
        "tokio::spawn",
        1,
        "Memory-only token-cache expiry loop; process loss discards the entire cache, no durable work or ownership.",
    ),
    (
        "services/proxy_http_response.rs",
        "observe_model_response",
        "body_stream",
        1,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "services/proxy_http_response.rs",
        "observe_model_response",
        "crate::turn_trace::spawn_cleanup",
        1,
        "Trace helper preserves context then enters shutdown Runtime (completion lane for cleanup); physical implementation independently inventoried.",
    ),
    (
        "services/proxy_http_response.rs",
        "relay_response",
        "crate::turn_trace::spawn_cleanup",
        1,
        "Trace helper preserves context then enters shutdown Runtime (completion lane for cleanup); physical implementation independently inventoried.",
    ),
    (
        "services/proxy_http_response.rs",
        "relay_response_inner",
        "body_stream",
        2,
        "Work (a): bounded result/transfer body; middleware retains work through EOF/Drop.",
    ),
    (
        "services/proxy_http_response.rs",
        "relay_response_inner",
        "crate::turn_trace::spawn_cleanup",
        2,
        "Trace helper preserves context then enters shutdown Runtime (completion lane for cleanup); physical implementation independently inventoried.",
    ),
    (
        "services/shutdown.rs",
        "spawn_joined_kind",
        "method::spawn",
        1,
        "Tracker primitive: Work registered before spawn; retained abort handle and Work Drop prove completion before ownership release.",
    ),
    (
        "services/shutdown.rs",
        "spawn_tracked",
        "tokio::spawn",
        1,
        "Tracker primitive: Work registered before spawn; retained abort handle and Work Drop prove completion before ownership release.",
    ),
    (
        "services/voice/grok_runtime.rs",
        "serve",
        "grok::Handle::spawn",
        1,
        "Actor helper enters shutdown Runtime in voice/grok/actor.rs; its physical spawn is independently inventoried.",
    ),
    (
        "shutdown_servers.rs",
        "resume",
        "method::spawn_worker",
        2,
        "self.runtime is the shutdown Runtime; Servers owns and joins both listener handles across graceful epochs.",
    ),
    (
        "telemetry/mod.rs",
        "from_config",
        "tokio::spawn",
        1,
        "Explicit best-effort analytics queue; no durable execution ownership. Existing bounded provider uncertainty/erasure rules remain unchanged.",
    ),
    (
        "turn_trace.rs",
        "init",
        "method::spawn",
        1,
        "Best-effort OS stdout writer with a bounded metadata queue; no execution or durable ownership, must not block process exit on stdout.",
    ),
];

#[test]
fn shutdown_spawn_inventory() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let (found, reached) = walk(&root);
    let mut sources = BTreeSet::new();
    all_sources(&root, &mut sources);
    assert_eq!(
        sources, reached,
        "unreachable source file: review module inventory"
    );
    let mut expected = BTreeMap::new();
    for &(file, function, primitive, count, reason) in ALLOWLIST {
        assert!(!reason.is_empty());
        assert!(
            expected
                .insert(
                    (file.to_owned(), function.to_owned(), primitive.to_owned()),
                    count
                )
                .is_none()
        );
    }
    let mut raw = BTreeMap::new();
    for (key, count) in &found {
        let reason = if key.2 == "tracked" {
            "shutdown runtime"
        } else {
            raw.insert(key.clone(), *count);
            ALLOWLIST
                .iter()
                .find(|e| e.0 == key.0 && e.1 == key.1 && e.2 == key.2)
                .map_or("UNREVIEWED", |e| e.4)
        };
        println!("{}::{} | {} | {} | {}", key.0, key.1, key.2, count, reason);
    }
    assert_eq!(
        raw, expected,
        "spawn inventory changed; route new work through shutdown or document its awaited lifetime"
    );
}

#[test]
fn shutdown_spawn_scanner_detects_aliases_joinsets_upgrades_and_skips_test_modules() {
    let root = Path::new("/fixture");
    let mut walk = Walk {
        root,
        production: Inventory::new(),
        reached: BTreeSet::new(),
    };
    let context = Context {
        file: "fixture.rs",
        file_dir: root,
        child_dir: root.to_owned(),
        top_level: true,
        test_only: false,
    };
    walk.level(TokenStream::from_str(r#"
        fn work() {
            tokio::spawn(async {}); task::spawn_local(async {}); task::spawn_blocking::<_, ()>(|| ());
            spawn_blocking(|| ()); set.spawn(async {});
            ws.on_upgrade(|socket| async {});
            state.shutdown.spawn(async {});
            crate::services::shutdown::process().spawn(async {});
            ws.on_upgrade(state.shutdown.clone().upgrade_callback(|socket| async {}));
        }
        #[cfg(test)] mod tests { fn ignored() { tokio::spawn(async {}); } }
        #[cfg(test)] fn ignored() { tokio::spawn(async {}); }
    "#).unwrap(), &context);
    let counts: BTreeMap<_, _> = walk
        .production
        .into_iter()
        .map(|((_, name, kind), count)| {
            assert_eq!(name, "work");
            (kind, count)
        })
        .collect();
    assert_eq!(
        counts,
        [
            ("tokio::spawn", 1),
            ("method::spawn", 1),
            ("task::spawn_local", 1),
            ("spawn_blocking", 1),
            ("task::spawn_blocking", 1),
            ("method::on_upgrade", 1),
            ("tracked", 3)
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect::<BTreeMap<_, usize>>()
    );
}

#[test]
#[should_panic(expected = "renamed spawn imports/casts")]
fn shutdown_spawn_inventory_rejects_an_alias_that_would_hide_new_calls() {
    scan(
        TokenStream::from_str("use tokio::spawn as hidden; hidden(async {});").unwrap(),
        "fixture.rs",
        "helper",
        &mut Inventory::new(),
    );
}
