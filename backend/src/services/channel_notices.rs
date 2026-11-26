//! Fixed NyxID channel copy. The settings chat supplies the audience language.
use crate::{
    errors::AppResult,
    models::nyxbot_channel::{NoticeLanguage, NyxbotThread, THREADS_COLLECTION_NAME},
};
use bson::doc;

pub(crate) async fn chat_language(
    db: &mongodb::Database,
    channel_id: &str,
    partition: &str,
) -> AppResult<Option<NoticeLanguage>> {
    let chats = db.collection::<NyxbotThread>(THREADS_COLLECTION_NAME);
    let Some(chat) = chats
        .find_one(doc! {"channel_id":channel_id,"partition":partition})
        .await?
    else {
        return Ok(None);
    };
    if let Some(settings) = &chat.follow.settings_chat_id {
        return Ok(chats
            .find_one(doc! {"_id":settings,"channel_id":&chat.channel_id,"user_id":&chat.user_id})
            .await?
            .and_then(|settings| settings.notice_language));
    }
    Ok(chat.notice_language)
}

pub(crate) enum Notice {
    GuestModelRoute,
    GuestModelRouteNoTools,
    GuestModelCapacity,
    GuestSessionBusy,
    RetryAttachmentUnavailable,
    GuestSetupNoTools,
    NoTools,
    OutcomeUnknown,
    NativeBusy,
    RelayBusy,
    ImagesUnavailable,
    Pending,
    Checking,
    Unavailable,
    GuestBusy,
    FollowBusy,
    StartFailure,
    DependencyUnavailable,
    Capacity,
    ThreadLimit,
    Stopped,
    OwnerAbsent,
    SlowDown,
    MediaUnavailable,
    QueuedThread,
    WorkingThread,
    Working,
    WorkingHereToo,
    WorkingNyxid,
    WorkingHere,
    Answered,
    QueuedNext,
    VerifyMember,
    RestoredMember,
}

impl Notice {
    pub(crate) fn org_guest_failure(code: Option<&str>) -> Self {
        match code {
            Some("guest_model_route_required") => Self::GuestModelRoute,
            Some("capacity_exceeded") => Self::GuestModelCapacity,
            Some("session_busy" | "queue_timeout") => Self::GuestSessionBusy,
            _ => Self::Unavailable,
        }
    }

