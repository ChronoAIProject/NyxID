use super::*;

#[cfg(test)]
tokio::task_local! {
    pub(super) static FAIL_PASSIVE_PERSISTENCE: bool;
}

fn require_observation_persistence(passive: bool) -> AppResult<()> {
    #[cfg(test)]
    if passive
        && FAIL_PASSIVE_PERSISTENCE
            .try_with(|fail| *fail)
            .unwrap_or(false)
    {
        return Err(AppError::Internal(
            "Injected passive observation persistence failure".into(),
        ));
    }
    let _ = passive;
    Ok(())
}

/// Only the concrete NyxID transport can construct this observer from a
/// successful final-send claim. There is no client completion-report endpoint.
pub(crate) struct CompletionObserver {
    state: AppState,
    claim: ClaimedDispatch,
}
impl CompletionObserver {
    pub(super) fn new(state: AppState, claim: ClaimedDispatch) -> Self {
        Self { state, claim }
    }
    pub(crate) fn requires_no_redirect(&self) -> bool {
        !self.claim.read_observation
    }

    pub(crate) fn redirect_probe(
        &self,
        actual: &reqwest::Client,
    ) -> Option<super::redirect_observation::Probe> {
        if !self.claim.read_observation {
            return None;
        }
        self.state.http_client.probe(actual)
    }

    pub(crate) async fn disable_observation(&self) -> AppResult<()> {
        if !self.claim.read_observation {
            return Err(conflict());
        }
        self.unknown().await
    }

    pub(crate) fn wrap(self, response: reqwest::Response) -> AppResult<reqwest::Response> {
        use futures::StreamExt;
        use reqwest::ResponseBuilderExt;
        let sse = response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .is_some_and(crate::mw::security_headers::is_sse_media_type);
        let success = response.status().is_success()
            && !(self.claim.operation == StepOperation::Compact && sse);
        let mut builder = http::Response::builder()
            .status(response.status())
            .version(response.version())
            .url(response.url().clone());
        *builder.headers_mut().ok_or_else(invalid)? = response.headers().clone();
        // The wrapper must reach actual upstream EOF, even for fixed lengths.
        builder
            .headers_mut()
            .ok_or_else(invalid)?
            .remove(http::header::CONTENT_LENGTH);
        let stream = async_stream::stream! {
            let mut source = response.bytes_stream();
            let mut bytes = Zeroizing::new(Vec::new());
            let mut bounded = success;
            while let Some(chunk) = source.next().await {
                match chunk {
                    Ok(chunk) => {
                        if bounded {
                            if bytes.len().saturating_add(chunk.len()) > wire::MAX_STREAM_BYTES { bounded=false; bytes.clear(); }
                            else { bytes.extend_from_slice(&chunk); }
                        }
                        yield Ok::<_,std::io::Error>(chunk);
                    }
                    Err(_) => {
                        let _ = self.unknown().await;
                        yield Err(std::io::Error::other("Model control stream interrupted"));
                        return;
                    }
                }
            }
            let completion = bounded.then(|| terminal_response(&bytes, sse)).transpose();
            match completion {
                Ok(Some(value)) => { if self.complete(value, hex::encode(Sha256::digest(bytes.as_slice()))).await.is_err() { let _=self.unknown().await; } },
                _ => { let _=self.unknown().await; },
            }
        };
        builder
            .body(reqwest::Body::wrap_stream(stream))
            .map(reqwest::Response::from)
            .map_err(|_| AppError::Internal("Model control response construction failed".into()))
    }

    async fn unknown(&self) -> AppResult<()> {
        require_observation_persistence(self.claim.read_observation)?;
        let mut session = self.state.db.client().start_session().await?;
        session.start_transaction().and_run2(async |session| {
            let result = async {
                let (mut row, mut private) = persistence::load(&self.state, &self.claim.root_id, Some(&mut *session)).await?;
                if self.claim.read_observation {
                    if row.phase == "unactivated" && !private.preconsent.ineligible {
                        private.preconsent.disable();
                        persistence::save(&self.state, &mut row, &private, session).await?;
                    }
                    return Ok(());
                }
                let step = private.steps.iter_mut().find(|s| s.id == self.claim.step_id).ok_or_else(conflict)?;
                if matches!(&step.phase, StepPhase::Dispatched {dispatch_id} if dispatch_id == &self.claim.dispatch_id) {
                    step.phase = StepPhase::OutcomeUnknown { dispatch_id:self.claim.dispatch_id.clone() };
                    persistence::save(&self.state, &mut row, &private, session).await?;
                }
                Ok(())
            }.await;
            super::super::api_key_mutation_service::transaction_result(result)
        }).await.map_err(super::super::api_key_mutation_service::map_transaction_error)
    }

