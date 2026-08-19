// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! One HTTP client for every provider that speaks OpenAI's chat-completions
//! dialect.
//!
//! `docs/ARCHITECTURE.md` lists model access as "OpenAI-compatible providers,
//! OpenRouter, Ollama" and that is not three integrations, it is one. OpenAI,
//! Groq, Cerebras, OpenRouter, Together, Ollama and LM Studio all serve
//! `POST {base}/chat/completions` with an SSE body, so base URL, model id and
//! API key are the only things that vary — and all three are injected.
//!
//! ## Keys
//!
//! The key is taken as an [`ApiKey`], which has no `Display`, redacts its
//! `Debug`, and is read exactly once: when the `Authorization` header is built
//! at construction time. That header value is then marked *sensitive*, so `http`
//! prints it as `Sensitive` and HPACK never puts it in an HTTP/2 dynamic table
//! where a trace could pick it up. Nothing in this module formats the key into a
//! string, an error or a URL.
//!
//! Transport errors have reqwest's URL stripped off them for the same reason. A
//! base URL is user input and some providers put credentials in a query string;
//! the provider id says which endpoint failed without repeating whatever the
//! user pasted.
//!
//! ## Timeouts
//!
//! There is deliberately no total request timeout. A long answer is not a
//! failure, and a deadline that kills one is worse than none. The read timeout
//! resets on every chunk instead, which catches the case that actually matters —
//! a connection that has gone quiet — without capping how long a healthy
//! generation may run.

use std::sync::Arc;
use std::time::Duration;

use futures_util::TryStreamExt;
use reqwest::header::{HeaderValue, ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, Url};

use super::cancel::CancellationToken;
use super::sse::{self, ByteStream, DeltaStream};
use super::types::{ApiKey, ChatRequest, ProviderError};
use super::wire::{self, CompletionRequest};
use super::Provider;

/// Appended to whatever base URL the user configured.
const COMPLETIONS_PATH: [&str; 2] = ["chat", "completions"];

/// Sent so a gateway that requires a user agent does not reject the request,
/// and so a provider's own logs name the app the user actually chose to run.
const USER_AGENT: &str = concat!("skia/", env!("CARGO_PKG_VERSION"));

/// How long to wait for a connection before giving up.
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a connection may go silent mid-answer before it counts as stalled.
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(60);

/// Keeping a connection warm removes a TLS handshake from time to first token,
/// which the latency budget puts at 0.3–0.9 s for the whole model round trip.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(90);

/// Everything needed to reach one OpenAI-compatible endpoint.
///
/// Safe to `Debug`: the key inside it redacts itself.
#[derive(Debug, Clone)]
pub struct OpenAiConfig {
    /// How this provider is named in errors and in the registry. Free-form, and
    /// the user's own label — `groq-fast`, `work-openrouter`, `laptop-ollama`.
    pub id: String,
    /// Everything up to but not including `/chat/completions`, e.g.
    /// `https://api.openai.com/v1` or `http://127.0.0.1:11434/v1`.
    pub base_url: String,
    /// The model this provider entry stands for. A request may override it.
    pub model: String,
    /// `None` for Ollama and LM Studio, which want no credential at all.
    pub api_key: Option<ApiKey>,
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
}

impl OpenAiConfig {
    /// A configuration with no key and the default timeouts.
    pub fn new(
        id: impl Into<String>,
        base_url: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            base_url: base_url.into(),
            model: model.into(),
            api_key: None,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            read_timeout: DEFAULT_READ_TIMEOUT,
        }
    }

    /// Attach the credential, which belongs in the OS keychain until this point.
    pub fn with_api_key(mut self, api_key: ApiKey) -> Self {
        self.api_key = Some(api_key);
        self
    }

    pub fn with_timeouts(mut self, connect: Duration, read: Duration) -> Self {
        self.connect_timeout = connect;
        self.read_timeout = read;
        self
    }
}

