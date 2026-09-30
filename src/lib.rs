//! A blocking client for the Telegram Bot API, with the types of the updates it polls.

use std::fmt;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// How long one attempt at a call may take, on top of the wait a poll names.
const TIMEOUT: Duration = Duration::from_secs(30);
/// A rejection Telegram would answer the same way stands, and the rest are worth asking
/// about again this many times. A retry can post a message twice when the answer to the
/// first was lost.
const ATTEMPTS: u32 = 3;
const BACKOFF: Duration = Duration::from_secs(1);

pub struct Client {
    base: String,
    token: String,
    agent: ureq::Agent,
}

/// Why a call failed once every attempt it was worth was made.
#[derive(Debug)]
pub enum Error {
    /// Telegram refused the request, with the code and the reason it gave.
    Rejected { code: u16, description: String },
    /// The request or its answer never made it across, with the bot token masked.
    Transport(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected { code, description } => write!(f, "{code} {description}"),
            Self::Transport(detail) => f.write_str(detail),
        }
    }
}

impl std::error::Error for Error {}

impl Client {
    #[must_use]
    pub fn new(base: &str, token: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            // Telegram explains a rejection in the body of the failing response.
            .http_status_as_error(false)
            .build()
            .into();
        Self {
            base: base.trim_end_matches('/').to_owned(),
            token: token.to_owned(),
            agent,
        }
    }

    /// Calls `method` with a JSON body.
    ///
    /// # Errors
    ///
    /// Fails as [`Client::request`] does.
    pub fn call<R: DeserializeOwned>(
        &self,
        method: &str,
        body: &impl Serialize,
    ) -> Result<R, Error> {
        self.request(method, TIMEOUT, |request| request.send_json(body))
    }

    /// One long poll for the messages and button presses since `offset`, which Telegram
    /// holds open for up to `timeout` seconds while nothing arrives. Each update is read
    /// on its own, and `trace` logs it first as Telegram sent it.
    ///
    /// # Errors
    ///
    /// Fails as [`Client::request`] does.
    pub fn get_updates(
        &self,
        offset: i64,
        timeout: u64,
        trace: bool,
    ) -> Result<Vec<Update>, Error> {
        let body = serde_json::json!({
            "offset": offset,
            "timeout": timeout,
            "allowed_updates": ["message", "callback_query"],
        });
        let updates: Vec<Value> = self.request(
            "getUpdates",
            TIMEOUT + Duration::from_secs(timeout),
            |request| request.send_json(&body),
        )?;
        Ok(updates
            .into_iter()
            .filter_map(|update| {
                if trace {
                    eprintln!("{update}");
                }
                serde_json::from_value(update)
                    .inspect_err(|error| eprintln!("update: {error}"))
                    .ok()
            })
            .collect())
    }