    async fn complete(&self, value: Value, body_sha256: String) -> AppResult<()> {
        require_observation_persistence(self.claim.read_observation)?;
        let mut session = self.state.db.client().start_session().await?;
        session.start_transaction().and_run2(async |session| {
            let result = async {
                let (mut row, mut private) = persistence::load(&self.state, &self.claim.root_id, Some(&mut *session)).await?;
                if self.claim.read_observation && private.preconsent.ineligible { return Ok(()); }
                let fixed = if self.claim.read_observation {
                    &private.preconsent.reads.iter().find(|r|r.record.id==self.claim.step_id).ok_or_else(conflict)?.envelope.fixed
                } else { &private.envelope.as_ref().ok_or_else(conflict)?.fixed }.clone();
                let step = if self.claim.read_observation {
                    &mut private.preconsent.reads.iter_mut().find(|r|r.record.id==self.claim.step_id).ok_or_else(conflict)?.record
                } else { private.steps.iter_mut().find(|s| s.id == self.claim.step_id).ok_or_else(conflict)? };
                if !matches!(&step.phase, StepPhase::Dispatched {dispatch_id} if dispatch_id == &self.claim.dispatch_id) { return Err(conflict()); }
                let (complete, compaction) = match step.operation {
                    StepOperation::Generation => {
                        (envelope::verify_completion(&value, &fixed)?, None)
                    }
                    StepOperation::Compact => {
                        let complete = compact::verify(value.clone(), &step.input)?;
                        (VerifiedProviderCompletion {response_id:complete.response_id,output:complete.output,calls:vec![]},
                            Some(ObservedCompaction {response:complete.response,body_sha256:body_sha256.clone()}))
                    }
                };
                let output_binding = match &compaction {
                    Some(compaction) => digest(json!({"response":compaction.response,"body_sha256":compaction.body_sha256})),
                    None => digest(json!(complete.output)),
                };
                let receipt = wire::CompletionReceipt {root_id:row.id.clone(),step_id:step.id.clone(),ordinal:step.ordinal,
                    request_binding:step.request_binding.clone(),output_binding,receipt_bearer:persistence::new_token()?};
                step.phase = StepPhase::Completed {completion:ObservedCompletion {response_id:complete.response_id,output:complete.output,calls:complete.calls,compaction},receipt};
                persistence::save(&self.state,&mut row,&private,session).await
            }.await;
            super::super::api_key_mutation_service::transaction_result(result)
        }).await.map_err(super::super::api_key_mutation_service::map_transaction_error)
    }
}

fn terminal_response(bytes: &[u8], sse: bool) -> AppResult<Value> {
    if !sse {
        if bytes.len() > wire::MAX_OUTPUT_BYTES {
            return Err(unsupported());
        }
        return serde_json::from_slice(bytes).map_err(|_| unsupported());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| unsupported())?;
    let normalized = text.replace("\r\n", "\n");
    let mut terminal = None;
    let mut response_id = None;
    let mut events = 0;
    for frame in normalized.split("\n\n") {
        let data = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data:").map(str::trim_start))
            .collect::<Vec<_>>()
            .join("\n");
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        events += 1;
        if terminal.is_some()
            || events > wire::MAX_STREAM_EVENTS
            || data.len() > wire::MAX_OUTPUT_BYTES
        {
            return Err(unsupported());
        }
        let event: Value = serde_json::from_str(&data).map_err(|_| unsupported())?;
        if let Some(id) = event["response"]["id"].as_str() {
            if response_id.as_deref().is_some_and(|prior| prior != id) {
                return Err(unsupported());
            }
            response_id = Some(id.to_owned());
        }
        match event["type"].as_str() {
            Some("response.completed") => terminal = Some(event["response"].clone()),
            Some("error" | "response.failed" | "response.incomplete") => return Err(unsupported()),
            Some(kind) if kind.starts_with("response.") => {}
            _ => return Err(unsupported()),
        }
    }
    // A partial final frame is never upgraded into accepted completed output.
    if !text.ends_with("\n\n") && !text.ends_with("\r\n\r\n") {
        return Err(unsupported());
    }
    terminal.ok_or_else(unsupported)
}