/// A streaming chat client for one configured endpoint.
///
/// Cloning is cheap — the HTTP client is reference counted internally — which is
/// what lets each request take its own copy into the stream it returns.
#[derive(Debug, Clone)]
pub struct OpenAiCompatible {
    id: Arc<str>,
    endpoint: Url,
    model: String,
    /// Pre-built at construction so a malformed key fails immediately rather
    /// than on the first answer a user waits for. Marked sensitive, so this
    /// struct's own `Debug` cannot leak it either.
    authorization: Option<HeaderValue>,
    client: Client,
}

impl OpenAiCompatible {
    /// Validate the configuration and build a client for it.
    ///
    /// No network traffic happens here: this resolves nothing, connects to
    /// nothing, and does not check that the endpoint exists. A registry can
    /// therefore be built at startup from whatever is in the keychain without
    /// making the app wait on a provider that may not even be needed.
    pub fn new(config: OpenAiConfig) -> Result<Self, ProviderError> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.read_timeout)
            .pool_idle_timeout(POOL_IDLE_TIMEOUT)
            .build()
            .map_err(|source| ProviderError::Config {
                provider: config.id.clone(),
                detail: format!(
                    "an HTTPS client could not be built: {}",
                    source.without_url()
                ),
            })?;

        Self::with_client(config, client)
    }

    /// Build a provider that shares an existing [`Client`].
    ///
    /// Worth doing when several providers point at the same host — a shared
    /// connection pool means the second one does not pay for a handshake.
    pub fn with_client(config: OpenAiConfig, client: Client) -> Result<Self, ProviderError> {
        if config.id.trim().is_empty() {
            return Err(ProviderError::Config {
                provider: "an unnamed provider".to_string(),
                detail: "a provider needs an id, because errors are reported against it"
                    .to_string(),
            });
        }

        if config.model.trim().is_empty() {
            return Err(ProviderError::Config {
                provider: config.id.clone(),
                detail: "no model is configured, and an OpenAI-compatible endpoint requires one"
                    .to_string(),
            });
        }

        Ok(Self {
            endpoint: completions_endpoint(&config.id, &config.base_url)?,
            authorization: authorization_header(&config.id, config.api_key.as_ref())?,
            id: Arc::from(config.id),
            model: config.model,
            client,
        })
    }

    /// The URL that will actually be posted to.
    ///
    /// Exposed because "which endpoint is this really hitting?" is the first
    /// question a misconfigured provider raises, and the developer panel in the
    /// roadmap needs to answer it.
    pub fn endpoint(&self) -> &str {
        self.endpoint.as_str()
    }

    /// Send the request and hand back its raw body.
    ///
    /// Consumes a clone of the provider so the returned stream owns everything
    /// it needs and nothing borrows across the await.
    async fn open(self, request: ChatRequest) -> Result<ByteStream, ProviderError> {
        request.validate()?;

        // An empty model means "the one this provider is configured with", which
        // is what a caller routing by role has.
        let model = if request.model.trim().is_empty() {
            self.model.as_str()
        } else {
            request.model.as_str()
        };

        // Encoded here rather than with `RequestBuilder::json`, so a
        // serialisation failure is reported as one instead of as a generic
        // client-builder error.
        let payload = serde_json::to_vec(&CompletionRequest {
            model,
            messages: &request.messages,
            stream: true,
            max_tokens: request.max_tokens,
            temperature: request.temperature,
        })
        .map_err(|source| ProviderError::Encode {
            provider: self.id.to_string(),
            source,
        })?;

        let mut post = self
            .client
            .post(self.endpoint.clone())
            .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
            .header(ACCEPT, HeaderValue::from_static("text/event-stream"))
            .body(payload);

        if let Some(authorization) = self.authorization.clone() {
            post = post.header(AUTHORIZATION, authorization);
        }

        let response = post
            .send()
            .await
            .map_err(|source| ProviderError::Transport {
                provider: self.id.to_string(),
                source: source.without_url(),
            })?;

        let status = response.status();
        if !status.is_success() {
            // The body is where the useful sentence is: which key was wrong,
            // which model does not exist, how long to back off for.
            let message = match response.bytes().await {
                Ok(body) => wire::describe_error_body(&body),
                Err(source) => format!(
                    "the error body could not be read either: {}",
                    source.without_url()
                ),
            };

            return Err(ProviderError::Http {
                provider: self.id.to_string(),
                status: status.as_u16(),
                message,
            });
        }

        let provider = Arc::clone(&self.id);
        Ok(Box::pin(
            response
                .bytes_stream()
                // Copied out of `Bytes` so the decoder never has to name a type
                // from a crate this one does not depend on directly. A chunk is
                // a few hundred bytes against a network round trip.
                .map_ok(|chunk| chunk.to_vec())
                .map_err(move |source| ProviderError::Transport {
                    provider: provider.to_string(),
                    source: source.without_url(),
                }),
        ))
    }
}