    /// Makes a request up to `ATTEMPTS` times, each bounded by `timeout`, and logs every
    /// failure it asks again after.
    ///
    /// # Errors
    ///
    /// [`Error::Rejected`] for a request Telegram refused and would refuse again, or
    /// still refused on the last attempt, and [`Error::Transport`] for a request or an
    /// answer that never made it across on the last attempt.
    pub fn request<R: DeserializeOwned>(
        &self,
        method: &str,
        timeout: Duration,
        send: impl Fn(
            ureq::RequestBuilder<ureq::typestate::WithBody>,
        ) -> Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<R, Error> {
        let url = format!("{}/bot{}/{method}", self.base, self.token);
        let mut attempt = 1;
        loop {
            let request = self
                .agent
                .post(&url)
                .config()
                .timeout_global(Some(timeout))
                .build();
            let (error, wait) = match send(request)
                .and_then(|mut response| response.body_mut().read_json::<Answer<R>>())
            {
                Ok(Answer {
                    result: Some(result),
                    ..
                }) => return Ok(result),
                Ok(answer) => {
                    let wait = retry_after(&answer, attempt);
                    let error = Error::Rejected {
                        code: answer.error_code.unwrap_or_default(),
                        description: answer.description.unwrap_or_default(),
                    };
                    (error, wait)
                }
                Err(error) => (
                    // ureq quotes the URL back in some errors, and the token rides in it.
                    Error::Transport(error.to_string().replace(&self.token, "***")),
                    Some(backoff(attempt)),
                ),
            };
            match wait {
                Some(wait) if attempt < ATTEMPTS => {
                    eprintln!("{method}: {error}, retrying in {wait:?}");
                    std::thread::sleep(wait);
                    attempt += 1;
                }
                _ => return Err(error),
            }
        }
    }
}

/// Telegram's answer to a call, which on a rejection names its code and, for a burst,
/// the wait it wants.
#[derive(Deserialize)]
struct Answer<R> {
    result: Option<R>,
    error_code: Option<u16>,
    description: Option<String>,
    parameters: Option<Parameters>,
}

#[derive(Deserialize)]
struct Parameters {
    retry_after: Option<u64>,
}

/// How long before asking again, for a rejection that asking again can answer
/// differently: a burst Telegram wants slowed down, which names the wait it wants, or a
/// failure on its own side.
fn retry_after<R>(answer: &Answer<R>, attempt: u32) -> Option<Duration> {
    match answer.error_code? {
        429 => Some(
            answer
                .parameters
                .as_ref()
                .and_then(|parameters| parameters.retry_after)
                .map_or_else(|| backoff(attempt), Duration::from_secs),
        ),
        500..600 => Some(backoff(attempt)),
        _ => None,
    }
}

/// Doubling, so three attempts span a few seconds rather than a burst of their own.
fn backoff(attempt: u32) -> Duration {
    BACKOFF * 2u32.pow(attempt - 1)
}

/// The id of a message a call sent. A call that sends a message reads its answer as
/// this alone, since an answer that fails to read is asked for again and the message
/// would be sent twice.
#[derive(Debug, Deserialize)]
pub struct Sent {
    #[serde(rename = "message_id")]
    pub id: i64,
}

/// One update of a poll. A message or a press that does not read as the types below is
/// logged and left out, so the update still moves the poll past it.
#[derive(Debug, Deserialize)]
pub struct Update {
    #[serde(rename = "update_id")]
    pub id: i64,
    #[serde(default, deserialize_with = "lenient")]
    pub message: Option<Message>,
    #[serde(default, deserialize_with = "lenient")]
    pub callback_query: Option<CallbackQuery>,
}

/// A field that does not read as its type costs that field alone, with the reason
/// logged.
fn lenient<'de, D: Deserializer<'de>, T: DeserializeOwned>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value)
        .inspect_err(|error| eprintln!("update: {error}"))
        .ok()
        .flatten())
}

/// A message as Telegram delivers it. It serializes under Telegram's names, so it can
/// be handed on as it arrived.
#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    #[serde(rename = "message_id")]
    pub id: i64,
    pub date: i64,
    pub chat: Chat,
    pub from: Option<User>,
    #[serde(rename = "message_thread_id")]
    thread: Option<i64>,
    #[serde(rename = "is_topic_message", default)]
    in_topic: bool,
    pub text: Option<String>,
    #[serde(default)]
    pub entities: Vec<Entity>,
    pub caption: Option<String>,
    #[serde(default)]
    pub caption_entities: Vec<Entity>,
    #[serde(rename = "reply_to_message")]
    pub replied: Option<Box<Message>>,
    #[serde(rename = "forum_topic_created")]
    pub opened: Option<TopicCreated>,
    #[serde(rename = "rich_message")]
    pub rich: Option<Rich>,
    #[serde(rename = "reply_markup")]
    pub keyboard: Option<Keyboard>,
}

