use super::*;
use crate::services::{assistant_authority_tests::connected, machine_gateway_service::Ingress};
use nyxid_machine::binary::{Frame, Kind};

async fn signed_gateway(f: &Fixture, job: &MachineJob) -> (u16, Vec<u8>) {
    let stored = f
        .state
        .db
        .collection::<Document>(machine_job::COLLECTION_NAME)
        .find_one(doc! {"_id":&job.id})
        .await
        .unwrap()
        .unwrap();
    let authority = stored.get_document("external_authority").unwrap();
    let id = Uuid::new_v4();
    let mut request = nyxid_machine::Request {
        version: 1,
        authority: None,
        external_authority: None,
        request_id: id.to_string(),
        node_id: f.node.id.clone(),
        operation: nyxid_machine::Operation::ServiceCall,
        parameters: json!({"job_id":job.id,"runtime_id":job.runtime_id,
            "method":"POST","path":"/s/external-policy/upload",
            "headers":[["content-type","application/octet-stream"]],
            "authority_binding":{"kind":"external","context_id":authority.get_str("context_id").unwrap(),
                "grant_id":authority.get_str("grant_id").unwrap(),"grant_revision":authority.get_i64("grant_revision").unwrap(),
                "assignment_revision":authority.get_i64("assignment_revision").unwrap(),"lease_id":authority.get_str("lease_id").unwrap()}}),
        timestamp: Utc::now().timestamp(),
        nonce: Uuid::new_v4().to_string(),
        signature: String::new(),
    };
    let secret =
        node_service::get_node_signing_secret(&f.state.db, &f.state.encryption_keys, &f.node.id)
            .await
            .unwrap();
    request.signature = nyxid_machine::signing::sign(&request, &secret);
    let (sender, mut replies) = mpsc::channel(16);
    let session = crate::handlers::machine_gateway::Session::new(sender);
    session.start(f.state.clone(), &f.node.id, request).await;
    session
        .receive(Frame {
            kind: Kind::GatewayUpload,
            end: true,
            id,
            sequence: 0,
            bytes: b"opaque external upload",
        })
        .await;
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        let mut status = None;
        let mut body = Vec::new();
        loop {
            match replies.recv().await.expect("gateway response frame") {
                NodeOutboundMessage::Text(text) => {
                    let value: Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(value["type"], "machine_service_response");
                    status = value["status"].as_u64().map(|s| s as u16);
                }
                NodeOutboundMessage::Binary(bytes) => {
                    let frame = Frame::decode(&bytes).unwrap();
                    assert_eq!(frame.kind, Kind::GatewayDownload);
                    assert_eq!(frame.id, id);
                    body.extend_from_slice(frame.bytes);
                    if frame.end {
                        return (status.expect("gateway status"), body);
                    }
                }
                NodeOutboundMessage::Close { .. } => panic!("gateway transport closed"),
            }
        }
    })
    .await
    .expect("bounded signed external gateway")
}

#[tokio::test]
async fn actual_signed_external_gateway_preserves_personal_and_org_streaming_and_rejects_revocation()
 {
    for org_owned in [false, true] {
        let f = fixture("external_gateway_nyxbot_policy_compat").await;
        let owner = if org_owned {
            let org = Uuid::new_v4().to_string();
            f.state
                .db
                .collection("users")
                .insert_one(test_user(&org, UserType::Org))
                .await
                .unwrap();
            f.state
                .db
                .collection("org_memberships")
                .insert_one(crate::test_utils::test_membership(
                    &org,
                    &f.owner,
                    crate::models::org_membership::OrgRole::Admin,
                    None,
                ))
                .await
                .unwrap();
            org
        } else {
            f.owner.clone()
        };
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_bytes(b"external response".to_vec()),
            )
            .mount(&server)
            .await;
        let service = connected(&f.state.db, &owner, "external-policy", &server.uri()).await;
        // Personal and org-owned jobs each opt in for this exact service only.
        crate::services::approval_service::set_service_approval_config(
            &f.state.db,
            &owner,
            &service,
            "External policy fixture",
            Some(false),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            crate::services::approval_service::user_requires_approval(
                &f.state.db,
                &f.owner,
                &crate::models::service_approval_config::ApprovalVerb::Write
            )
            .await
            .unwrap()
        );
        let grant = grants::edit(
            &f.state.db,
            &f.owner,
            &f.grant_id,
            1,
            &GrantSelection {
                separation: Some(nyxid_machine::context::Choice::SharedLegacy),
                legacy_warning_acknowledged: true,
                assignments: BTreeMap::from([(
                    f.node.id.clone(),
                    GrantAssignment {
                        capabilities: Capabilities {
                            shell: true,
                            files: true,
                            ..Default::default()
                        },
                        file_roots: vec!["/workspace".into()],
                        service_ids: vec![service.clone()],
                    },
                )]),
                expires_at: None,
            },
        )
        .await
        .unwrap();
        let source = source(&f, &f.secret).await;
        let (peer, mut requests) = peer(&f).await;
        let issued = Box::pin(dispatch_authorized(&f.state, &source, "nyx__machine_exec", &mut json!({
            "machine":f.node.id,"command":"consume declared service","services":[service],"background":true,
        }))).await.unwrap();
        requests.recv().await.unwrap();
        let job = job(&f, issued["job_id"].as_str().unwrap()).await;
        let auth = gateway_auth(&f.state.db, &job).await.unwrap();
        assert!(auth.assistant_chat.is_none());
        assert!(job.conversation_id.is_empty());
        assert_eq!(
            signed_gateway(&f, &job).await,
            (200, b"external response".to_vec())
        );
        let calls = server.received_requests().await.unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].body, b"opaque external upload");
        let ingress = Ingress {
            declared_id: service.clone(),
            git: None,
            job: Some((&job).into()),
        };
        assert_eq!(ingress.job_id(), Some(job.id.as_str()));
        assert!(ingress.assistant_job_id().is_none());
        let target = Box::pin(
            crate::services::proxy_service::resolve_proxy_target_by_user_service_id(
                &f.state.db,
                &f.state.encryption_keys,
                &f.owner,
                &service,
                None,
                None,
                crate::services::proxy_service::ProxyExecutionContext::new(
                    None,
                    crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
                ),
            ),
        )
        .await
        .unwrap()
        .unwrap()
        .target;
        let held = crate::services::assistant_service_execution::HttpEgressGuard::for_proxy(
            None,
            Some(&ingress),
            &f.state.db,
            &f.state.encryption_keys,
            &target,
            "POST",
            "/upload",
        )
        .expect("external job has a final egress guard");
        held.revalidate().await.unwrap();
        grants::revoke(&f.state.db, &f.owner, &grant.id, grant.revision)
            .await
            .unwrap();
        assert!(
            held.revalidate().await.is_err(),
            "held guard must reload external grant/source authority"
        );
        let (status, _) = signed_gateway(&f, &job).await;
        assert_ne!(status, 200);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert_eq!(
            f.state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        peer.abort();
        f.state.db.drop().await.unwrap();
    }
}