impl Provider for OpenAiCompatible {
    fn id(&self) -> &str {
        &self.id
    }

    fn model(&self) -> &str {
        &self.model
    }

    /// Nothing is sent until the returned stream is polled.
    ///
    /// That is deliberate. A speculative generation that gets cancelled before
    /// anyone reads it never reaches the network at all, which on a live call is
    /// the difference between wasted tokens and none.
    fn stream_chat(&self, request: ChatRequest, cancel: CancellationToken) -> DeltaStream {
        let provider = Arc::clone(&self.id);
        let connection = self.clone();

        // `once` defers the request to the first poll; `try_flatten` splices the
        // response body in behind it, so a failure to even send arrives as the
        // stream's first item rather than as a separate error channel.
        let body =
            futures_util::stream::once(async move { connection.open(request).await }).try_flatten();

        sse::delta_stream(Box::pin(body), provider, cancel)
    }
}

/// Derive `{base}/chat/completions`, tolerating a trailing slash.
///
/// Built through the URL's path segments rather than string concatenation, so a
/// base URL with a trailing slash, a sub-path or a port lands in the right place
/// instead of producing something like `https://host/v1chat/completions`.
fn completions_endpoint(provider: &str, base_url: &str) -> Result<Url, ProviderError> {
    let trimmed = base_url.trim();
    if trimmed.is_empty() {
        return Err(ProviderError::Config {
            provider: provider.to_string(),
            detail: "no base URL is set, so there is nowhere to send the request".to_string(),
        });
    }

    let mut endpoint = Url::parse(trimmed).map_err(|source| ProviderError::Config {
        provider: provider.to_string(),
        detail: format!("the base URL {trimmed:?} is not a URL: {source}"),
    })?;

    // Anything else — `file:`, `data:`, a bare hostname parsed as a scheme —
    // cannot reach a model provider, and silently trying would be worse than
    // saying so.
    if !matches!(endpoint.scheme(), "http" | "https") {
        return Err(ProviderError::Config {
            provider: provider.to_string(),
            detail: format!(
                "the base URL uses the {:?} scheme; a model provider is reached over \
                 http or https",
                endpoint.scheme()
            ),
        });
    }

    {
        let Ok(mut segments) = endpoint.path_segments_mut() else {
            return Err(ProviderError::Config {
                provider: provider.to_string(),
                detail: format!("the base URL {trimmed:?} has no path to append to"),
            });
        };
        // Drops the empty segment a trailing slash leaves behind, so
        // `.../v1` and `.../v1/` produce the same endpoint.
        segments.pop_if_empty();
        for segment in COMPLETIONS_PATH {
            segments.push(segment);
        }
    }

    Ok(endpoint)
}

