//! Deliberate inventory of every production JWT verification entry point
//! (#1959). The scanner walks the compiled module tree from `main.rs`, skips
//! `#[cfg(test)]` items, and records each call to a NyxID JWT verifier
//! (`verify_token`, `validate_relay_*_token`), each `JwtKeys` key selection
//! (`decoding_key`, `.decoding`) and each `jsonwebtoken::decode`, keyed by file
//! and enclosing function. A new or removed entry point fails this test until
//! the allowlist below records its token class and the live authority checks
//! it applies, after review against the shared authority service.
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    str::FromStr,
};

use proc_macro2::{Delimiter, TokenStream, TokenTree};

type Inventory = BTreeMap<(String, String, &'static str), usize>;

const WRAPPERS: [&str; 3] = [
    "verify_token",
    "validate_relay_reply_token",
    "validate_relay_callback_token",
];

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

fn path_sep_before(toks: &[Tok], index: usize) -> bool {
    index >= 2 && punct(toks.get(index - 1), ':') && punct(toks.get(index - 2), ':')
}

/// Record verifier uses inside one function body (or module-level item).
fn scan(
    stream: TokenStream,
    file: &str,
    function: &str,
    imports_decode: bool,
    found: &mut Inventory,
) {
    let mut toks = Vec::new();
    flatten(stream, &mut toks);
    for index in 0..toks.len() {
        let Tok::Ident(name) = &toks[index] else {
            continue;
        };
        let prev = index.checked_sub(1).and_then(|prev| toks.get(prev));
        let next = toks.get(index + 1);
        if ident(prev, "fn") {
            continue;
        }
        let call = matches!(next, Some(Tok::Open(Delimiter::Parenthesis)));
        let turbofish = punct(next, ':');
        let primitive = if let Some(wrapper) = WRAPPERS.iter().find(|w| *w == name) {
            (call && !punct(prev, '.')).then_some(*wrapper)
        } else if name == "decoding_key" {
            (call && (punct(prev, '.') || path_sep_before(&toks, index))).then_some("decoding_key")
        } else if name == "decoding" {
            (punct(prev, '.') && !call).then_some(".decoding")
        } else if name == "decode" && (call || turbofish) {
            let qualified = path_sep_before(&toks, index);
            let via_crate = qualified && ident(toks.get(index.wrapping_sub(3)), "jsonwebtoken");
            let bare = !qualified && !punct(prev, '.') && imports_decode;
            (via_crate || bare).then_some("jsonwebtoken::decode")
        } else {
            None
        };
        if let Some(primitive) = primitive {
            *found
                .entry((file.to_owned(), function.to_owned(), primitive))
                .or_default() += 1;
        }
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
    imports_decode: bool,
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
        let imports_decode = regex::Regex::new(r"use\s+jsonwebtoken\s*::[^;]*\bdecode\b")
            .unwrap()
            .is_match(&source);
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
            imports_decode,
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
                        context.imports_decode,
                        &mut self.production,
                    );
                }
                _ => scan(
                    tokens.iter().cloned().collect(),
                    context.file,
                    "<module>",
                    context.imports_decode,
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

/// One reviewed verification entry point.
struct Verifier {
    file: &'static str,
    function: &'static str,
    primitive: &'static str,
    count: usize,
    /// Which token class the call verifies.
    token_class: &'static str,
    /// The live authority checks applied after signature verification.
    authority: &'static str,
}

const fn v(
    file: &'static str,
    function: &'static str,
    primitive: &'static str,
    count: usize,
    token_class: &'static str,
    authority: &'static str,
) -> Verifier {
    Verifier {
        file,
        function,
        primitive,
        count,
        token_class,
        authority,
    }
}

const ALLOWLIST: &[Verifier] = &[
    // Primitives: every NyxID-issued JWT is verified through these.
    v(
        "crypto/jwt.rs",
        "decoding_key",
        ".decoding",
        1,
        "active NyxID RS256 verification key",
        "key selection only: RS256 required; a missing or unknown kid is rejected; \
         retained overlap keys match their exact kid, with no probing or fallback",
    ),
    v(
        "crypto/jwt.rs",
        "verify_token",
        "decoding_key",
        1,
        "NyxID access/refresh JWTs (user, OAuth, delegated, relay, service account)",
        "shared verifier: kid-selected key, signature, iss, aud = BASE_URL, exp; \
         rejects the retired assistant_forward claim; live authority belongs to callers",
    ),
    v(
        "crypto/jwt.rs",
        "verify_token",
        "jsonwebtoken::decode",
        1,
        "NyxID access/refresh JWTs (user, OAuth, delegated, relay, service account)",
        "see verify_token decoding_key entry",
    ),
    v(
        "crypto/jwt.rs",
        "validate_relay_reply_token",
        "decoding_key",
        1,
        "relay reply token (aud channel-relay/reply)",
        "kid-selected key and signature; manual exp/iat skew, TTL bound, token_type, \
         aud and iss; single-use and live key checks belong to callers",
    ),
    v(
        "crypto/jwt.rs",
        "validate_relay_reply_token",
        "jsonwebtoken::decode",
        1,
        "relay reply token (aud channel-relay/reply)",
        "see validate_relay_reply_token decoding_key entry",
    ),
    v(
        "crypto/jwt.rs",
        "validate_relay_callback_token",
        "decoding_key",
        1,
        "relay callback token",
        "kid-selected key and signature; manual exp/iat skew, TTL bound, token_type, \
         aud and iss; route binding belongs to the receiver",
    ),
    v(
        "crypto/jwt.rs",
        "validate_relay_callback_token",
        "jsonwebtoken::decode",
        1,
        "relay callback token",
        "see validate_relay_callback_token decoding_key entry",
    ),
    // REST extractor (also userinfo, proxy and relay access tokens).
    v(
        "mw/auth.rs",
        "from_request_parts",
        "verify_token",
        1,
        "Bearer/DPoP access tokens on every AuthUser route, including /oauth/userinfo, \
         /proxy and relay (X-NyxID-User-Token) access tokens",
        "delegated claim consistency; token_type access; DPoP/mTLS cnf binding; service \
         account active + token record; oauth_token_authority_service::validate_claims \
         (consent revision, delegation row, source session; outage -> 503); active user; \
         catalog delegation live grant; relay agent key active; live sid session",
    ),
    // MCP admission.
    v(
        "handlers/mcp_transport.rs",
        "authenticate_mcp_or_public",
        "verify_token",
        1,
        "Bearer access tokens at /mcp admission",
        "token_type access; oauth_token_authority_service::validate_claims (outage -> \
         503); DPoP/mTLS cnf binding; relay agent key active; catalog delegation live \
         grant; active service account/user; org agent key; session fallback only for \
         session_fallback_allowed sessions",
    ),
    // OAuth endpoints.
    v(
        "handlers/oauth.rs",
        "introspect",
        "verify_token",
        1,
        "access/refresh JWTs at /oauth/introspect",
        "token_type access|refresh; oauth_token_authority_service::validate_claims \
         (outage -> 503); refresh row live (not revoked, owner, unexpired) with protected \
         consent revalidated; service-account token record; active user",
    ),
    v(
        "handlers/oauth.rs",
        "revoke",
        "verify_token",
        1,
        "access/refresh JWTs at /oauth/revoke",
        "signature only to read the jti; marks the stored token revoked and revokes the \
         client's catalog delegations; never grants access (RFC 7009 always 200)",
    ),
    v(
        "handlers/oauth.rs",
        "verify_consent_request_with_expiry",
        "decoding_key",
        1,
        "signed consent-request assertion (authorize -> consent round trip)",
        "kid-selected key, iss and consent audience; exp unless explicitly re-displaying \
         (bounded to 24h, expired -> grant_changed); token_type and sub = acting user; \
         the authorize request is then re-validated by the consent handler",
    ),
    v(
        "handlers/oauth.rs",
        "verify_consent_request_with_expiry",
        "jsonwebtoken::decode",
        1,
        "signed consent-request assertion (authorize -> consent round trip)",
        "see verify_consent_request_with_expiry decoding_key entry",
    ),
    v(
        "services/resource_request_proof.rs",
        "introspect",
        "decoding_key",
        1,
        "resource-request proof JWT introspected by a confidential catalog client",
        "feature flag, active confidential client listed in introspection_client_ids; \
         RS256 + typ, aud = service id, iss, exact 60s lifetime, request descriptor \
         binding; nyx_authority.require_current live check; RBAC resolved live",
    ),
    v(
        "services/resource_request_proof.rs",
        "introspect",
        "jsonwebtoken::decode",
        1,
        "resource-request proof JWT introspected by a confidential catalog client",
        "see resource_request_proof introspect decoding_key entry",
    ),
    // Refresh and token exchange.
    v(
        "services/token_service.rs",
        "refresh_tokens",
        "verify_token",
        1,
        "refresh JWT at /auth/refresh and /oauth/token refresh_token grant",
        "token_type refresh; stored row by jti (revoked/expired, reuse detection); live \
         session; active OAuth client (deactivated -> revoke); live consent grant and \
         protected consent reference; service allowlist narrowed to current consent",
    ),
    v(
        "handlers/auth.rs",
        "refresh",
        "verify_token",
        1,
        "access token just minted by token_service::refresh_tokens",
        "telemetry subject only, no authorization decision; refresh authority is \
         token_service::refresh_tokens",
    ),
    v(
        "services/token_exchange_service.rs",
        "exchange_token_with_authority",
        "verify_token",
        2,
        "RFC 8693 subject access token, and the delegated token minted from it",
        "subject: token_type access, exchange_bindings captures live consent/session \
         authority, consent check, delegation scope and catalog delegation authority; \
         minted token is re-read only to persist its delegation authority row",
    ),
    v(
        "services/token_exchange_service.rs",
        "refresh_delegation_token",
        "verify_token",
        1,
        "delegated access token minted by /delegation/refresh",
        "re-read only to persist the renewed delegation authority row; the source was \
         authorized by load_delegation, active user/clients, live consent and catalog \
         authority before minting",
    ),
    v(
        "services/oauth_broker_service.rs",
        "verify_binding_refresh_jwt",
        "verify_token",
        1,
        "broker-binding refresh JWT (urn:nyxid:params:oauth:token-type:binding-id flow)",
        "token_type refresh and jti equal to the binding's refresh_token_jti; callers \
         load the live binding and reject a revoked stored refresh token",
    ),
    // Relay and callback receivers.
    v(
        "handlers/channel_relay.rs",
        "resolve_reply_token_message_context",
        "validate_relay_reply_token",
        1,
        "relay reply token at POST /channel-relay/reply",
        "active inbound message, conversation, live agent API key and active bot; bot \
         scope; JTI consumed once in reply_token_uses",
    ),
    v(
        "handlers/channel_relay.rs",
        "resolve_reply_token_edit_context",
        "validate_relay_reply_token",
        1,
        "relay reply token at /channel-relay/reply/update",
        "JTI must already be recorded in reply_token_uses (used to send first); outbound \
         lookup scoped to the token's key, conversation and inbound anchor",
    ),
    v(
        "handlers/nyxbot.rs",
        "relay_callback",
        "validate_relay_callback_token",
        1,
        "relay callback JWT on NyxBot's direct channel relay endpoint",
        "active direct NyxbotChannel row; body SHA-256, route API key, message id, \
         platform and correlation id (jti) bound to the payload; personal route authority",
    ),
    // Public platform contract assertions.
    v(
        "services/platform_public_contract_service.rs",
        "verify_assertion",
        "decoding_key",
        1,
        "NyxID public-authority assertion (typ nyxid-public+jwt)",
        "kid-selected key, iss, audience, zero leeway; typ, scope, token_type, request \
         body digest, iat not in future, lifetime <= 300s; validate_authority for scope",
    ),
    v(
        "services/platform_public_contract_service.rs",
        "verify_assertion",
        "jsonwebtoken::decode",
        1,
        "NyxID public-authority assertion (typ nyxid-public+jwt)",
        "see verify_assertion decoding_key entry",
    ),
    // Non-NyxID keys (recorded so a new JWT verifier anywhere is reviewed).
    v(
        "crypto/dpop.rs",
        "validate_proof_claims",
        "jsonwebtoken::decode",
        1,
        "DPoP proof (RFC 9449) signed by the client's ES256 key from the proof header",
        "typ/alg pinned; htm, htu and iat window checked; validate_proof claims the jti \
         in dpop_jti_cache (replay) and callers bind the thumbprint to cnf.jkt",
    ),
    v(
        "crypto/jwks.rs",
        "verify_with_keys_generic",
        "jsonwebtoken::decode",
        1,
        "third-party Google/Apple ID tokens (native social login exchange)",
        "provider JWKS key by kid, issuer and audience pinned; NyxID authority is only \
         issued after the social exchange resolves an active user",
    ),
    v(
        "services/remote_machine_bootstrap_verifier.rs",
        "verify",
        "jsonwebtoken::decode",
        1,
        "remote machine provider bootstrap ticket (configured provider key)",
        "issuer, audience, required claims, zero leeway; purpose, exact binding, \
         lifetime <= 900s and recipient key digest",
    ),
];

#[test]
fn every_jwt_verification_entry_point_is_reviewed() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let (found, reached) = walk(&root);

    let mut sources = BTreeSet::new();
    all_sources(&root, &mut sources);
    let unreached: Vec<_> = sources.difference(&reached).collect();
    assert!(
        unreached.is_empty(),
        "source files outside the module tree (include!/orphan?) are not inventoried: {unreached:#?}"
    );

    let mut expected = Inventory::new();
    for entry in ALLOWLIST {
        assert!(
            !entry.token_class.trim().is_empty() && !entry.authority.trim().is_empty(),
            "allowlist entry {}::{} must record its token class and live authority checks",
            entry.file,
            entry.function
        );
        let previous = expected.insert(
            (entry.file.into(), entry.function.into(), entry.primitive),
            entry.count,
        );
        assert!(
            previous.is_none(),
            "duplicate allowlist entry {}::{} ({})",
            entry.file,
            entry.function,
            entry.primitive
        );
    }
    let unlisted: Vec<_> = found
        .iter()
        .filter(|(key, count)| expected.get(*key) != Some(count))
        .collect();
    let stale: Vec<_> = expected
        .iter()
        .filter(|(key, count)| found.get(*key) != Some(count))
        .collect();
    assert!(
        unlisted.is_empty() && stale.is_empty(),
        "JWT verification inventory changed. Review each new verifier against the shared \
         live authority checks (revocation, consent revision, key selection by kid, outage \
         503 behaviour) and record its token class in ALLOWLIST.\n\
         new or changed: {unlisted:#?}\nstale: {stale:#?}"
    );
}

fn scan_fixture(source: &str) -> Inventory {
    let root = Path::new("/fixture");
    let mut walk = Walk {
        root,
        production: Inventory::new(),
        reached: BTreeSet::new(),
    };
    let context = Context {
        file: "new.rs",
        file_dir: root,
        child_dir: root.to_owned(),
        top_level: true,
        test_only: false,
        imports_decode: true,
    };
    walk.level(TokenStream::from_str(source).unwrap(), &context);
    walk.production
}

#[test]
fn inventory_detects_new_qualified_multiline_and_impl_paths_but_not_tests() {
    let found = scan_fixture(
        r#"
        use jsonwebtoken::{decode, Validation};
        pub(crate) const fn not_a_call() {}
        fn qualified() {
            let _ = crate::crypto::jwt::verify_token(&keys, &config, token);
            let _ = jwt::validate_relay_reply_token
                (&keys, &config, token);
        }
        impl Receiver {
            async fn handle(&self) -> impl Iterator<Item = u8> {
                let key = state.jwt_keys
                    .decoding_key(token)?;
                let _ = decode::<Claims>(token, key, &validation);
                let _ = jsonwebtoken::decode::<Claims>(token, &keys.decoding, &validation);
                let _ = validate_relay_callback_token(&keys, &config, token);
            }
        }
        fn not_verifiers(bot: Bot) {
            let _ = bot.verify_token;
            let _ = hex::decode(input);
            let _ = "verify_token(";
        }
        #[cfg(test)]
        mod tests {
            fn fixture() { verify_token(&keys, &config, token); }
        }
        #[cfg(test)]
        fn helper() { verify_token(&keys, &config, token); }
    "#,
    );
    let key = |function: &str, primitive| ("new.rs".to_owned(), function.to_owned(), primitive);
    let expected: Inventory = [
        (key("qualified", "verify_token"), 1),
        (key("qualified", "validate_relay_reply_token"), 1),
        (key("handle", "decoding_key"), 1),
        (key("handle", "jsonwebtoken::decode"), 2),
        (key("handle", ".decoding"), 1),
        (key("handle", "validate_relay_callback_token"), 1),
    ]
    .into_iter()
    .collect();
    assert_eq!(found, expected);
}
