use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::Utc;
use tauri::{AppHandle, Emitter};
use tauri_plugin_opener::OpenerExt;

use super::client::{
    ClientError, ClientErrorKind, LoginChallenge, LogoutOutcome, NyxIdClient, PollOutcome,
    next_retry_delay,
};
use super::model::{
    CredentialBundle, NyxIdRetryAction, NyxIdView, PendingLoginRecovery, PendingLoginRecoveryPhase,
};
use super::store::{KeyringSessionStore, SharedSessionStore};

pub const NYXID_CHANGED_EVENT: &str = "companion://nyxid-changed";
const ACCESS_EXPIRY_SKEW_SECONDS: i64 = 60;

#[derive(Clone)]
pub struct NyxIdState {
    shared: Arc<Shared>,
}

struct Shared {
    data: Mutex<StateData>,
    client: NyxIdClient,
    store: SharedSessionStore,
    session_mutation: Arc<tokio::sync::Mutex<()>>,
    pending_revoke_memory: Mutex<Vec<CredentialBundle>>,
    pending_login_memory: Mutex<Option<PendingLoginRecovery>>,
}

struct StateData {
    generation: u64,
    view: NyxIdView,
    current_attempt_id: Option<String>,
    current_attempt_cancel: Option<AttemptCancellation>,
    pending_cancel_session_id: Option<String>,
}

#[derive(Clone)]
struct AttemptCancellation {
    attempt_id: String,
    sender: tokio::sync::watch::Sender<bool>,
}

enum RevokeAttempt {
    Complete,
    Retryable {
        bundle: CredentialBundle,
        rotated: bool,
    },
}

struct QueueRevokeResult {
    source_may_clear: bool,
    result: Result<(), NyxIdView>,
}

fn continued_poll_delay(outcome: &PollOutcome, current: Duration) -> Option<Duration> {
    match outcome {
        PollOutcome::Pending => Some(current),
        PollOutcome::SlowDown(hint) => Some(
            current
                .saturating_add(Duration::from_secs(5))
                .max(*hint)
                .min(Duration::from_secs(300)),
        ),
        PollOutcome::RateLimited(retry_after) => {
            Some(next_retry_delay(current, Some(*retry_after)))
        }
        PollOutcome::Denied
        | PollOutcome::Expired
        | PollOutcome::AlreadyDelivered
        | PollOutcome::Delivered(_) => None,
    }
}

impl QueueRevokeResult {
    fn complete() -> Self {
        Self {
            source_may_clear: true,
            result: Ok(()),
        }
    }

    fn failed(source_may_clear: bool, view: NyxIdView) -> Self {
        Self {
            source_may_clear,
            result: Err(view),
        }
    }
}

impl NyxIdState {
    pub fn system() -> Result<Self, reqwest::Error> {
        Ok(Self::new(
            NyxIdClient::new()?,
            KeyringSessionStore::shared(),
        ))
    }

    fn new(client: NyxIdClient, store: SharedSessionStore) -> Self {
        Self {
            shared: Arc::new(Shared {
                data: Mutex::new(StateData {
                    generation: 0,
                    view: NyxIdView::Checking,
                    current_attempt_id: None,
                    current_attempt_cancel: None,
                    pending_cancel_session_id: None,
                }),
                client,
                store,
                session_mutation: Arc::new(tokio::sync::Mutex::new(())),
                pending_revoke_memory: Mutex::new(Vec::new()),
                pending_login_memory: Mutex::new(None),
            }),
        }
    }