/// Turn a key into a sensitive `Authorization` header, or nothing.
fn authorization_header(
    provider: &str,
    api_key: Option<&ApiKey>,
) -> Result<Option<HeaderValue>, ProviderError> {
    let Some(api_key) = api_key else {
        return Ok(None);
    };

    let key = api_key.expose_secret().trim();
    if key.is_empty() {
        return Err(ProviderError::Config {
            provider: provider.to_string(),
            detail: "the API key is empty; leave it unset for a provider that needs none"
                .to_string(),
        });
    }

    let mut value = HeaderValue::from_str(&format!("Bearer {key}")).map_err(|source| {
        ProviderError::Config {
            // The key itself is never in this message. A stray newline from a
            // copy-paste is by far the most common cause, so it is named.
            provider: provider.to_string(),
            detail: format!(
                "the API key cannot go in an HTTP header, most likely because a line \
                 break or a control character was copied along with it ({source})"
            ),
        }
    })?;

    // From here on `http` prints this as `Sensitive`, and HPACK will not put it
    // in a dynamic table.
    value.set_sensitive(true);

    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;

    use futures_util::StreamExt;

    use super::*;
    use crate::providers::Delta;

    /// A one-shot HTTP server on a loopback port.
    ///
    /// Real sockets, a real reqwest client and a real hyper response parse, with
    /// no mock-HTTP dependency: the only thing faked is the provider at the far
    /// end. It answers exactly one request and then closes.
    struct TestServer {
        base_url: String,
        request: mpsc::Receiver<Vec<u8>>,
    }

    impl TestServer {
        /// Start a server that reads one request, publishes it, then lets
        /// `respond` write whatever it likes to the socket.
        fn start<F>(respond: F) -> Self
        where
            F: FnOnce(&mut TcpStream) + Send + 'static,
        {
            let listener =
                TcpListener::bind("127.0.0.1:0").expect("a loopback port must be available");
            let port = listener
                .local_addr()
                .expect("the bound address must be readable")
                .port();
            let (sender, request) = mpsc::channel();

            // Deliberately not joined on drop. One test leaves the server
            // sleeping to prove cancellation does not wait for it, and joining
            // would make the test wait for exactly what it is testing.
            thread::spawn(move || {
                let (mut socket, _) = listener.accept().expect("the client must connect");
                socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .expect("a read timeout must be settable");

                let raw = read_request(&mut socket);
                sender.send(raw).expect("the test must still be listening");

                respond(&mut socket);
                let _ = socket.flush();
                let _ = socket.shutdown(Shutdown::Both);
            });

            Self {
                base_url: format!("http://127.0.0.1:{port}/v1"),
                request,
            }
        }

        /// The request the client sent, headers and body, lowercased so header
        /// name casing cannot make an assertion flaky.
        fn captured_request(&self) -> String {
            let raw = self
                .request
                .recv_timeout(Duration::from_secs(10))
                .expect("the client must have sent a request");
            String::from_utf8_lossy(&raw).to_lowercase()
        }
    }

    /// Read one HTTP request: headers a byte at a time, then exactly the body.
    ///
    /// One byte at a time is slow and does not matter; what matters is not
    /// reading past the body and blocking forever.
    fn read_request(socket: &mut TcpStream) -> Vec<u8> {
        let mut raw = Vec::new();
        let mut byte = [0u8; 1];

        while !raw.ends_with(b"\r\n\r\n") {
            match socket.read(&mut byte) {
                Ok(0) => return raw,
                Ok(_) => raw.push(byte[0]),
                Err(error) => panic!("reading the request failed: {error}"),
            }
        }

        let length = content_length(&raw);
        if length > 0 {
            let mut body = vec![0u8; length];
            socket
                .read_exact(&mut body)
                .expect("the declared body length must arrive");
            raw.extend_from_slice(&body);
        }

        raw
    }

    fn content_length(headers: &[u8]) -> usize {
        String::from_utf8_lossy(headers)
            .to_lowercase()
            .lines()
            .find_map(|line| line.strip_prefix("content-length:").map(str::to_string))
            .map(|value| {
                value
                    .trim()
                    .parse()
                    .expect("content-length must be a number")
            })
            .unwrap_or(0)
    }

    /// One well-formed completion chunk as an SSE event.
    fn event(content: &str) -> String {
        format!(
            "data: {{\"id\":\"c\",\"object\":\"chat.completion.chunk\",\
             \"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{content}\"}},\
             \"finish_reason\":null}}]}}\n\n"
        )
    }

    /// A complete `200 text/event-stream` response.
    fn sse_response(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\n\
             Content-Type: text/event-stream\r\n\
             Cache-Control: no-cache\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            body.len()
        )
    }

    /// A response with any status and body.
    fn response(status: &str, content_type: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\n\
             Content-Type: {content_type}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\
             \r\n\
             {body}",
            body.len()
        )
    }

    fn provider(base_url: &str) -> OpenAiCompatible {
        OpenAiCompatible::new(
            OpenAiConfig::new("test-provider", base_url, "canned-model")
                .with_api_key(ApiKey::new("sk-test-not-a-real-key")),
        )
        .expect("the provider must build")
    }

    /// Drain a delta stream into text.
    async fn text_of(mut stream: DeltaStream) -> Result<String, ProviderError> {
        let mut text = String::new();
        while let Some(delta) = stream.next().await {
            text.push_str(&delta?.content);
        }
        Ok(text)
    }

    #[tokio::test]
    async fn a_canned_sse_response_is_streamed_and_decoded() {
        let body = format!(
            "{}{}{}data: [DONE]\n\n",
            event("Two"),
            event(" audio"),
            event(" streams")
        );
        let server = TestServer::start(move |socket| {
            let raw = sse_response(&body);
            // Written in two goes with a flush between, so the client really
            // does have to decode across separate reads.
            let (head, tail) = raw.as_bytes().split_at(raw.len() / 2);
            socket.write_all(head).expect("the head must be writable");
            socket.flush().expect("the head must flush");
            thread::sleep(Duration::from_millis(20));
            socket.write_all(tail).expect("the tail must be writable");
        });

        let provider = provider(&server.base_url);
        let request = ChatRequest {
            messages: vec![
                crate::providers::ChatMessage::system("be brief"),
                crate::providers::ChatMessage::user("what about the audio?"),
            ],
            max_tokens: Some(32),
            temperature: Some(0.2),
            ..ChatRequest::default()
        };

        let answer = text_of(provider.stream_chat(request, CancellationToken::new()))
            .await
            .expect("a well-formed stream must decode");
        assert_eq!(answer, "Two audio streams");

        let sent = server.captured_request();
        assert!(
            sent.starts_with("post /v1/chat/completions http/1.1"),
            "the endpoint is derived from the base URL: {sent}"
        );
        assert!(sent.contains("authorization: bearer sk-test-not-a-real-key"));
        assert!(sent.contains("accept: text/event-stream"));
        assert!(sent.contains("content-type: application/json"));
        assert!(sent.contains("user-agent: skia/"));
        assert!(sent.contains("\"stream\":true"), "{sent}");
        assert!(sent.contains("\"model\":\"canned-model\""), "{sent}");
        assert!(sent.contains("\"max_tokens\":32"), "{sent}");
        assert!(sent.contains("\"temperature\":0.2"), "{sent}");
        assert!(
            sent.contains("\"role\":\"system\"") && sent.contains("\"role\":\"user\""),
            "both turns must be sent: {sent}"
        );
        assert!(
            !sent.contains("maxtokens"),
            "the wire shape is snake_case: {sent}"
        );
    }

    #[tokio::test]
    async fn a_model_on_the_request_overrides_the_configured_one() {
        let server = TestServer::start(|socket| {
            let body = format!("{}data: [DONE]\n\n", event("ok"));
            socket
                .write_all(sse_response(&body).as_bytes())
                .expect("the response must be writable");
        });

        let provider = provider(&server.base_url);
        let answer = text_of(provider.stream_chat(
            ChatRequest {
                model: "llama-3.3-70b".to_string(),
                ..ChatRequest::user("hi")
            },
            CancellationToken::new(),
        ))
        .await
        .expect("the stream must decode");

        assert_eq!(answer, "ok");
        let sent = server.captured_request();
        assert!(sent.contains("\"model\":\"llama-3.3-70b\""), "{sent}");
        assert!(!sent.contains("canned-model"), "{sent}");
    }

    #[tokio::test]
    async fn a_provider_with_no_key_sends_no_authorization_header() {
        let server = TestServer::start(|socket| {
            let body = format!("{}data: [DONE]\n\n", event("local"));
            socket
                .write_all(sse_response(&body).as_bytes())
                .expect("the response must be writable");
        });

        // What Ollama and LM Studio look like: a local base URL and no key.
        let provider = OpenAiCompatible::new(OpenAiConfig::new(
            "laptop-ollama",
            &server.base_url,
            "llama3.2",
        ))
        .expect("a keyless provider must build");

        assert_eq!(
            text_of(provider.stream_chat(ChatRequest::user("hi"), CancellationToken::new()))
                .await
                .expect("the stream must decode"),
            "local"
        );
        assert!(
            !server.captured_request().contains("authorization"),
            "a provider with no key must not invent one"
        );
    }

    #[tokio::test]
    async fn an_http_error_reports_the_status_and_the_provider_message() {
        let server = TestServer::start(|socket| {
            let body = r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#;
            socket
                .write_all(response("401 Unauthorized", "application/json", body).as_bytes())
                .expect("the response must be writable");
        });

        let error = text_of(
            provider(&server.base_url)
                .stream_chat(ChatRequest::user("hi"), CancellationToken::new()),
        )
        .await
        .expect_err("a 401 is not an answer");

        match error {
            ProviderError::Http {
                ref provider,
                status,
                ref message,
            } => {
                assert_eq!(provider, "test-provider");
                assert_eq!(status, 401);
                assert_eq!(
                    message,
                    "Incorrect API key provided (invalid_request_error)"
                );
            }
            other => panic!("expected an HTTP error, got {other}"),
        }

        assert!(
            !error.to_string().contains("sk-test"),
            "an error must never quote the key back: {error}"
        );
    }

    #[tokio::test]
    async fn an_error_body_that_is_not_json_is_still_reported() {
        let server = TestServer::start(|socket| {
            socket
                .write_all(
                    response(
                        "502 Bad Gateway",
                        "text/html",
                        "<html><body>upstream unavailable</body></html>",
                    )
                    .as_bytes(),
                )
                .expect("the response must be writable");
        });

        let error = text_of(
            provider(&server.base_url)
                .stream_chat(ChatRequest::user("hi"), CancellationToken::new()),
        )
        .await
        .expect_err("a 502 from a proxy is not an answer");

        assert!(error.to_string().contains("502"), "{error}");
        assert!(
            error.to_string().contains("upstream unavailable"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_malformed_chunk_over_http_is_an_error_and_not_a_short_answer() {
        let server = TestServer::start(|socket| {
            let body = format!("{}data: {{\"choices\":[oops]}}\n\n", event("good"));
            socket
                .write_all(sse_response(&body).as_bytes())
                .expect("the response must be writable");
        });

        let mut stream = provider(&server.base_url)
            .stream_chat(ChatRequest::user("hi"), CancellationToken::new());

        assert_eq!(
            stream.next().await.unwrap().unwrap(),
            Delta::new("good"),
            "the deltas before the breakage still arrive"
        );
        let error = stream
            .next()
            .await
            .expect("the failure must be reported")
            .expect_err("a broken chunk must not be skipped");
        assert!(matches!(error, ProviderError::Json { .. }), "{error}");
        assert!(stream.next().await.is_none());
    }

    #[tokio::test]
    async fn a_two_hundred_that_is_not_an_event_stream_is_reported() {
        let server = TestServer::start(|socket| {
            // A gateway that ignored `stream: true` and answered in one go.
            let body = r#"{"choices":[{"message":{"role":"assistant","content":"hello"}}]}"#;
            socket
                .write_all(response("200 OK", "application/json", body).as_bytes())
                .expect("the response must be writable");
        });

        let error = text_of(
            provider(&server.base_url)
                .stream_chat(ChatRequest::user("hi"), CancellationToken::new()),
        )
        .await
        .expect_err("a non-streaming body must not read as an empty answer");

        assert!(
            error.to_string().contains("no server-sent events"),
            "{error}"
        );
    }

    #[tokio::test]
    async fn cancelling_ends_a_live_http_stream_without_waiting_for_the_provider() {
        let server = TestServer::start(|socket| {
            // A Content-Length far beyond what is sent, so the client stays
            // parked waiting for the rest of a response that never comes.
            let head = "HTTP/1.1 200 OK\r\n\
                        Content-Type: text/event-stream\r\n\
                        Content-Length: 100000\r\n\
                        \r\n";
            socket
                .write_all(head.as_bytes())
                .expect("the head must be writable");
            let body = format!("{}{}", event("first"), event("second"));
            socket
                .write_all(body.as_bytes())
                .expect("the body must be writable");
            socket.flush().expect("the body must flush");

            // The provider then goes quiet for far longer than the test will.
            thread::sleep(Duration::from_secs(5));
        });

        let cancel = CancellationToken::new();
        let mut stream =
            provider(&server.base_url).stream_chat(ChatRequest::user("hi"), cancel.clone());

        assert_eq!(stream.next().await.unwrap().unwrap(), Delta::new("first"));
        assert_eq!(stream.next().await.unwrap().unwrap(), Delta::new("second"));

        cancel.cancel();

        let ended = tokio::time::timeout(Duration::from_secs(2), stream.next())
            .await
            .expect("barge-in must not wait on a silent provider");
        assert!(
            ended.is_none(),
            "a cancelled stream must end rather than yield again"
        );
    }

    #[tokio::test]
    async fn an_unreachable_endpoint_is_a_transport_error() {
        // Bound and immediately dropped, so the port is almost certainly free
        // and nothing is listening on it.
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("a port must be available");
            listener.local_addr().expect("an address").port()
        };

        let error = text_of(
            provider(&format!("http://127.0.0.1:{port}/v1"))
                .stream_chat(ChatRequest::user("hi"), CancellationToken::new()),
        )
        .await
        .expect_err("a closed port cannot answer");

        match error {
            ProviderError::Transport { ref provider, .. } => assert_eq!(provider, "test-provider"),
            other => panic!("expected a transport error, got {other}"),
        }
    }

    #[tokio::test]
    async fn an_invalid_request_never_reaches_the_network() {
        // No server at all: if validation did not run first, this would fail as
        // a transport error instead.
        let provider = provider("http://127.0.0.1:1/v1");
        let mut stream = provider.stream_chat(ChatRequest::default(), CancellationToken::new());

        let error = stream
            .next()
            .await
            .expect("the failure must be an item")
            .expect_err("a request with no messages is not sendable");
        assert!(
            matches!(error, ProviderError::InvalidRequest { .. }),
            "{error}"
        );
    }

    #[test]
    fn the_endpoint_is_derived_from_the_base_url() {
        for base in [
            "https://api.openai.com/v1",
            "https://api.openai.com/v1/",
            "  https://api.openai.com/v1  ",
        ] {
            assert_eq!(
                completions_endpoint("p", base).unwrap().as_str(),
                "https://api.openai.com/v1/chat/completions",
                "base {base:?} produced the wrong endpoint"
            );
        }

        assert_eq!(
            completions_endpoint("p", "http://127.0.0.1:11434/v1")
                .unwrap()
                .as_str(),
            "http://127.0.0.1:11434/v1/chat/completions",
            "a local Ollama endpoint must work over plain http"
        );
        assert_eq!(
            completions_endpoint("p", "https://gateway.example.com/proxy/openai/v1")
                .unwrap()
                .as_str(),
            "https://gateway.example.com/proxy/openai/v1/chat/completions",
            "a base URL with a sub-path must keep it"
        );
    }

    #[test]
    fn a_base_url_that_cannot_work_is_rejected_at_construction() {
        for (base, expected) in [
            ("", "no base URL"),
            ("   ", "no base URL"),
            ("api.openai.com/v1", "is not a URL"),
            ("file:///etc/passwd", "scheme"),
            ("data:text/plain,hi", "scheme"),
        ] {
            let error = completions_endpoint("p", base)
                .expect_err("an unusable base URL must not be accepted");
            assert!(
                error.to_string().contains(expected),
                "base {base:?} gave the wrong error: {error}"
            );
        }
    }

    #[test]
    fn an_incomplete_configuration_is_rejected() {
        let client = Client::new();

        let error = OpenAiCompatible::with_client(
            OpenAiConfig::new("", "https://api.openai.com/v1", "gpt-4o-mini"),
            client.clone(),
        )
        .expect_err("a provider with no id cannot be reported against");
        assert!(error.to_string().contains("needs an id"), "{error}");

        let error = OpenAiCompatible::with_client(
            OpenAiConfig::new("p", "https://api.openai.com/v1", "  "),
            client.clone(),
        )
        .expect_err("a provider with no model cannot send a request");
        assert!(error.to_string().contains("no model"), "{error}");

        let error = OpenAiCompatible::with_client(
            OpenAiConfig::new("p", "https://api.openai.com/v1", "m")
                .with_api_key(ApiKey::new("   ")),
            client,
        )
        .expect_err("an empty key is a misconfiguration, not an absent key");
        assert!(error.to_string().contains("empty"), "{error}");
    }

    #[test]
    fn a_key_with_a_newline_in_it_is_rejected_without_being_quoted() {
        let error = authorization_header("p", Some(&ApiKey::new("sk-line\nOne: injected")))
            .expect_err("a header value cannot contain a line break");

        let text = error.to_string();
        assert!(text.contains("line break"), "{text}");
        assert!(
            !text.contains("sk-line") && !text.contains("injected"),
            "the rejected key must not appear in the error: {text}"
        );
    }

    #[test]
    fn the_authorization_header_is_marked_sensitive() {
        let header = authorization_header("p", Some(&ApiKey::new(" sk-secret ")))
            .expect("a plain key must be accepted")
            .expect("a key was given, so a header must come back");

        assert!(header.is_sensitive(), "an API key header must be sensitive");
        assert_eq!(
            format!("{header:?}"),
            "Sensitive",
            "http must refuse to print it"
        );
        assert_eq!(
            header.to_str().expect("the header is ASCII"),
            "Bearer sk-secret",
            "surrounding whitespace from a paste is trimmed"
        );

        assert!(authorization_header("p", None)
            .expect("no key is valid")
            .is_none());
    }

    #[test]
    fn the_provider_never_debug_prints_its_key() {
        let provider = provider("https://api.openai.com/v1");
        let printed = format!("{provider:#?}");

        assert!(
            !printed.contains("sk-test"),
            "the key leaked into Debug output: {printed}"
        );
        assert!(printed.contains("Sensitive"), "{printed}");
        assert_eq!(provider.id(), "test-provider");
        assert_eq!(provider.model(), "canned-model");
        assert_eq!(
            provider.endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );
    }
}