impl Message {
    /// The topic the message is in. `message_thread_id` also numbers the reply threads
    /// of a group without topics, where nothing can be sent to one, so only a topic
    /// message's counts.
    #[must_use]
    pub fn topic(&self) -> Option<i64> {
        self.thread.filter(|_| self.in_topic)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Chat {
    pub id: i64,
    pub title: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub username: Option<String>,
}

/// A span of formatting in a text or a caption, counted in UTF-16 code units.
#[derive(Debug, Serialize, Deserialize)]
pub struct Entity {
    #[serde(rename = "type")]
    pub kind: String,
    pub offset: usize,
    pub length: usize,
}

/// The service message that opened a topic.
#[derive(Debug, Serialize, Deserialize)]
pub struct TopicCreated {
    #[serde(rename = "is_name_implicit", default)]
    pub implicit: bool,
}

/// The blocks Telegram rendered a rich message into.
#[derive(Debug, Serialize, Deserialize)]
pub struct Rich {
    #[serde(default)]
    pub blocks: Vec<Block>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Block {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<RichText>,
}

/// Rich text, which Telegram writes as a bare string wherever it carries no formatting
/// of its own, and as an array of such pieces where it does.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RichText {
    Plain(String),
    Pieces(Vec<RichText>),
    Span(Span),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Span {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<Box<RichText>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Keyboard {
    pub inline_keyboard: Vec<Vec<Button>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Button {
    pub text: String,
    pub callback_data: Option<String>,
}

/// A press on an inline button. `message` is the one the button hangs from, and it is
/// absent once that message is too old for the bot to access.
#[derive(Debug, Serialize, Deserialize)]
pub struct CallbackQuery {
    pub id: String,
    pub from: User,
    pub message: Option<Message>,
    pub data: Option<String>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_message_is_in_a_topic_only_when_telegram_says_it_is_a_topic_message() {
        let message = |extra: Value| -> Message {
            let mut message = json!({"message_id": 1, "date": 0, "chat": {"id": 7}});
            message
                .as_object_mut()
                .expect("an object")
                .extend(extra.as_object().expect("an object").clone());
            serde_json::from_value(message).expect("a message")
        };
        assert_eq!(
            message(json!({"message_thread_id": 77, "is_topic_message": true})).topic(),
            Some(77)
        );
        assert_eq!(message(json!({"message_thread_id": 5})).topic(), None);
    }

    #[test]
    fn a_message_that_does_not_read_leaves_its_update_to_move_the_poll() {
        let update: Update = serde_json::from_value(
            json!({"update_id": 5, "message": {"message_id": 1, "text": "no chat"}}),
        )
        .expect("an update");
        assert_eq!(update.id, 5);
        assert!(update.message.is_none());
    }

    /// A server that gives each connection the next of `answers` and counts them.
    fn server(answers: &'static [&'static str]) -> (String, std::thread::JoinHandle<usize>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let base = format!("http://{}", listener.local_addr().expect("an address"));
        let served = std::thread::spawn(move || {
            for (count, answer) in answers.iter().enumerate() {
                let (mut stream, _) = listener.accept().expect("accept");
                // Closing with part of the request unread resets the connection, so the
                // whole request is read before the answer.
                let mut request = Vec::new();
                let mut chunk = [0; 4096];
                while !whole(&request) {
                    let read = stream.read(&mut chunk).expect("a request");
                    request.extend_from_slice(&chunk[..read]);
                }
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{answer}",
                    answer.len()
                )
                .expect("an answer");
                if count + 1 == answers.len() {
                    return answers.len();
                }
            }
            0
        });
        (base, served)
    }

    /// Whether `request` holds its head and as much body as the head announces.
    fn whole(request: &[u8]) -> bool {
        let text = String::from_utf8_lossy(request);
        let Some((head, body)) = text.split_once("\r\n\r\n") else {
            return false;
        };
        let length = head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("a length"))
            })
            .unwrap_or_default();
        body.len() >= length
    }

    #[test]
    fn a_burst_is_asked_about_again_after_the_wait_telegram_names() {
        let (base, served) = server(&[
            r#"{"ok": false, "error_code": 429, "description": "slow down", "parameters": {"retry_after": 0}}"#,
            r#"{"ok": true, "result": true}"#,
        ]);
        let client = Client::new(&base, "123:secret");
        assert!(
            client
                .call::<bool>("deleteMessage", &json!({}))
                .expect("a result")
        );
        assert_eq!(served.join().expect("the server"), 2);
    }

    #[test]
    fn a_refused_request_is_asked_once() {
        let (base, served) = server(&[
            r#"{"ok": false, "error_code": 400, "description": "Bad Request: message is not modified"}"#,
        ]);
        let client = Client::new(&base, "123:secret");
        let error = client
            .call::<bool>("editMessageText", &json!({}))
            .expect_err("a rejection");
        assert!(matches!(
            error,
            Error::Rejected { code: 400, ref description } if description.contains("not modified")
        ));
        assert_eq!(served.join().expect("the server"), 1);
    }

    #[test]
    fn a_transport_error_keeps_the_token_out_of_its_message() {
        let client = Client::new("http://bad host", "123:secret");
        let error = client
            .call::<bool>("getMe", &json!({}))
            .expect_err("an unreachable server");
        assert!(matches!(error, Error::Transport(_)));
        assert!(!error.to_string().contains("secret"), "{error}");
    }

    #[test]
    fn a_rejection_is_asked_about_again_only_when_the_answer_can_differ() {
        let rejection = |answer: Value| {
            let answer: Answer<bool> = serde_json::from_value(answer).expect("an answer");
            retry_after(&answer, 1)
        };
        assert_eq!(
            rejection(json!({"ok": false, "error_code": 429, "parameters": {"retry_after": 7}})),
            Some(Duration::from_secs(7))
        );
        assert_eq!(
            rejection(json!({"ok": false, "error_code": 429})),
            Some(BACKOFF)
        );
        assert_eq!(
            rejection(json!({"ok": false, "error_code": 502})),
            Some(BACKOFF)
        );
        assert_eq!(rejection(json!({"ok": false, "error_code": 400})), None);
        assert_eq!(rejection(json!({"ok": false})), None);
        assert_eq!(backoff(3), BACKOFF * 4);
    }
}
