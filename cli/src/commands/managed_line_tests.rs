use super::*;
use crate::cli::Cli;
use crate::test_support::mock_auth;
use clap::Parser;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path},
};

const LINE: &str = "0b8f4b44-3d73-4d0c-8f59-5d1c4f7b2a61";
const CLAIM: &str = "8f8a6e93-1f0c-4f3e-9d77-2a8b9b0b6c3a";
const QUOTE: &str = "a7c0f5d2-5b8e-4d3a-8c5f-0e1d2c3b4a59";
const REQUEST: &str = "3f0b8a3e-6c55-4b1e-9a61-1c2f0e9a7d10";
const AGENT: &str = "d1e2f3a4-b5c6-4d7e-8f90-a1b2c3d4e5f6";
const BINDING: &str = "c3d2e1f0-9a8b-4c7d-8e6f-5a4b3c2d1e0f";

fn receipt(kind: &str) -> Value {
    json!({"request_id":REQUEST,"kind":kind,"line_id":LINE,"claim_id":CLAIM,"period_id":null})
}
async fn server_with_line(auto_renew: bool) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/managed-lines"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "lines": [{
                "id": LINE, "phone_number": "+6560000000", "country": "SG", "state": "claimed",
                "voice": true, "sms": true, "standard_voice_options": [],
                "claim": {
                    "id": CLAIM, "generation": 3, "state": "claimed", "suspension": null,
                    "auto_renew": auto_renew, "standard_voice": null,
                    "covered_from": "2026-11-01T00:00:00+00:00",
                    "covered_until": "2026-12-01T00:00:00+00:00",
                    "period_index": 4,
                    "renewal_quote": {"id": QUOTE},
                    "voice_binding_id": BINDING, "voice_binding_revision": 9
                }
            }],
            "next_after_id": null
        })))
        .mount(&server)
        .await;
    server
}
fn consent(eua: Option<&str>) -> Consent {
    Consent {
        terms: "2026-11".into(),
        eua: eua.map(Into::into),
    }
}

