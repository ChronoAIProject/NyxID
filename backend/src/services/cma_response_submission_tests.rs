use super::*;
use futures::{StreamExt, stream};

#[test]
fn receipt_identity_accepts_the_relative_path_built_by_mcp() {
    for path in [
        "api/v2/agents/agt_one/responses",
        "/api/v2/agents/agt_one/responses",
    ] {
        assert_eq!(agent_from_path(path).as_deref(), Some("agt_one"));
    }
    for path in [
        "//api/v2/agents/agt_one/responses",
        "api/v2/agents/agt_one/responses/events",
        "api/v2/agents/agt_one/extra/responses",
        "api/v2/agents/agt_%2Ftwo/responses",
        "api/v2/agents/wrong/responses",
    ] {
        assert!(agent_from_path(path).is_none());
    }
}

fn receipt(agent: &str, id: &str) -> String {
    format!(
        "event: response.created\ndata: {{\"type\":\"response.created\",\"response\":{{\"id\":\"{id}\",\"object\":\"response\",\"conversation\":{{\"id\":\"{agent}\"}},\"status\":\"queued\"}}}}\n\n"
    )
}

#[tokio::test]
async fn receipt_returns_while_task_stream_stays_open() {
    let source = stream::iter([Ok::<_, ()>(receipt("agt_one", "resp_one").into_bytes())])
        .chain(stream::pending());
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        read_receipt(source, "agt_one", 65536),
    )
    .await
    .expect("must not wait for task completion")
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(value["id"], "resp_one");
    assert_eq!(value["status"], "queued");
}

#[tokio::test]
async fn receipt_handles_chunk_boundaries_crlf_comments_and_multiline_data() {
    let source = format!(
        ": keepalive\r\n\r\n{}",
        receipt("agt_one", "resp_one")
            .replace(",\"response\"", ",\ndata: \"response\"")
            .replace('\n', "\r\n")
    );
    for width in [1, 2, 3, 17, 4096] {
        let chunks = source
            .as_bytes()
            .chunks(width)
            .map(|bytes| Ok::<_, ()>(bytes.to_vec()));
        let result = read_receipt(stream::iter(chunks), "agt_one", 65536)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&result).unwrap()["id"],
            "resp_one"
        );
    }
}

#[tokio::test]
async fn receipt_rejects_wrong_identity_truncation_overflow_and_observation_error() {
    for body in [
        receipt("agt_other", "resp_one"),
        receipt("agt_one", "not_response"),
        receipt("agt_one", "resp_one").trim_end().into(),
        "data: {\"type\":\"error\"}\n\n".into(),
        "data: broken\n\n".into(),
    ] {
        assert!(
            read_receipt(
                stream::iter([Ok::<_, ()>(body.into_bytes())]),
                "agt_one",
                65536
            )
            .await
            .is_err()
        );
    }
    assert!(
        read_receipt(
            stream::iter([Ok::<_, ()>(vec![b'x'; 65537])]),
            "agt_one",
            65536
        )
        .await
        .is_err()
    );
    assert!(
        read_receipt(stream::iter([Err::<Vec<u8>, _>(())]), "agt_one", 65536)
            .await
            .is_err()
    );
}

#[tokio::test(start_paused = true)]
async fn receipt_observation_is_time_bounded() {
    assert!(
        read_receipt(stream::pending::<Result<Vec<u8>, ()>>(), "agt_one", 65536)
            .await
            .is_err()
    );
}
