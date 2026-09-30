//! Bodies of Bot API methods, each naming its method and the type its result reads as.

use std::ops::Not;

use serde::Serialize;
use serde::de::{DeserializeOwned, IgnoredAny};

use crate::{Keyboard, Sent, User};

/// A body [`crate::Client::send`] posts as JSON to the method `NAME`.
pub trait Method: Serialize {
    const NAME: &'static str;
    type Response: DeserializeOwned;
}

macro_rules! method {
    ($body:ty, $name:literal, $response:ty) => {
        impl Method for $body {
            const NAME: &'static str = $name;
            type Response = $response;
        }
    };
}

method!(GetMe, "getMe", User);
method!(SetMyCommands<'_>, "setMyCommands", bool);
method!(SendMessage<'_>, "sendMessage", Sent);
method!(SendRichMessage<'_>, "sendRichMessage", Sent);
// The result is the edited message, which no caller reads.
method!(EditMessageText<'_>, "editMessageText", IgnoredAny);
method!(DeleteMessage, "deleteMessage", bool);
method!(SetMessageReaction<'_>, "setMessageReaction", bool);
method!(AnswerCallbackQuery<'_>, "answerCallbackQuery", bool);

#[derive(Serialize)]
pub struct GetMe;

#[derive(Serialize)]
pub struct SetMyCommands<'a> {
    pub commands: &'a [BotCommand<'a>],
    pub scope: CommandScope,
}

#[derive(Serialize)]
pub struct BotCommand<'a> {
    pub command: &'a str,
    pub description: &'a str,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommandScope {
    Chat { chat_id: i64 },
    ChatMember { chat_id: i64, user_id: i64 },
}

#[derive(Serialize)]
pub struct SendMessage<'a> {
    pub chat_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_thread_id: Option<i64>,
    pub text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<ParseMode>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link_preview_options: Option<LinkPreviewOptions>,
    #[serde(skip_serializing_if = "Not::not")]
    pub disable_notification: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_parameters: Option<ReplyParameters>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_markup: Option<ReplyMarkup<'a>>,
}

impl<'a> SendMessage<'a> {
    #[must_use]
    pub fn new(chat_id: i64, message_thread_id: Option<i64>, text: &'a str) -> Self {
        Self {
            chat_id,
            message_thread_id,
            text,
            parse_mode: None,
            link_preview_options: None,
            disable_notification: false,
            reply_parameters: None,
            reply_markup: None,
        }
    }
}

#[derive(Serialize)]
pub struct SendRichMessage<'a> {
    pub chat_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_thread_id: Option<i64>,
    pub rich_message: RichInput<'a>,
    #[serde(skip_serializing_if = "Not::not")]
    pub disable_notification: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_parameters: Option<ReplyParameters>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_markup: Option<ReplyMarkup<'a>>,
}

impl<'a> SendRichMessage<'a> {
    #[must_use]
    pub fn new(chat_id: i64, message_thread_id: Option<i64>, rich_message: RichInput<'a>) -> Self {
        Self {
            chat_id,
            message_thread_id,
            rich_message,
            disable_notification: false,
            reply_parameters: None,
            reply_markup: None,
        }
    }
}

#[derive(Serialize)]
pub struct EditMessageText<'a> {
    pub chat_id: i64,
    pub message_id: i64,
    #[serde(flatten)]
    pub content: Content<'a>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_markup: Option<ReplyMarkup<'a>>,
}

/// What an edited message shows, which is a plain text or a rich message.
#[derive(Serialize)]
pub enum Content<'a> {
    #[serde(rename = "text")]
    Text(&'a str),
    #[serde(rename = "rich_message")]
    Rich(RichInput<'a>),
}

#[derive(Serialize)]
pub struct DeleteMessage {
    pub chat_id: i64,
    pub message_id: i64,
}

#[derive(Serialize)]
pub struct SetMessageReaction<'a> {
    pub chat_id: i64,
    pub message_id: i64,
    pub reaction: &'a [Reaction<'a>],
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reaction<'a> {
    Emoji { emoji: &'a str },
}

#[derive(Serialize)]
pub struct AnswerCallbackQuery<'a> {
    pub callback_query_id: &'a str,
}

#[derive(Clone, Copy, Serialize)]
pub enum ParseMode {
    #[serde(rename = "HTML")]
    Html,
}

#[derive(Serialize)]
pub struct LinkPreviewOptions {
    pub is_disabled: bool,
}

#[derive(Serialize)]
pub struct ReplyParameters {
    pub message_id: i64,
    pub allow_sending_without_reply: bool,
}

/// The source of a rich message, in one of the markups Telegram renders it from.
#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RichInput<'a> {
    Html(&'a str),
    Markdown(&'a str),
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum ReplyMarkup<'a> {
    Keyboard(Keyboard),
    ForceReply {
        force_reply: bool,
        input_field_placeholder: &'a str,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_body_carries_its_content_under_telegram_names_and_leaves_unset_options_out() {
        let edit = EditMessageText {
            chat_id: 7,
            message_id: 9,
            content: Content::Rich(RichInput::Html("<p>hi</p>")),
            reply_markup: None,
        };
        assert_eq!(
            serde_json::to_value(&edit).expect("a body"),
            json!({"chat_id": 7, "message_id": 9, "rich_message": {"html": "<p>hi</p>"}})
        );
        let send = SendMessage {
            parse_mode: Some(ParseMode::Html),
            disable_notification: true,
            ..SendMessage::new(7, Some(77), "hi")
        };
        assert_eq!(
            serde_json::to_value(&send).expect("a body"),
            json!({
                "chat_id": 7,
                "message_thread_id": 77,
                "text": "hi",
                "parse_mode": "HTML",
                "disable_notification": true,
            })
        );
        let scope = CommandScope::ChatMember {
            chat_id: -1,
            user_id: 2,
        };
        assert_eq!(
            serde_json::to_value(&scope).expect("a scope"),
            json!({"type": "chat_member", "chat_id": -1, "user_id": 2})
        );
    }
}