    pub fn bootstrap(&self, app: AppHandle) {
        let generation = self.begin_operation(NyxIdView::Checking, &app);
        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            let _mutation = state.shared.session_mutation.lock().await;
            if let Err(view) = state
                .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, false)
                .await
            {
                state.publish_if_current(generation, view, &app);
                return;
            }
            let view = state
                .resolve_connected_from_store_locked(generation, NyxIdRetryAction::Refresh)
                .await;
            state.publish_if_current(generation, view, &app);
        });
    }

    pub fn view(&self) -> NyxIdView {
        self.data().view.clone()
    }

    pub async fn start_login(&self, app: AppHandle) -> NyxIdView {
        if self.login_start_blocked() {
            return self.view();
        }
        let operation_guard = match self.shared.session_mutation.clone().try_lock_owned() {
            Ok(guard) => guard,
            Err(_) => return self.view(),
        };
        if let Err(view) = self
            .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, false)
            .await
        {
            self.begin_operation(view, &app);
            return self.view();
        }
        if let Err(view) = self
            .drain_pending_revoke_locked(NyxIdRetryAction::Connect)
            .await
        {
            self.begin_operation(view, &app);
            return self.view();
        }
        if self.login_start_blocked() {
            return self.view();
        }
        match self.shared.store.load() {
            Ok(Some(bundle)) => {
                let generation = self.begin_operation(NyxIdView::Checking, &app);
                let view = self
                    .resolve_connected_locked(generation, bundle, NyxIdRetryAction::Refresh)
                    .await;
                self.publish_if_current(generation, view, &app);
                return self.view();
            }
            Ok(None) => {}
            Err(_) => {
                self.begin_operation(
                    NyxIdView::error(
                        "credential_store_read_failed",
                        "无法读取本机 NyxID 登录，请稍后重试。",
                        Some(NyxIdRetryAction::Connect),
                    ),
                    &app,
                );
                return self.view();
            }
        }
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let (cancel_sender, cancel_receiver) = tokio::sync::watch::channel(false);
        let generation = self.begin_operation_with_attempt(
            NyxIdView::Checking,
            Some(AttemptCancellation {
                attempt_id: attempt_id.clone(),
                sender: cancel_sender,
            }),
            &app,
        );
        let challenge = match self.shared.client.request_login().await {
            Ok(challenge) => challenge,
            Err(error) => {
                let view = client_error_view(&error, NyxIdRetryAction::Connect);
                self.finish_attempt_if_current(generation, view, &app);
                return self.view();
            }
        };
        let recovery = PendingLoginRecovery::new(
            challenge.device_code.to_string(),
            challenge.recovery_secret.to_string(),
            attempt_id.clone(),
        )
        .expect("validated device challenge and generated recovery secret");
        if self.shared.store.save_pending_login(&recovery).is_err() {
            *self
                .shared
                .pending_login_memory
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(recovery);
            let view = match self
                .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                .await
            {
                Ok(()) => credential_store_view(
                    "无法安全保存 NyxID 登录恢复信息，本次登录已取消。",
                    NyxIdRetryAction::Connect,
                ),
                Err(view) => view,
            };
            self.finish_attempt_if_current(generation, view, &app);
            return self.view();
        }
        if !self.is_current(generation) {
            let _ = self
                .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                .await;
            return self.view();
        }

        let authorizing = NyxIdView::Authorizing {
            user_code: challenge.user_code.clone(),
            verification_url: challenge.verification_url.as_str().to_owned(),
            expires_at: challenge.expires_at.to_rfc3339(),
        };
        if !self.publish_if_current(generation, authorizing, &app) {
            return self.view();
        }
        if app
            .opener()
            .open_url(challenge.verification_url.as_str(), None::<&str>)
            .is_err()
        {
            let view = match self
                .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                .await
            {
                Ok(()) => NyxIdView::error(
                    "verification_url_open_failed",
                    "无法打开 NyxID 确认页面，请重试。",
                    Some(NyxIdRetryAction::Connect),
                ),
                Err(view) => view,
            };
            self.finish_attempt_if_current(generation, view, &app);
            return self.view();
        }

        let state = self.clone();
        let poll_app = app.clone();
        tauri::async_runtime::spawn(async move {
            state
                .poll_until_complete(
                    generation,
                    attempt_id,
                    challenge,
                    cancel_receiver,
                    poll_app,
                    operation_guard,
                )
                .await;
        });
        self.view()
    }

    pub async fn cancel_login(&self, app: AppHandle) -> NyxIdView {
        self.cancel_login_emitting(|view| emit_changed(&app, view))
            .await
    }

    async fn cancel_login_emitting(&self, emit: impl Fn(&NyxIdView) + Send + Sync) -> NyxIdView {
        let attempt_id = {
            let mut data = self.data();
            match &data.view {
                NyxIdView::Authorizing { .. } | NyxIdView::Checking => {
                    let Some(attempt_id) = data.current_attempt_id.clone() else {
                        return data.view.clone();
                    };
                    data.pending_cancel_session_id = Some(attempt_id.clone());
                    Some(attempt_id)
                }
                NyxIdView::Error { error }
                    if error.retry_action == Some(NyxIdRetryAction::Cancel) =>
                {
                    data.pending_cancel_session_id.clone()
                }
                _ => return data.view.clone(),
            }
        };
        let cancellation_generation = self
            .transition_and_emit(None, Some(None), NyxIdView::Checking, &emit)
            .expect("unconditional transition");
        let _mutation = self.shared.session_mutation.lock().await;
        let result = self.complete_cancel_locked(attempt_id.as_deref()).await;
        if let Err(view) = result {
            self.transition_and_emit(Some(cancellation_generation), None, view, &emit);
        } else {
            self.data().pending_cancel_session_id = None;
            self.transition_and_emit(
                Some(cancellation_generation),
                None,
                NyxIdView::SignedOut,
                &emit,
            );
        }
        self.view()
    }

    async fn complete_cancel_locked(&self, attempt_id: Option<&str>) -> Result<(), NyxIdView> {
        self.drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
            .await?;
        self.drain_pending_revoke_locked(NyxIdRetryAction::Cancel)
            .await?;
        match attempt_id {
            Some(attempt_id) => self.revoke_stored_session_locked(attempt_id).await,
            None => Ok(()),
        }
    }

    pub async fn refresh_capabilities(&self, app: AppHandle) -> NyxIdView {
        let _mutation = self.shared.session_mutation.lock().await;
        let generation = self.begin_operation(NyxIdView::Checking, &app);
        if let Err(view) = self
            .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, false)
            .await
        {
            self.publish_if_current(generation, view, &app);
            return self.view();
        }
        let view = self
            .resolve_connected_from_store_locked(generation, NyxIdRetryAction::Refresh)
            .await;
        self.publish_if_current(generation, view, &app);
        self.view()
    }

    pub async fn logout(&self, app: AppHandle) -> NyxIdView {
        let generation = self.begin_operation(NyxIdView::Checking, &app);
        let _mutation = self.shared.session_mutation.lock().await;
        if !self.is_current(generation) {
            return self.view();
        }
        if let Err(view) = self
            .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
            .await
        {
            self.publish_if_current(generation, view, &app);
            return self.view();
        }
        if let Err(view) = self
            .drain_pending_revoke_locked(NyxIdRetryAction::Logout)
            .await
        {
            self.publish_if_current(generation, view, &app);
            return self.view();
        }
        if let Err(view) = self.complete_explicit_logout().await {
            self.publish_if_current(generation, view, &app);
            return self.view();
        }
        self.publish_if_current(generation, NyxIdView::SignedOut, &app);
        self.view()
    }

    async fn complete_explicit_logout(&self) -> Result<(), NyxIdView> {
        let saved = self.shared.store.load().map_err(|_| {
            credential_store_view(
                "无法读取本机 NyxID 登录，请稍后重试。",
                NyxIdRetryAction::Logout,
            )
        })?;
        let revoke = match saved {
            Some(bundle) => Some(
                self.queue_and_revoke_locked(bundle, NyxIdRetryAction::Logout)
                    .await,
            ),
            None => None,
        };
        if revoke
            .as_ref()
            .is_some_and(|outcome| !outcome.source_may_clear)
        {
            return revoke.expect("checked above").result;
        }
        if self.shared.store.clear().is_err() {
            return Err(NyxIdView::error(
                "credential_store_delete_failed",
                "无法清除本机 NyxID 登录，请稍后重试。",
                Some(NyxIdRetryAction::Logout),
            ));
        }
        match revoke {
            Some(outcome) => outcome.result,
            None => Ok(()),
        }
    }

    async fn fail_closed_after_repeated_unauthorized(&self) -> NyxIdView {
        match self.complete_explicit_logout().await {
            Ok(()) => NyxIdView::SignedOut,
            Err(view) => view,
        }
    }

    async fn revoke_bundle(&self, bundle: CredentialBundle) -> RevokeAttempt {
        // An expired access token cannot revoke the server session. Refresh
        // first, and retain the rotated credential whenever logout is retryable.
        let access_is_expired = bundle.access_expires_at
            <= Utc::now() + chrono::Duration::seconds(ACCESS_EXPIRY_SKEW_SECONDS);
        if access_is_expired {
            return self.refresh_then_revoke(bundle).await;
        }

        match self.shared.client.logout(&bundle.access_token).await {
            LogoutOutcome::Complete => RevokeAttempt::Complete,
            LogoutOutcome::Unauthorized => self.refresh_then_revoke(bundle).await,
            LogoutOutcome::Retryable => RevokeAttempt::Retryable {
                bundle,
                rotated: false,
            },
        }
    }

    async fn refresh_then_revoke(&self, bundle: CredentialBundle) -> RevokeAttempt {
        let session_id = bundle.session_id.clone();
        match self.shared.client.refresh(&bundle.refresh_token).await {
            Ok(refreshed) => {
                let refreshed = refreshed
                    .for_session_id(session_id)
                    .expect("stored session id is a UUID v4");
                match self.shared.client.logout(&refreshed.access_token).await {
                    LogoutOutcome::Complete | LogoutOutcome::Unauthorized => {
                        RevokeAttempt::Complete
                    }
                    LogoutOutcome::Retryable => RevokeAttempt::Retryable {
                        bundle: refreshed,
                        rotated: true,
                    },
                }
            }
            Err(error) if error.kind == ClientErrorKind::Unauthorized => RevokeAttempt::Complete,
            Err(_) => RevokeAttempt::Retryable {
                bundle,
                rotated: false,
            },
        }
    }

    async fn drain_pending_revoke_locked(
        &self,
        retry_action: NyxIdRetryAction,
    ) -> Result<(), NyxIdView> {
        let pending = self.shared.store.load_pending_revoke().map_err(|_| {
            credential_store_view("读取待撤销的 NyxID 会话失败，请稍后重试。", retry_action)
        })?;
        if let Some(bundle) = pending {
            match self.revoke_bundle(bundle).await {
                RevokeAttempt::Complete => {
                    self.shared.store.clear_pending_revoke().map_err(|_| {
                        credential_store_view(
                            "清理已撤销的 NyxID 会话失败，请稍后重试。",
                            retry_action,
                        )
                    })?;
                }
                RevokeAttempt::Retryable { bundle, rotated } => {
                    if rotated && self.shared.store.save_pending_revoke(&bundle).is_err() {
                        self.push_pending_memory(bundle);
                        return Err(credential_store_view(
                            "无法保存更新后的待撤销 NyxID 会话，请保持应用运行并重试。",
                            retry_action,
                        ));
                    }
                    return Err(pending_revoke_view(retry_action));
                }
            }
        }

        let memory = {
            let mut pending = self
                .shared
                .pending_revoke_memory
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            std::mem::take(&mut *pending)
        };
        let mut iter = memory.into_iter();
        while let Some(bundle) = iter.next() {
            let initially_persisted = self.shared.store.save_pending_revoke(&bundle).is_ok();
            match self.revoke_bundle(bundle).await {
                RevokeAttempt::Complete => {
                    if initially_persisted && self.shared.store.clear_pending_revoke().is_err() {
                        self.retain_pending_memory(iter);
                        return Err(credential_store_view(
                            "清理已撤销的 NyxID 会话失败，请稍后重试。",
                            retry_action,
                        ));
                    }
                }
                RevokeAttempt::Retryable { bundle, rotated } => {
                    let durable = if initially_persisted && !rotated {
                        true
                    } else {
                        self.shared.store.save_pending_revoke(&bundle).is_ok()
                    };
                    if !durable {
                        self.push_pending_memory(bundle);
                    }
                    self.retain_pending_memory(iter);
                    return Err(if durable {
                        pending_revoke_view(retry_action)
                    } else {
                        credential_store_view(
                            "无法保存待撤销的 NyxID 会话，请保持应用运行并重试。",
                            retry_action,
                        )
                    });
                }
            }
        }
        Ok(())
    }

    async fn queue_and_revoke_locked(
        &self,
        bundle: CredentialBundle,
        retry_action: NyxIdRetryAction,
    ) -> QueueRevokeResult {
        if let Err(view) = self.drain_pending_revoke_locked(retry_action).await {
            self.push_pending_memory(bundle);
            return QueueRevokeResult::failed(false, view);
        }

        let initially_persisted = self.shared.store.save_pending_revoke(&bundle).is_ok();
        match self.revoke_bundle(bundle).await {
            RevokeAttempt::Complete => {
                if self.shared.store.clear_pending_revoke().is_err() {
                    return QueueRevokeResult::failed(
                        true,
                        credential_store_view(
                            "清理已撤销的 NyxID 会话失败，请稍后重试。",
                            retry_action,
                        ),
                    );
                }
                QueueRevokeResult::complete()
            }
            RevokeAttempt::Retryable { bundle, rotated } => {
                let durable = if initially_persisted && !rotated {
                    true
                } else {
                    self.shared.store.save_pending_revoke(&bundle).is_ok()
                };
                if !durable {
                    self.push_pending_memory(bundle);
                }
                QueueRevokeResult::failed(
                    durable,
                    if durable {
                        pending_revoke_view(retry_action)
                    } else {
                        credential_store_view(
                            "无法保存待撤销的 NyxID 会话，请保持应用运行并重试。",
                            retry_action,
                        )
                    },
                )
            }
        }
    }

    fn push_pending_memory(&self, bundle: CredentialBundle) {
        self.shared
            .pending_revoke_memory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(bundle);
    }

    fn retain_pending_memory(&self, bundles: impl IntoIterator<Item = CredentialBundle>) {
        self.shared
            .pending_revoke_memory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend(bundles);
    }

    fn pending_login_recovery(&self) -> Result<Option<PendingLoginRecovery>, Box<NyxIdView>> {
        let durable = self.shared.store.load_pending_login().map_err(|_| {
            Box::new(credential_store_view(
                "无法读取待恢复的 NyxID 登录，请稍后重试。",
                NyxIdRetryAction::Cancel,
            ))
        })?;
        let memory = self
            .shared
            .pending_login_memory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        match (durable, memory) {
            (Some(durable), Some(memory)) if durable.attempt_id != memory.attempt_id => {
                Err(Box::new(credential_store_view(
                    "检测到冲突的 NyxID 登录恢复信息，请保持应用运行并重试。",
                    NyxIdRetryAction::Cancel,
                )))
            }
            (Some(_), Some(memory)) => Ok(Some(memory)),
            (Some(durable), None) => Ok(Some(durable)),
            (None, memory) => Ok(memory),
        }
    }

    fn save_pending_login_phase(
        &self,
        recovery: &PendingLoginRecovery,
        retry_action: NyxIdRetryAction,
    ) -> Result<(), Box<NyxIdView>> {
        if self.shared.store.save_pending_login(recovery).is_ok() {
            return Ok(());
        }
        *self
            .shared
            .pending_login_memory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(recovery.clone());
        Err(Box::new(credential_store_view(
            "无法更新 NyxID 登录恢复信息，请保持应用运行并重试。",
            retry_action,
        )))
    }

    fn clear_pending_login_tracking(
        &self,
        retry_action: NyxIdRetryAction,
    ) -> Result<(), Box<NyxIdView>> {
        self.shared.store.clear_pending_login().map_err(|_| {
            Box::new(credential_store_view(
                "无法清理 NyxID 登录恢复信息，请稍后重试。",
                retry_action,
            ))
        })?;
        self.shared
            .pending_login_memory
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Ok(())
    }

    async fn drain_pending_login_recovery_locked(
        &self,
        retry_action: NyxIdRetryAction,
        force_cancel: bool,
    ) -> Result<(), NyxIdView> {
        let Some(recovery) = self.pending_login_recovery().map_err(|view| *view)? else {
            return Ok(());
        };

        if recovery.phase == PendingLoginRecoveryPhase::DeliveryCommitting && !force_cancel {
            let active = self.shared.store.load().map_err(|_| {
                credential_store_view("无法核对已交付的 NyxID 登录，请稍后重试。", retry_action)
            })?;
            if active
                .as_ref()
                .is_some_and(|bundle| bundle.session_id == recovery.attempt_id)
            {
                return self
                    .clear_pending_login_tracking(NyxIdRetryAction::Refresh)
                    .map_err(|view| *view);
            }
        }

        self.shared
            .client
            .cancel_login(&recovery.device_code, &recovery.recovery_secret)
            .await
            .map_err(|_| pending_login_cancel_view())?;

        let active = self.shared.store.load().map_err(|_| {
            credential_store_view(
                "NyxID 登录已取消，但无法核对本机会话，请稍后重试。",
                retry_action,
            )
        })?;
        if active
            .as_ref()
            .is_some_and(|bundle| bundle.session_id == recovery.attempt_id)
        {
            self.shared.store.clear().map_err(|_| {
                credential_store_view(
                    "NyxID 登录已取消，但无法清除本机会话，请稍后重试。",
                    retry_action,
                )
            })?;
        }
        self.clear_pending_login_tracking(retry_action)
            .map_err(|view| *view)
    }

    async fn poll_until_complete(
        &self,
        generation: u64,
        attempt_id: String,
        challenge: LoginChallenge,
        mut cancel: tokio::sync::watch::Receiver<bool>,
        app: AppHandle,
        _operation_guard: tokio::sync::OwnedMutexGuard<()>,
    ) {
        let mut delay = challenge.interval;
        loop {
            if !self.is_current(generation) || *cancel.borrow() {
                return;
            }
            let remaining = match (challenge.expires_at - Utc::now()).to_std() {
                Ok(remaining) if !remaining.is_zero() => remaining,
                _ => {
                    let view = match self
                        .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                        .await
                    {
                        Ok(()) => NyxIdView::Expired {
                            message: "NyxID 确认已过期，请重新连接。".into(),
                        },
                        Err(view) => view,
                    };
                    self.finish_attempt_if_current(generation, view, &app);
                    return;
                }
            };
            tokio::select! {
                () = tokio::time::sleep(delay.min(remaining)) => {}
                changed = cancel.changed() => {
                    if changed.is_err() || *cancel.borrow() {
                        return;
                    }
                }
            }
            if !self.is_current(generation) {
                return;
            }
            if Utc::now() >= challenge.expires_at {
                let view = match self
                    .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                    .await
                {
                    Ok(()) => NyxIdView::Expired {
                        message: "NyxID 确认已过期，请重新连接。".into(),
                    },
                    Err(view) => view,
                };
                self.finish_attempt_if_current(generation, view, &app);
                return;
            }

            let outcome = match self
                .shared
                .client
                .poll_login(challenge.device_code.as_str())
                .await
            {
                Ok(outcome) => outcome,
                Err(error) if error.retryable => {
                    delay = next_retry_delay(delay, None);
                    continue;
                }
                Err(error) => {
                    let view = match self
                        .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                        .await
                    {
                        Ok(()) => client_error_view(&error, NyxIdRetryAction::Connect),
                        Err(view) => view,
                    };
                    self.finish_attempt_if_current(generation, view, &app);
                    return;
                }
            };
            if let Some(next_delay) = continued_poll_delay(&outcome, delay) {
                delay = next_delay;
                continue;
            }

            match outcome {
                PollOutcome::Denied => {
                    let view = match self
                        .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                        .await
                    {
                        Ok(()) => NyxIdView::Denied {
                            message: "这次 NyxID 连接已被拒绝。".into(),
                        },
                        Err(view) => view,
                    };
                    self.finish_attempt_if_current(generation, view, &app);
                    return;
                }
                PollOutcome::Expired => {
                    let view = match self
                        .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                        .await
                    {
                        Ok(()) => NyxIdView::Expired {
                            message: "NyxID 确认已过期，请重新连接。".into(),
                        },
                        Err(view) => view,
                    };
                    self.finish_attempt_if_current(generation, view, &app);
                    return;
                }
                PollOutcome::AlreadyDelivered => {
                    let view = match self
                        .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                        .await
                    {
                        Ok(()) => NyxIdView::error(
                            "auth_device_already_delivered",
                            "这次 NyxID 登录已经被领取，请重新连接。",
                            Some(NyxIdRetryAction::Connect),
                        ),
                        Err(view) => view,
                    };
                    self.finish_attempt_if_current(generation, view, &app);
                    return;
                }
                PollOutcome::Delivered(bundle) => {
                    let view = self
                        .persist_delivery_and_resolve(generation, &attempt_id, bundle)
                        .await;
                    self.finish_attempt_if_current(generation, view, &app);
                    return;
                }
                PollOutcome::Pending | PollOutcome::SlowDown(_) | PollOutcome::RateLimited(_) => {
                    unreachable!("continuing outcomes handled above")
                }
            }
        }
    }

    async fn persist_delivery_and_resolve(
        &self,
        generation: u64,
        attempt_id: &str,
        bundle: CredentialBundle,
    ) -> NyxIdView {
        let bundle = bundle
            .for_session_id(attempt_id.to_owned())
            .expect("attempt id is generated as UUID v4");
        if !self.is_current(generation) {
            return match self
                .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                .await
            {
                Ok(()) => self.view(),
                Err(view) => view,
            };
        }
        match self.shared.store.load() {
            Ok(Some(_)) => {
                if let Err(view) = self
                    .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                    .await
                {
                    return view;
                }
                return NyxIdView::error(
                    "nyxid_session_already_connected",
                    "NyxID 已连接，未替换现有登录。",
                    None,
                );
            }
            Ok(None) => {}
            Err(_) => {
                if let Err(view) = self
                    .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                    .await
                {
                    return view;
                }
                return credential_store_view(
                    "无法确认本机 NyxID 登录状态，请稍后重试。",
                    NyxIdRetryAction::Connect,
                );
            }
        }

        let mut recovery = match self.pending_login_recovery() {
            Ok(Some(recovery)) if recovery.attempt_id == attempt_id => recovery,
            Ok(_) | Err(_) => {
                let cleanup = self
                    .queue_and_revoke_locked(bundle, NyxIdRetryAction::Connect)
                    .await;
                if let Err(view) = cleanup.result {
                    return view;
                }
                return credential_store_view(
                    "无法确认 NyxID 登录恢复状态，本次会话已撤销。",
                    NyxIdRetryAction::Connect,
                );
            }
        };
        recovery.mark_delivery_committing();
        if let Err(store_view) = self.save_pending_login_phase(&recovery, NyxIdRetryAction::Cancel)
        {
            return match self
                .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                .await
            {
                Ok(()) => *store_view,
                Err(view) => view,
            };
        }

        if self.shared.store.save(&bundle).is_err() {
            return match self
                .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                .await
            {
                Ok(()) => NyxIdView::error(
                    "credential_store_write_failed",
                    "无法安全保存 NyxID 登录，已撤销本次会话。",
                    Some(NyxIdRetryAction::Connect),
                ),
                Err(view) => view,
            };
        }
        if !self.is_current(generation) {
            return match self
                .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
                .await
            {
                Ok(()) => self.view(),
                Err(view) => view,
            };
        }
        if let Err(view) = self.clear_pending_login_tracking(NyxIdRetryAction::Refresh) {
            return *view;
        }
        self.resolve_connected_locked(generation, bundle, NyxIdRetryAction::Refresh)
            .await
    }

    async fn revoke_stored_session_locked(&self, session_id: &str) -> Result<(), NyxIdView> {
        let saved = self.shared.store.load().map_err(|_| {
            credential_store_view(
                "无法读取本机 NyxID 登录，请稍后重试。",
                NyxIdRetryAction::Cancel,
            )
        })?;
        let Some(bundle) = saved else {
            return Ok(());
        };
        if bundle.session_id != session_id {
            return Ok(());
        }
        let revoke = self
            .queue_and_revoke_locked(bundle, NyxIdRetryAction::Cancel)
            .await;
        if revoke.source_may_clear && self.shared.store.clear().is_err() {
            return Err(credential_store_view(
                "无法清除已取消的 NyxID 登录，请稍后重试。",
                NyxIdRetryAction::Cancel,
            ));
        }
        revoke.result
    }

    async fn resolve_connected_from_store_locked(
        &self,
        generation: u64,
        retry_action: NyxIdRetryAction,
    ) -> NyxIdView {
        if !self.is_current(generation) {
            return self.view();
        }
        if let Err(view) = self
            .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, false)
            .await
        {
            return view;
        }
        if let Err(view) = self.drain_pending_revoke_locked(retry_action).await {
            return view;
        }
        let bundle = match self.shared.store.load() {
            Ok(Some(bundle)) => bundle,
            Ok(None) => return NyxIdView::SignedOut,
            Err(_) => {
                return NyxIdView::error(
                    "credential_store_read_failed",
                    "无法读取本机 NyxID 登录，请稍后重试。",
                    Some(retry_action),
                );
            }
        };
        self.resolve_connected_locked(generation, bundle, retry_action)
            .await
    }

    async fn resolve_connected_locked(
        &self,
        generation: u64,
        mut bundle: CredentialBundle,
        retry_action: NyxIdRetryAction,
    ) -> NyxIdView {
        if !self.is_current(generation) {
            return self.view();
        }

        if bundle.access_expires_at
            <= Utc::now() + chrono::Duration::seconds(ACCESS_EXPIRY_SKEW_SECONDS)
            && let Err(view) = self
                .rotate_and_store(generation, &mut bundle, retry_action)
                .await
        {
            return view;
        }

        let mut user = match self.shared.client.get_me(&bundle.access_token).await {
            Ok(user) => user,
            Err(error) if error.kind == ClientErrorKind::Unauthorized => {
                if !self.is_current(generation) {
                    return self.view();
                }
                if let Err(view) = self
                    .rotate_and_store(generation, &mut bundle, retry_action)
                    .await
                {
                    return view;
                }
                match self.shared.client.get_me(&bundle.access_token).await {
                    Ok(user) => user,
                    Err(error) if error.kind == ClientErrorKind::Unauthorized => {
                        return self.fail_closed_after_repeated_unauthorized().await;
                    }
                    Err(error) => return client_error_view(&error, retry_action),
                }
            }
            Err(error) => return client_error_view(&error, retry_action),
        };
        if !self.is_current(generation) {
            return self.view();
        }

        let capabilities = match self
            .shared
            .client
            .get_capabilities(&bundle.access_token)
            .await
        {
            Ok(capabilities) => capabilities,
            Err(error) if error.kind == ClientErrorKind::Unauthorized => {
                if !self.is_current(generation) {
                    return self.view();
                }
                if let Err(view) = self
                    .rotate_and_store(generation, &mut bundle, retry_action)
                    .await
                {
                    return view;
                }
                user = match self.shared.client.get_me(&bundle.access_token).await {
                    Ok(user) => user,
                    Err(error) if error.kind == ClientErrorKind::Unauthorized => {
                        return self.fail_closed_after_repeated_unauthorized().await;
                    }
                    Err(error) => return client_error_view(&error, retry_action),
                };
                match self
                    .shared
                    .client
                    .get_capabilities(&bundle.access_token)
                    .await
                {
                    Ok(capabilities) => capabilities,
                    Err(error) if error.kind == ClientErrorKind::Unauthorized => {
                        return self.fail_closed_after_repeated_unauthorized().await;
                    }
                    Err(error) => return client_error_view(&error, retry_action),
                }
            }
            Err(error) => return client_error_view(&error, retry_action),
        };
        if !self.is_current(generation) {
            return self.view();
        }

        NyxIdView::Connected { user, capabilities }
    }

    async fn rotate_and_store(
        &self,
        generation: u64,
        bundle: &mut CredentialBundle,
        retry_action: NyxIdRetryAction,
    ) -> Result<(), NyxIdView> {
        let session_id = bundle.session_id.clone();
        let refreshed = match self.shared.client.refresh(&bundle.refresh_token).await {
            Ok(refreshed) => refreshed
                .for_session_id(session_id.clone())
                .expect("stored session id is a UUID v4"),
            Err(error) => {
                if error.kind == ClientErrorKind::Unauthorized && self.is_current(generation) {
                    if self.shared.store.clear().is_err() {
                        return Err(credential_store_view(
                            "NyxID 登录已失效，但无法清除本机凭据，请稍后重试。",
                            retry_action,
                        ));
                    }
                    return Err(NyxIdView::SignedOut);
                }
                return Err(client_error_view(&error, retry_action));
            }
        };
        if !self.is_current(generation) {
            let revoke = self.queue_and_revoke_locked(refreshed, retry_action).await;
            if revoke.source_may_clear && self.shared.store.clear().is_err() {
                return Err(credential_store_view(
                    "无法清理失效的本机 NyxID 登录，请稍后重试。",
                    retry_action,
                ));
            }
            revoke.result?;
            return Err(self.view());
        }
        if self.shared.store.save(&refreshed).is_err() {
            let revoke = self.queue_and_revoke_locked(refreshed, retry_action).await;
            if revoke.source_may_clear && self.shared.store.clear().is_err() {
                return Err(credential_store_view(
                    "NyxID 登录已更新，但无法清除失效的本机凭据，请重新连接。",
                    retry_action,
                ));
            }
            revoke.result?;
            return Err(NyxIdView::error(
                "nyxid_reauthentication_required",
                "NyxID 登录已更新但无法安全保存，旧登录已清除，请重新连接。",
                Some(NyxIdRetryAction::Connect),
            ));
        }
        *bundle = refreshed;
        Ok(())
    }

    fn begin_operation(&self, view: NyxIdView, app: &AppHandle) -> u64 {
        self.begin_operation_with_attempt(view, None, app)
    }

    fn begin_operation_with_attempt(
        &self,
        view: NyxIdView,
        attempt: Option<AttemptCancellation>,
        app: &AppHandle,
    ) -> u64 {
        self.transition_and_emit(None, Some(attempt), view, |emitted| {
            emit_changed(app, emitted);
        })
        .expect("unconditional transition")
    }

    fn publish_if_current(&self, generation: u64, view: NyxIdView, app: &AppHandle) -> bool {
        self.transition_and_emit(Some(generation), None, view, |emitted| {
            emit_changed(app, emitted);
        })
        .is_some()
    }

    fn finish_attempt_if_current(&self, generation: u64, view: NyxIdView, app: &AppHandle) -> bool {
        self.transition_and_emit(Some(generation), Some(None), view, |emitted| {
            emit_changed(app, emitted);
        })
        .is_some()
    }

    fn transition_and_emit(
        &self,
        expected_generation: Option<u64>,
        attempt_update: Option<Option<AttemptCancellation>>,
        view: NyxIdView,
        emit: impl FnOnce(&NyxIdView),
    ) -> Option<u64> {
        let mut data = self.data();
        if expected_generation.is_some_and(|expected| data.generation != expected) {
            return None;
        }
        if expected_generation.is_none() {
            data.generation = data.generation.wrapping_add(1);
        }
        if let Some(attempt) = attempt_update {
            if let Some(current) = data.current_attempt_cancel.take() {
                let _ = current.sender.send(true);
            }
            data.current_attempt_id = attempt.as_ref().map(|attempt| attempt.attempt_id.clone());
            data.current_attempt_cancel = attempt;
        }
        data.view = view;
        emit(&data.view);
        Some(data.generation)
    }

    fn is_current(&self, generation: u64) -> bool {
        self.data().generation == generation
    }

    fn login_start_blocked(&self) -> bool {
        matches!(
            self.view(),
            NyxIdView::Connected { .. } | NyxIdView::Authorizing { .. }
        )
    }

    fn data(&self) -> MutexGuard<'_, StateData> {
        self.shared
            .data
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