#[tokio::test]
async fn claim_sends_explicit_consent_and_the_supplied_request_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/managed-lines/claims"))
        .and(body_json(json!({
            "request_id": REQUEST, "quote_id": QUOTE, "line_id": LINE, "agent_id": AGENT,
            "voice_tier": "premium",
            "standard_voice": {"voice": "ElevenLabs.flash.abc", "language": "zh-CN"},
            "enable_sms": true, "caller_confirmations": true, "auto_renew": false,
            "accepted_terms_versions": {"managed_line": "2026-11", "elevenlabs_eua": "eua-1"}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(receipt("claim")))
        .expect(1)
        .mount(&server)
        .await;
    run(LineCommands::Claim(ClaimArgs {
        line: LINE.into(),
        quote: QUOTE.into(),
        agent: AGENT.into(),
        voice_tier: VoiceTier::Premium,
        voice: Some("ElevenLabs.flash.abc".into()),
        language: Some("zh-CN".into()),
        sms: true,
        caller_confirmations: true,
        auto_renew: false,
        consent: consent(Some("eua-1")),
        request_id: Some(REQUEST.into()),
        auth: mock_auth(server.uri()),
    }))
    .await
    .unwrap();
}

#[tokio::test]
async fn release_requires_confirmation_and_resolves_the_current_claim() {
    let server = server_with_line(true).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/managed-lines/releases"))
        .and(body_json(
            json!({"request_id": REQUEST, "line_id": LINE, "claim_id": CLAIM}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(receipt("release")))
        .expect(1)
        .mount(&server)
        .await;
    let command = |yes| LineCommands::Release {
        line: LINE.into(),
        yes,
        request_id: Some(REQUEST.into()),
        auth: mock_auth(server.uri()),
    };
    assert!(
        run(command(false))
            .await
            .unwrap_err()
            .to_string()
            .contains("--yes")
    );
    run(command(true)).await.unwrap();
}

#[tokio::test]
async fn renewal_accept_uses_the_pending_quote_and_keeps_auto_renew() {
    let server = server_with_line(true).await;
    Mock::given(method("POST"))
        .and(path("/api/v1/managed-lines/coverage"))
        .and(body_json(json!({
            "request_id": REQUEST, "quote_id": QUOTE, "auto_renew": true,
            "accepted_terms_versions": {"managed_line": "2026-11", "elevenlabs_eua": null}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(receipt("renewal")))
        .expect(1)
        .mount(&server)
        .await;
    run(LineCommands::Renewal {
        command: RenewalCommands::Accept(CoverageArgs {
            line: LINE.into(),
            quote: None,
            auto_renew: false,
            no_auto_renew: false,
            consent: consent(None),
            request_id: Some(REQUEST.into()),
            auth: mock_auth(server.uri()),
        }),
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn renewal_quotes_name_the_next_period_and_owner_routes_use_binding_revision() {
    let server = server_with_line(false).await;
    Mock::given(method("POST"))
        .and(path(format!("/api/v1/managed-lines/{LINE}/quotes")))
        .and(body_json(json!({"kind": {"kind": "renewal", "claim_id": CLAIM, "generation": 3, "period_index": 5}})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": QUOTE, "rental": {"credits_per_unit": "0.5"}, "usage": [], "period_days": 30,
            "terms": {"managed_line": "2026-11", "elevenlabs_eua": null}, "accept_before": "2026-11-20T00:00:00Z",
            "paid_period_refundable": false
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!(
            "/api/v1/call-adapters/bindings/{BINDING}/caller-confirmations"
        )))
        .and(body_json(
            json!({"enabled": true, "sms_binding_id": null, "voice_revision": 9}),
        ))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("PUT"))
        .and(path(format!("/api/v1/managed-lines/{LINE}/standard-voice")))
        .and(body_json(json!({"claim_id": CLAIM, "binding_revision": 9,
            "option": {"voice": "Telnyx.Ultra.Callie", "language": "zh-TW"}})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    run(LineCommands::Quote {
        line: LINE.into(),
        renewal: true,
        resume: false,
        auth: mock_auth(server.uri()),
    })
    .await
    .unwrap();
    run(LineCommands::Confirmations {
        command: ConfirmationCommands::Enable(LineArgs {
            line: LINE.into(),
            auth: mock_auth(server.uri()),
        }),
    })
    .await
    .unwrap();
    run(LineCommands::Voice {
        command: VoiceCommands::Set {
            line: LINE.into(),
            voice: "Telnyx.Ultra.Callie".into(),
            language: "zh-TW".into(),
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn terms_acceptance_binds_the_current_eligibility_revision() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/managed-lines/eligibility"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"revision": 6})))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/managed-lines/eligibility/terms"))
        .and(body_json(json!({"revision": 6,
            "terms": {"managed_line": "2026-11", "elevenlabs_eua": "eua-1"}})))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    run(LineCommands::Terms {
        command: TermsCommands::Accept {
            managed_line: "2026-11".into(),
            eua: Some("eua-1".into()),
            auth: mock_auth(server.uri()),
        },
    })
    .await
    .unwrap();
}

#[test]
fn claim_flags_require_consent_and_sms_for_confirmations() {
    let base = [
        "nyxid", "nyxbot", "line", "claim", LINE, "--quote", QUOTE, "--agent", AGENT,
    ];
    assert!(
        Cli::try_parse_from(base).is_err(),
        "terms consent is required"
    );
    let mut accepted = base.to_vec();
    accepted.extend(["--accept-terms", "2026-11"]);
    assert!(Cli::try_parse_from(&accepted).is_ok());
    let mut confirmations = accepted.clone();
    confirmations.push("--caller-confirmations");
    assert!(Cli::try_parse_from(&confirmations).is_err());
    confirmations.push("--sms");
    assert!(Cli::try_parse_from(&confirmations).is_ok());
    let mut bad = accepted.clone();
    bad.extend(["--request-id", "not-a-uuid"]);
    assert!(Cli::try_parse_from(&bad).is_err());
}