    pub(crate) fn text(self, language: Option<NoticeLanguage>) -> &'static str {
        let (en, cn, tw) = match self {
            Self::GuestModelRoute => (
                "The assistant could not use its model for this question. Please check any actions already taken before trying again.",
                "助手无法使用模型回答这个问题。请先确认已执行的操作，再试一次。",
                "助理無法使用模型回答這個問題。請先確認已執行的操作，再試一次。",
            ),
            Self::GuestModelRouteNoTools => (
                "The assistant could not use its model for this question. No tool ran for this request.",
                "助手无法使用模型回答这个问题。这次请求没有调用任何工具。",
                "助理無法使用模型回答這個問題。這次請求沒有呼叫任何工具。",
            ),
            Self::GuestModelCapacity => (
                "The assistant is at capacity right now. Please try again shortly.",
                "助手当前已满载，请稍后再试。",
                "助理目前已滿載，請稍後再試。",
            ),
            Self::GuestSessionBusy => (
                "The assistant is busy with another request. Please try again shortly.",
                "助手正在处理另一个请求，请稍后再试。",
                "助理正在處理另一個請求，請稍後再試。",
            ),
            Self::RetryAttachmentUnavailable => (
                "I could no longer verify access to the original attachments while waiting. Please check the attachments and send the question again.",
                "等待期间，我无法再确认原附件的访问权限。请检查附件后重新发送问题。",
                "等候期間，我無法再確認原附件的存取權限。請檢查附件後重新傳送問題。",
            ),
            Self::GuestSetupNoTools => (
                "Visitor setup could not finish. No tool ran for this request. The owner has been notified.",
                "访客设置未能完成。这次请求没有调用任何工具，已通知所有者。",
                "訪客設定未能完成。這次請求沒有呼叫任何工具，已通知擁有者。",
            ),
            Self::NoTools => (
                "I couldn't answer this message. No tool ran for this request.",
                "我没能回复这条消息。这次请求没有调用任何工具。",
                "我未能回覆這則訊息。這次請求沒有呼叫任何工具。",
            ),
            Self::OutcomeUnknown => (
                "The previous operation may have taken effect. Check its result before trying again.",
                "上一个操作可能已生效。请先检查结果，再试一次。",
                "上一個操作可能已生效。請先檢查結果，再試一次。",
            ),
            Self::NativeBusy => (
                "NyxBot is busy. Please try again shortly.",
                "NyxBot 正忙，请稍后再试。",
                "NyxBot 正忙，請稍後再試。",
            ),
            Self::RelayBusy => (
                "NyxBot is busy. This message was not started. Please try again shortly.",
                "NyxBot 正忙，还没开始处理这条消息。请稍后再试。",
                "NyxBot 正忙，還沒開始處理這則訊息。請稍後再試。",
            ),
            Self::ImagesUnavailable => (
                "Images are unavailable on this native gateway connection. View them in NyxID.",
                "此网关连接暂不支持图片。请在 NyxID 查看。",
                "此閘道連線暫不支援圖片。請在 NyxID 查看。",
            ),
            Self::Pending => (
                super::assistant_engine_policy::CAPACITY_ACKNOWLEDGEMENT,
                "收到，我现在有点忙，稍后就开始处理。",
                "收到，我現在有點忙，稍後就開始處理。",
            ),
            Self::Checking => (
                super::assistant_checking_notice::TEXT,
                "收到，我查一下再回复你。",
                "收到，我查一下再回覆你。",
            ),
            Self::Unavailable => (
                super::org_channel_service::UNAVAILABLE,
                "助手暂时无法使用，请稍后再试。",
                "助理暫時無法使用，請稍後再試。",
            ),
            Self::GuestBusy => (
                "I'm answering another message right now. Please try again in a moment.",
                "我正在回复另一条消息，请稍后再试。",
                "我正在回覆另一則訊息，請稍後再試。",
            ),
            Self::FollowBusy => (
                "I'm busy right now. Please try again in a moment.",
                "我现在有点忙，请稍后再试。",
                "我現在有點忙，請稍後再試。",
            ),
            Self::StartFailure => (
                "I could not start that. Please try again.",
                "我没能开始处理，请再试一次。",
                "我未能開始處理，請再試一次。",
            ),
            Self::DependencyUnavailable => (
                "The assistant could not start because a dependency is temporarily unavailable. Nothing was done. Please try again shortly.",
                "助手依赖的服务暂时不可用，未能开始处理。尚未执行任何操作，请稍后再试。",
                "助理所需的服務暫時無法使用，未能開始處理。尚未執行任何操作，請稍後再試。",
            ),
            Self::Capacity => (
                "I couldn't accept this message right now. Please try again in a moment.",
                "我暂时无法处理这条消息，请稍后再试。",
                "我暫時無法處理這則訊息，請稍後再試。",
            ),
            Self::ThreadLimit => (
                "I can't follow another thread right now. Please try again later.",
                "我暂时无法再关注新的话题，请稍后再试。",
                "我暫時無法再追蹤新的討論串，請稍後再試。",
            ),
            Self::Stopped => (
                "Stopped following this thread. Its history is still available in NyxID.",
                "已停止关注这个话题。你仍可在 NyxID 查看历史记录。",
                "已停止追蹤這個討論串。你仍可在 NyxID 查看歷史紀錄。",
            ),
            Self::OwnerAbsent => (
                "I'll answer everyone here once my owner has talked to me in this chat.",
                "我的主人在这里和我说过话后，我就可以回复大家了。",
                "我的主人在這裡和我說過話後，我就可以回覆大家了。",
            ),
            Self::SlowDown => (
                "Please wait a moment before sending another message.",
                "请稍等片刻再发送下一条消息。",
                "請稍等片刻再傳送下一則訊息。",
            ),
            Self::MediaUnavailable => (
                super::nyxbot_response_recovery::MEDIA_UNAVAILABLE,
                "此网关连接暂不支持附件。请发送文字，或在 NyxID 上传附件。",
                "此閘道連線暫不支援附件。請傳送文字，或在 NyxID 上傳附件。",
            ),
            Self::QueuedThread => (
                "That question is already queued; I'll answer it in this thread.",
                "这个问题已加入队列，我会在这个话题里回复。",
                "這個問題已排入佇列，我會在這個討論串裡回覆。",
            ),
            Self::WorkingThread => (
                "I'm already working on that question and will answer in this thread.",
                "我正在处理这个问题，会在这个话题里回复。",
                "我正在處理這個問題，會在這個討論串裡回覆。",
            ),
            Self::Working => (
                "I'm still working on that question and will answer it shortly.",
                "我还在处理这个问题，很快就会回复。",
                "我還在處理這個問題，很快就會回覆。",
            ),
            Self::WorkingHereToo => (
                "I'm already working on that question; I'll send the answer here too.",
                "我正在处理这个问题，也会把答案发到这里。",
                "我正在處理這個問題，也會把答案傳到這裡。",
            ),
            Self::WorkingNyxid => (
                "I'm already working on that question; you'll find the answer in NyxID.",
                "我正在处理这个问题，你可以在 NyxID 查看答案。",
                "我正在處理這個問題，你可以在 NyxID 查看答案。",
            ),
            Self::WorkingHere => (
                "I'm already working on that question; I'll send the answer here.",
                "我正在处理这个问题，会把答案发到这里。",
                "我正在處理這個問題，會把答案傳到這裡。",
            ),
            Self::Answered => (
                "I've just answered that question; you'll find it in NyxID.",
                "我刚刚回答了这个问题，你可以在 NyxID 查看答案。",
                "我剛剛回答了這個問題，你可以在 NyxID 查看答案。",
            ),
            Self::QueuedNext => (
                "That question is already queued; I'll answer it next.",
                "这个问题已加入队列，我接下来会回答。",
                "這個問題已排入佇列，我接下來會回答。",
            ),
            Self::VerifyMember => (
                "Verify your organization membership in NyxID to chat with this assistant.",
                "请先在 NyxID 验证组织成员身份，再与助手聊天。",
                "請先在 NyxID 驗證組織成員身分，再與助理聊天。",
            ),
            Self::RestoredMember => (
                "This organization connection was restored. Members must verify their chat account again in NyxID before chatting with this assistant.",
                "组织连接已恢复。成员必须在 NyxID 重新验证聊天账号，才能与助手聊天。",
                "組織連線已恢復。成員必須在 NyxID 重新驗證聊天帳號，才能與助理聊天。",
            ),
        };
        match language.unwrap_or_default() {
            NoticeLanguage::English => en,
            NoticeLanguage::Simplified => cn,
            NoticeLanguage::Traditional => tw,
        }
    }
}

pub(crate) fn capacity(language: Option<NoticeLanguage>, wait_ms: Option<u64>) -> String {
    let (message, minutes, retry) = match language.unwrap_or_default() {
        NoticeLanguage::English => {
            return super::assistant_turn_observability::capacity_message(wait_ms);
        }
        NoticeLanguage::Simplified => ("NyxBot 当前已满载", "分钟", "请稍后再试。"),
        NoticeLanguage::Traditional => ("NyxBot 目前已滿載", "分鐘", "請稍後再試。"),
    };
    let mut waited = String::new();
    if let Some(ms) = wait_ms.filter(|ms| *ms > 0) {
        let seconds = ms.div_ceil(1000);
        waited.push_str("（已等待");
        if seconds >= 60 {
            waited.push_str(&format!(" {} {minutes}", seconds / 60));
        }
        if !seconds.is_multiple_of(60) {
            waited.push_str(&format!(" {} 秒", seconds % 60));
        }
        waited.push('）');
    }
    format!("{message}{waited}。{retry}")
}