fn client_error_view(error: &ClientError, retry_action: NyxIdRetryAction) -> NyxIdView {
    NyxIdView::error(
        error.code,
        error.message,
        error.retryable.then_some(retry_action),
    )
}

fn credential_store_view(message: &str, retry_action: NyxIdRetryAction) -> NyxIdView {
    NyxIdView::error("credential_store_failed", message, Some(retry_action))
}

fn pending_revoke_view(retry_action: NyxIdRetryAction) -> NyxIdView {
    NyxIdView::error(
        "nyxid_revoke_pending",
        "暂时无法撤销旧的 NyxID 会话，凭据已保留，将在重试时继续处理。",
        Some(retry_action),
    )
}

fn pending_login_cancel_view() -> NyxIdView {
    NyxIdView::error(
        "nyxid_login_cancel_pending",
        "暂时无法确认 NyxID 登录已取消，恢复信息已保留，请重试取消。",
        Some(NyxIdRetryAction::Cancel),
    )
}

fn emit_changed(app: &AppHandle, view: &NyxIdView) {
    let _ = app.emit(NYXID_CHANGED_EVENT, view.clone());
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread;

    use super::super::store::SessionStore;
    use super::super::store::StoreOperation;
    use super::super::store::tests_support::MemoryStore;
    use super::*;

    fn login_recovery(attempt_id: &str) -> PendingLoginRecovery {
        PendingLoginRecovery::new(
            format!("nyx_adc_{}", URL_SAFE_NO_PAD.encode([21_u8; 32])),
            URL_SAFE_NO_PAD.encode([22_u8; 32]),
            attempt_id.to_owned(),
        )
        .unwrap()
    }

    #[test]
    fn generation_fence_rejects_stale_mutation() {
        let state = NyxIdState::new(NyxIdClient::new().unwrap(), MemoryStore::shared());
        {
            let mut data = state.data();
            data.generation = 8;
            data.view = NyxIdView::Checking;
        }
        {
            let mut data = state.data();
            if data.generation == 7 {
                data.view = NyxIdView::SignedOut;
            }
        }
        assert_eq!(state.view(), NyxIdView::Checking);
    }

    #[tokio::test]
    async fn missing_bundle_resolves_to_signed_out() {
        let state = NyxIdState::new(NyxIdClient::new().unwrap(), MemoryStore::shared());
        assert_eq!(
            state
                .resolve_connected_from_store_locked(0, NyxIdRetryAction::Refresh)
                .await,
            NyxIdView::SignedOut
        );
    }

    #[test]
    fn rate_limited_polling_honors_retry_after_and_remains_in_the_loop() {
        assert_eq!(
            continued_poll_delay(
                &PollOutcome::RateLimited(Duration::from_secs(75)),
                Duration::from_secs(5),
            ),
            Some(Duration::from_secs(75))
        );
        assert_eq!(
            continued_poll_delay(&PollOutcome::Pending, Duration::from_secs(75)),
            Some(Duration::from_secs(75))
        );
        assert_eq!(
            continued_poll_delay(&PollOutcome::Denied, Duration::from_secs(75)),
            None
        );
    }

    #[tokio::test]
    async fn restart_cancels_durable_pending_login_with_the_bound_capabilities() {
        let store = MemoryStore::shared();
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let recovery = login_recovery(&attempt_id);
        store.save_pending_login(&recovery).unwrap();
        let (client, cancel_probe) = NyxIdClient::with_cancel_probe(
            recovery.device_code.to_string(),
            recovery.recovery_secret.to_string(),
            super::super::client::CancelProbeOutcome::Complete,
        );
        let restarted = NyxIdState::new(client, store.clone());

        restarted
            .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, false)
            .await
            .unwrap();

        assert_eq!(cancel_probe.load(Ordering::SeqCst), 1);
        assert!(store.load_pending_login().unwrap().is_none());
    }

    #[tokio::test]
    async fn matching_delivery_cleanup_retry_never_cancels_the_committed_session() {
        let store = MemoryStore::shared();
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let mut recovery = login_recovery(&attempt_id);
        recovery.mark_delivery_committing();
        store.save_pending_login(&recovery).unwrap();
        store
            .save(
                &CredentialBundle::new(
                    "committed-access".into(),
                    "committed-refresh".into(),
                    Utc::now() + chrono::Duration::minutes(10),
                )
                .unwrap()
                .for_session_id(attempt_id.clone())
                .unwrap(),
            )
            .unwrap();
        let (client, cancel_probe) = NyxIdClient::with_cancel_probe(
            recovery.device_code.to_string(),
            recovery.recovery_secret.to_string(),
            super::super::client::CancelProbeOutcome::Retryable,
        );
        let restarted = NyxIdState::new(client, store.clone());

        store.set_fail_delete(true);
        let failed_cleanup = restarted
            .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, false)
            .await
            .unwrap_err();

        assert!(matches!(
            failed_cleanup,
            NyxIdView::Error { error }
                if error.retry_action == Some(NyxIdRetryAction::Refresh)
        ));
        assert_eq!(cancel_probe.load(Ordering::SeqCst), 0);
        assert_eq!(store.load().unwrap().unwrap().session_id, attempt_id);
        assert!(store.load_pending_login().unwrap().is_some());

        store.set_fail_delete(false);
        restarted
            .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, false)
            .await
            .unwrap();

        assert_eq!(cancel_probe.load(Ordering::SeqCst), 0);
        assert_eq!(store.load().unwrap().unwrap().session_id, attempt_id);
        assert!(store.load_pending_login().unwrap().is_none());
    }

    #[tokio::test]
    async fn failed_cancel_retains_recovery_and_returns_cancel_retry_action() {
        let store = MemoryStore::shared();
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let recovery = login_recovery(&attempt_id);
        store.save_pending_login(&recovery).unwrap();
        let (client, cancel_probe) = NyxIdClient::with_cancel_probe(
            recovery.device_code.to_string(),
            recovery.recovery_secret.to_string(),
            super::super::client::CancelProbeOutcome::Retryable,
        );
        let state = NyxIdState::new(client, store.clone());

        let view = state
            .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
            .await
            .unwrap_err();

        assert_eq!(cancel_probe.load(Ordering::SeqCst), 1);
        assert!(store.load_pending_login().unwrap().is_some());
        assert!(matches!(
            view,
            NyxIdView::Error { error }
                if error.retry_action == Some(NyxIdRetryAction::Cancel)
        ));
    }

    #[tokio::test]
    async fn confirmed_cancel_with_failed_keychain_cleanup_retains_recovery() {
        let store = MemoryStore::shared();
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let recovery = login_recovery(&attempt_id);
        store.save_pending_login(&recovery).unwrap();
        store.set_fail_delete(true);
        let (client, cancel_probe) = NyxIdClient::with_cancel_probe(
            recovery.device_code.to_string(),
            recovery.recovery_secret.to_string(),
            super::super::client::CancelProbeOutcome::Complete,
        );
        let state = NyxIdState::new(client, store.clone());

        let first = state
            .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
            .await
            .unwrap_err();

        assert!(matches!(
            first,
            NyxIdView::Error { error }
                if error.retry_action == Some(NyxIdRetryAction::Cancel)
        ));
        assert_eq!(cancel_probe.load(Ordering::SeqCst), 1);
        assert!(store.load_pending_login().unwrap().is_some());

        store.set_fail_delete(false);
        state
            .drain_pending_login_recovery_locked(NyxIdRetryAction::Cancel, true)
            .await
            .unwrap();
        assert_eq!(cancel_probe.load(Ordering::SeqCst), 2);
        assert!(store.load_pending_login().unwrap().is_none());
    }

    #[tokio::test]
    async fn user_cancel_signals_before_waiting_for_guard_then_calls_server_cancel() {
        let store = MemoryStore::shared();
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let recovery = login_recovery(&attempt_id);
        store.save_pending_login(&recovery).unwrap();
        let (client, cancel_probe) = NyxIdClient::with_cancel_probe(
            recovery.device_code.to_string(),
            recovery.recovery_secret.to_string(),
            super::super::client::CancelProbeOutcome::Complete,
        );
        let state = NyxIdState::new(client, store.clone());
        let (sender, mut receiver) = tokio::sync::watch::channel(false);
        state.transition_and_emit(
            None,
            Some(Some(AttemptCancellation { attempt_id, sender })),
            NyxIdView::Authorizing {
                user_code: "ABCD-EFGH".into(),
                verification_url: "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-EFGH"
                    .into(),
                expires_at: (Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
            },
            |_| {},
        );
        let guard = state.shared.session_mutation.clone().lock_owned().await;
        let cancel_state = state.clone();
        let cancel_task =
            tokio::spawn(async move { cancel_state.cancel_login_emitting(|_| {}).await });

        tokio::time::timeout(Duration::from_millis(100), receiver.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(*receiver.borrow());
        assert!(!cancel_task.is_finished());
        assert_eq!(cancel_probe.load(Ordering::SeqCst), 0);

        drop(guard);
        assert_eq!(cancel_task.await.unwrap(), NyxIdView::SignedOut);
        assert_eq!(cancel_probe.load(Ordering::SeqCst), 1);
        assert!(store.load_pending_login().unwrap().is_none());
    }

    #[tokio::test]
    async fn delivery_phase_write_failure_cancels_before_persisting_session() {
        let store = MemoryStore::shared();
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let recovery = login_recovery(&attempt_id);
        store.save_pending_login(&recovery).unwrap();
        store.set_fail_pending_login_write(true);
        let (client, cancel_probe) = NyxIdClient::with_cancel_probe(
            recovery.device_code.to_string(),
            recovery.recovery_secret.to_string(),
            super::super::client::CancelProbeOutcome::Complete,
        );
        let state = NyxIdState::new(client, store.clone());

        let view = state
            .persist_delivery_and_resolve(
                0,
                &attempt_id,
                CredentialBundle::new(
                    "uncommitted-access".into(),
                    "uncommitted-refresh".into(),
                    Utc::now() + chrono::Duration::minutes(10),
                )
                .unwrap(),
            )
            .await;

        assert!(matches!(view, NyxIdView::Error { .. }));
        assert_eq!(cancel_probe.load(Ordering::SeqCst), 1);
        assert!(store.load().unwrap().is_none());
        assert!(store.load_pending_login().unwrap().is_none());
    }

    #[test]
    fn store_operation_debug_carries_no_secret_material() {
        assert_eq!(format!("{:?}", StoreOperation::Write), "Write");
    }

    #[tokio::test]
    async fn stale_device_delivery_is_revoked_without_being_stored() {
        let store = MemoryStore::shared();
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let recovery = login_recovery(&attempt_id);
        store.save_pending_login(&recovery).unwrap();
        let (client, cancel_probe) = NyxIdClient::with_cancel_probe(
            recovery.device_code.to_string(),
            recovery.recovery_secret.to_string(),
            super::super::client::CancelProbeOutcome::Complete,
        );
        let state = NyxIdState::new(client, store.clone());
        state.data().generation = 2;
        let delivered = CredentialBundle::new(
            "delivered-access".into(),
            "delivered-refresh".into(),
            Utc::now() + chrono::Duration::minutes(10),
        )
        .unwrap();

        let _ = state
            .persist_delivery_and_resolve(1, &attempt_id, delivered)
            .await;

        assert_eq!(cancel_probe.load(Ordering::SeqCst), 1);
        assert!(store.load().unwrap().is_none());
        assert!(store.load_pending_login().unwrap().is_none());
    }

    #[tokio::test]
    async fn failed_stale_delivery_cancel_retains_recovery_for_retry() {
        let store = MemoryStore::shared();
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let recovery = login_recovery(&attempt_id);
        store.save_pending_login(&recovery).unwrap();
        let (client, cancel_probe) = NyxIdClient::with_cancel_probe(
            recovery.device_code.to_string(),
            recovery.recovery_secret.to_string(),
            super::super::client::CancelProbeOutcome::Retryable,
        );
        let state = NyxIdState::new(client, store.clone());
        state.data().generation = 2;
        let delivered = CredentialBundle::new(
            "late-access".into(),
            "late-refresh".into(),
            Utc::now() + chrono::Duration::minutes(10),
        )
        .unwrap();

        let _ = state
            .persist_delivery_and_resolve(1, &attempt_id, delivered)
            .await;

        assert_eq!(cancel_probe.load(Ordering::SeqCst), 1);
        assert!(store.load().unwrap().is_none());
        assert!(store.load_pending_revoke().unwrap().is_none());
        assert_eq!(
            store.load_pending_login().unwrap().unwrap().attempt_id,
            attempt_id
        );
    }

    #[tokio::test]
    async fn matching_saved_delivery_is_taken_and_revoked() {
        let store = MemoryStore::shared();
        let bundle = CredentialBundle::new(
            "saved-access".into(),
            "saved-refresh".into(),
            Utc::now() + chrono::Duration::minutes(10),
        )
        .unwrap()
        .for_session_id(uuid::Uuid::new_v4().to_string())
        .unwrap();
        let session_id = bundle.session_id.clone();
        store.save(&bundle).unwrap();
        let (client, logout_probe) = NyxIdClient::with_logout_probe();
        let state = NyxIdState::new(client, store.clone());

        state
            .revoke_stored_session_locked(&session_id)
            .await
            .unwrap();

        assert_eq!(logout_probe.load(Ordering::SeqCst), 1);
        assert!(store.load().unwrap().is_none());
    }

    #[tokio::test]
    async fn explicit_logout_refreshes_an_expired_access_token_before_revoking() {
        let store = MemoryStore::shared();
        let (client, logout_probe, refresh_probe) = NyxIdClient::with_logout_and_refresh_probe();
        let state = NyxIdState::new(client, store.clone());
        let bundle = CredentialBundle::new(
            "expired-access".into(),
            "refresh-token".into(),
            Utc::now() - chrono::Duration::minutes(1),
        )
        .unwrap();
        store.save(&bundle).unwrap();

        state.complete_explicit_logout().await.unwrap();

        assert_eq!(refresh_probe.load(Ordering::SeqCst), 1);
        assert_eq!(logout_probe.load(Ordering::SeqCst), 1);
        assert!(store.load().unwrap().is_none());
        assert!(store.load_pending_revoke().unwrap().is_none());
    }

    #[tokio::test]
    async fn retryable_remote_logout_preserves_the_session_in_durable_pending_storage() {
        let store = MemoryStore::shared();
        let bundle = CredentialBundle::new(
            "valid-access".into(),
            "refresh-token".into(),
            Utc::now() + chrono::Duration::minutes(10),
        )
        .unwrap();
        store.save(&bundle).unwrap();
        let (client, logout_probe) =
            NyxIdClient::with_logout_probe_outcome(LogoutOutcome::Retryable);
        let state = NyxIdState::new(client, store.clone());

        let view = state.complete_explicit_logout().await.unwrap_err();

        assert!(matches!(view, NyxIdView::Error { .. }));
        assert_eq!(logout_probe.load(Ordering::SeqCst), 1);
        assert!(store.load().unwrap().is_none());
        assert_eq!(
            store
                .load_pending_revoke()
                .unwrap()
                .unwrap()
                .access_token
                .as_str(),
            "valid-access"
        );
    }

    #[tokio::test]
    async fn explicit_logout_read_failure_preserves_active_credentials() {
        let store = MemoryStore::shared();
        let bundle = CredentialBundle::new(
            "valid-access".into(),
            "valid-refresh".into(),
            Utc::now() + chrono::Duration::minutes(10),
        )
        .unwrap();
        store.save(&bundle).unwrap();
        store.set_fail_read(true);
        let (client, logout_probe) = NyxIdClient::with_logout_probe();
        let state = NyxIdState::new(client, store.clone());

        let view = state.complete_explicit_logout().await.unwrap_err();

        assert!(matches!(view, NyxIdView::Error { .. }));
        assert_eq!(logout_probe.load(Ordering::SeqCst), 0);
        store.set_fail_read(false);
        assert_eq!(
            store.load().unwrap().unwrap().access_token.as_str(),
            "valid-access"
        );
    }

    #[tokio::test]
    async fn expired_logout_retry_persists_the_rotated_bundle() {
        let store = MemoryStore::shared();
        let bundle = CredentialBundle::new(
            "expired-access".into(),
            "old-refresh".into(),
            Utc::now() - chrono::Duration::minutes(1),
        )
        .unwrap();
        store.save(&bundle).unwrap();
        let (client, logout_probe, refresh_probe) =
            NyxIdClient::with_logout_outcome_and_refresh_probe(LogoutOutcome::Retryable);
        let state = NyxIdState::new(client, store.clone());

        let view = state.complete_explicit_logout().await.unwrap_err();

        assert!(matches!(view, NyxIdView::Error { .. }));
        assert_eq!(refresh_probe.load(Ordering::SeqCst), 1);
        assert_eq!(logout_probe.load(Ordering::SeqCst), 1);
        assert!(store.load().unwrap().is_none());
        let pending = store.load_pending_revoke().unwrap().unwrap();
        assert_eq!(pending.access_token.as_str(), "refreshed-access");
        assert_eq!(pending.refresh_token.as_str(), "refreshed-refresh");
    }

    #[tokio::test]
    async fn retryable_logout_with_failed_pending_write_keeps_active_credentials() {
        let store = MemoryStore::shared();
        let bundle = CredentialBundle::new(
            "valid-access".into(),
            "valid-refresh".into(),
            Utc::now() + chrono::Duration::minutes(10),
        )
        .unwrap();
        store.save(&bundle).unwrap();
        store.set_fail_pending_write(true);
        let (client, logout_probe) =
            NyxIdClient::with_logout_probe_outcome(LogoutOutcome::Retryable);
        let state = NyxIdState::new(client, store.clone());

        let view = state.complete_explicit_logout().await.unwrap_err();

        assert!(matches!(view, NyxIdView::Error { .. }));
        assert_eq!(logout_probe.load(Ordering::SeqCst), 1);
        assert_eq!(
            store.load().unwrap().unwrap().access_token.as_str(),
            "valid-access"
        );
        assert!(store.load_pending_revoke().unwrap().is_none());
        assert_eq!(state.shared.pending_revoke_memory.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn retryable_pending_revoke_blocks_active_session_work_without_overwrite() {
        let store = MemoryStore::shared();
        store
            .save(
                &CredentialBundle::new(
                    "active-access".into(),
                    "active-refresh".into(),
                    Utc::now() + chrono::Duration::minutes(10),
                )
                .unwrap(),
            )
            .unwrap();
        store
            .save_pending_revoke(
                &CredentialBundle::new(
                    "pending-access".into(),
                    "pending-refresh".into(),
                    Utc::now() + chrono::Duration::minutes(10),
                )
                .unwrap(),
            )
            .unwrap();
        let (client, logout_probe) =
            NyxIdClient::with_logout_probe_outcome(LogoutOutcome::Retryable);
        let state = NyxIdState::new(client, store.clone());

        let view = state
            .resolve_connected_from_store_locked(0, NyxIdRetryAction::Refresh)
            .await;

        assert!(matches!(view, NyxIdView::Error { .. }));
        assert_eq!(logout_probe.load(Ordering::SeqCst), 1);
        assert_eq!(
            store.load().unwrap().unwrap().access_token.as_str(),
            "active-access"
        );
        assert_eq!(
            store
                .load_pending_revoke()
                .unwrap()
                .unwrap()
                .access_token
                .as_str(),
            "pending-access"
        );
    }

    #[tokio::test]
    async fn cancel_failure_emits_checking_then_error_and_retry_finishes_signed_out() {
        let store = MemoryStore::shared();
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let bundle = CredentialBundle::new(
            "cancel-access".into(),
            "cancel-refresh".into(),
            Utc::now() + chrono::Duration::minutes(10),
        )
        .unwrap()
        .for_session_id(attempt_id.clone())
        .unwrap();
        store.save(&bundle).unwrap();
        store.set_fail_delete(true);
        let (client, logout_probe) = NyxIdClient::with_logout_probe();
        let state = NyxIdState::new(client, store.clone());
        {
            let mut data = state.data();
            data.view = NyxIdView::Authorizing {
                user_code: "ABCD-EFGH".into(),
                verification_url: "https://nyx.chrono-ai.fun/login/device?user_code=ABCD-EFGH"
                    .into(),
                expires_at: (Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
            };
            data.current_attempt_id = Some(attempt_id);
        }
        let events = Arc::new(Mutex::new(Vec::new()));

        let first_events = events.clone();
        let first = state
            .cancel_login_emitting(move |view| first_events.lock().unwrap().push(view.clone()))
            .await;

        assert!(matches!(first, NyxIdView::Error { .. }));
        assert!(matches!(events.lock().unwrap()[0], NyxIdView::Checking));
        assert!(matches!(events.lock().unwrap()[1], NyxIdView::Error { .. }));
        assert!(
            !events
                .lock()
                .unwrap()
                .iter()
                .any(|view| matches!(view, NyxIdView::SignedOut))
        );

        store.set_fail_delete(false);
        events.lock().unwrap().clear();
        let retry_events = events.clone();
        let retried = state
            .cancel_login_emitting(move |view| retry_events.lock().unwrap().push(view.clone()))
            .await;

        assert_eq!(retried, NyxIdView::SignedOut);
        assert_eq!(
            *events.lock().unwrap(),
            [NyxIdView::Checking, NyxIdView::SignedOut]
        );
        assert!(store.load().unwrap().is_none());
        assert!(store.load_pending_revoke().unwrap().is_none());
        assert!(state.data().pending_cancel_session_id.is_none());
        assert_eq!(logout_probe.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn refresh_store_failure_revokes_new_session_and_clears_invalid_old_session() {
        let store = MemoryStore::shared();
        let mut bundle = CredentialBundle::new(
            "old-access".into(),
            "old-refresh".into(),
            Utc::now() - chrono::Duration::minutes(1),
        )
        .unwrap();
        store.save(&bundle).unwrap();
        store.set_fail_write(true);
        let (client, logout_probe, refresh_probe) = NyxIdClient::with_logout_and_refresh_probe();
        let state = NyxIdState::new(client, store.clone());

        let view = state
            .rotate_and_store(0, &mut bundle, NyxIdRetryAction::Refresh)
            .await
            .unwrap_err();

        assert!(matches!(view, NyxIdView::Error { .. }));
        assert_eq!(refresh_probe.load(Ordering::SeqCst), 1);
        assert_eq!(logout_probe.load(Ordering::SeqCst), 1);
        store.set_fail_write(false);
        assert!(store.load().unwrap().is_none());
        assert!(store.load_pending_revoke().unwrap().is_none());
    }

    #[tokio::test]
    async fn unauthorized_refresh_clears_credentials_and_returns_to_signed_out() {
        let store = MemoryStore::shared();
        let mut bundle = CredentialBundle::new(
            "expired-access".into(),
            "revoked-refresh".into(),
            Utc::now() - chrono::Duration::minutes(1),
        )
        .unwrap();
        store.save(&bundle).unwrap();
        let (client, refresh_probe) = NyxIdClient::with_unauthorized_refresh_probe();
        let state = NyxIdState::new(client, store.clone());

        let view = state
            .rotate_and_store(0, &mut bundle, NyxIdRetryAction::Refresh)
            .await
            .unwrap_err();

        assert_eq!(view, NyxIdView::SignedOut);
        assert_eq!(refresh_probe.load(Ordering::SeqCst), 1);
        assert!(store.load().unwrap().is_none());
    }

    #[tokio::test]
    async fn repeated_profile_or_capability_unauthorized_fails_closed_after_refresh() {
        use super::super::client::AccountProbeMode;

        for (mode, expected_profile_calls, expected_capability_calls) in [
            (AccountProbeMode::ProfileAlwaysUnauthorized, 2, 0),
            (AccountProbeMode::CapabilitiesAlwaysUnauthorized, 2, 2),
        ] {
            let store = MemoryStore::shared();
            store
                .save(
                    &CredentialBundle::new(
                        "initial-access".into(),
                        "initial-refresh".into(),
                        Utc::now() + chrono::Duration::minutes(10),
                    )
                    .unwrap(),
                )
                .unwrap();
            let (client, probes) = NyxIdClient::with_repeated_account_unauthorized_probe(mode);
            let state = NyxIdState::new(client, store.clone());

            let view = state
                .resolve_connected_from_store_locked(0, NyxIdRetryAction::Refresh)
                .await;

            assert_eq!(view, NyxIdView::SignedOut);
            assert_eq!(probes.refresh_calls.load(Ordering::SeqCst), 1);
            assert_eq!(probes.logout_calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                probes.profile_calls.load(Ordering::SeqCst),
                expected_profile_calls
            );
            assert_eq!(
                probes.capabilities_calls.load(Ordering::SeqCst),
                expected_capability_calls
            );
            assert!(store.load().unwrap().is_none());
            assert!(store.load_pending_revoke().unwrap().is_none());
        }
    }

    #[tokio::test]
    async fn refresh_store_failure_with_retryable_revoke_persists_new_session_as_pending() {
        let store = MemoryStore::shared();
        let mut bundle = CredentialBundle::new(
            "old-access".into(),
            "old-refresh".into(),
            Utc::now() - chrono::Duration::minutes(1),
        )
        .unwrap();
        store.save(&bundle).unwrap();
        store.set_fail_write(true);
        let (client, logout_probe, refresh_probe) =
            NyxIdClient::with_logout_outcome_and_refresh_probe(LogoutOutcome::Retryable);
        let state = NyxIdState::new(client, store.clone());

        let view = state
            .rotate_and_store(0, &mut bundle, NyxIdRetryAction::Refresh)
            .await
            .unwrap_err();

        assert!(matches!(view, NyxIdView::Error { .. }));
        assert_eq!(refresh_probe.load(Ordering::SeqCst), 1);
        assert_eq!(logout_probe.load(Ordering::SeqCst), 1);
        store.set_fail_write(false);
        assert!(store.load().unwrap().is_none());
        assert_eq!(
            store
                .load_pending_revoke()
                .unwrap()
                .unwrap()
                .access_token
                .as_str(),
            "refreshed-access"
        );
    }

    #[tokio::test]
    async fn cancellation_before_waiter_registration_is_observed_and_is_attempt_scoped() {
        let state = Arc::new(NyxIdState::new(
            NyxIdClient::new().unwrap(),
            MemoryStore::shared(),
        ));
        let first_attempt_id = uuid::Uuid::new_v4().to_string();
        let (first_sender, mut first_receiver) = tokio::sync::watch::channel(false);
        let stale_sender = first_sender.clone();
        let generation = state
            .transition_and_emit(
                None,
                Some(Some(AttemptCancellation {
                    attempt_id: first_attempt_id,
                    sender: first_sender,
                })),
                NyxIdView::Checking,
                |_| {},
            )
            .unwrap();
        let guard = state.shared.session_mutation.clone().lock_owned().await;

        state.transition_and_emit(None, Some(None), NyxIdView::SignedOut, |_| {});

        let simulated_poll = tokio::spawn(async move {
            let _guard = guard;
            tokio::select! {
                () = tokio::time::sleep(Duration::from_secs(30)) => false,
                changed = first_receiver.changed() => {
                    changed.is_ok() && *first_receiver.borrow()
                }
            }
        });

        assert!(
            tokio::time::timeout(Duration::from_millis(100), simulated_poll)
                .await
                .unwrap()
                .unwrap()
        );
        assert!(
            tokio::time::timeout(
                Duration::from_millis(100),
                state.shared.session_mutation.lock()
            )
            .await
            .is_ok()
        );

        let second_attempt_id = uuid::Uuid::new_v4().to_string();
        let (second_sender, second_receiver) = tokio::sync::watch::channel(false);
        state.transition_and_emit(
            None,
            Some(Some(AttemptCancellation {
                attempt_id: second_attempt_id,
                sender: second_sender,
            })),
            NyxIdView::Checking,
            |_| {},
        );
        let _ = stale_sender.send(true);
        assert!(!*second_receiver.borrow());
        assert!(!state.is_current(generation));
    }

    #[test]
    fn connected_state_blocks_a_second_device_login() {
        use super::super::model::{NyxIdCapabilities, NyxIdUser};

        let state = NyxIdState::new(NyxIdClient::new().unwrap(), MemoryStore::shared());
        state.data().view = NyxIdView::Connected {
            user: NyxIdUser {
                id: "user-1".into(),
                email: "user@example.com".into(),
                display_name: None,
                avatar_url: None,
            },
            capabilities: NyxIdCapabilities::from_services(Vec::new(), Utc::now()),
        };

        assert!(state.login_start_blocked());
    }

    #[test]
    fn state_events_cannot_overtake_the_mutation_that_emits_them() {
        let state = Arc::new(NyxIdState::new(
            NyxIdClient::new().unwrap(),
            MemoryStore::shared(),
        ));
        let events = Arc::new(Mutex::new(Vec::new()));
        let (first_emitting_tx, first_emitting_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();

        let first_state = state.clone();
        let first_events = events.clone();
        let first = thread::spawn(move || {
            first_state.transition_and_emit(None, None, NyxIdView::Checking, |view| {
                first_emitting_tx.send(()).unwrap();
                release_first_rx.recv().unwrap();
                first_events.lock().unwrap().push(view.clone());
            });
        });
        first_emitting_rx.recv().unwrap();

        let second_state = state.clone();
        let second_events = events.clone();
        let second = thread::spawn(move || {
            second_state.transition_and_emit(None, None, NyxIdView::SignedOut, |view| {
                second_events.lock().unwrap().push(view.clone());
            });
        });

        assert!(events.lock().unwrap().is_empty());
        release_first_tx.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();

        assert_eq!(
            *events.lock().unwrap(),
            [NyxIdView::Checking, NyxIdView::SignedOut]
        );
        assert_eq!(state.view(), NyxIdView::SignedOut);
    }
}
