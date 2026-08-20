// Copyright 2026 Apoorv Dixit
// SPDX-License-Identifier: Apache-2.0

//! One embeddings client for every OpenAI-compatible `/embeddings` endpoint.
//!
//! The same argument that gives the app one chat client applies here: OpenAI,
//! Gemini's compatibility surface, Ollama and LM Studio all answer
//! `POST {base}/embeddings` with the same shape, so one client means one
//! request builder and one response parse to get right. Notably this makes
//! the free, local path real — an Ollama embedding model serves the identical
//! wire format, so "semantic search without a key" needs nothing new.
//!
//! No streaming and no cancellation: an embeddings call is one small POST
//! with one JSON answer, and the machinery `chat/completions` needs would be
//! dead weight. Timeouts bound it instead.

use reqwest::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE};
use reqwest::{Client, StatusCode, Url};
use serde::Deserialize;

use super::openai::{authorization_header, endpoint_under, USER_AGENT};
use super::types::{ApiKey, ProviderError};

const EMBEDDINGS_PATH: [&str; 1] = ["embeddings"];

/// Everything needed to reach one `/embeddings` endpoint.
#[derive(Debug, Clone)]
pub struct EmbeddingsConfig {
    /// Named in errors, like every provider.
    pub id: String,
    /// Base URL up to but excluding `/embeddings` — the same base the chat
    /// client uses for the same provider.
    pub base_url: String,
    /// The embedding model. Also the namespace vectors are stored under, so
    /// changing it invalidates the stored index by design.
    pub model: String,
    pub api_key: Option<ApiKey>,
    pub connect_timeout: std::time::Duration,
    pub read_timeout: std::time::Duration,
}

impl EmbeddingsConfig {
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
            connect_timeout: std::time::Duration::from_secs(10),
            read_timeout: std::time::Duration::from_secs(60),
        }
    }

    pub fn with_api_key(mut self, api_key: ApiKey) -> Self {
        self.api_key = Some(api_key);
        self
    }
}

/// A client for one configured embeddings endpoint.
#[derive(Debug, Clone)]
pub struct EmbeddingsClient {
    id: String,
    endpoint: Url,
    model: String,
    authorization: Option<HeaderValue>,
    client: Client,
}

/// The `/embeddings` response, reduced to what matters.
#[derive(Deserialize)]
struct WireResponse {
    data: Vec<WireEmbedding>,
}

#[derive(Deserialize)]
struct WireEmbedding {
    /// Position in the request's input list. The spec allows out-of-order
    /// data, so this is honoured rather than assumed sequential — a vector
    /// stored against the wrong chunk would corrupt retrieval invisibly.
    index: usize,
    embedding: Vec<f32>,
}

impl EmbeddingsClient {
    /// Validate the configuration and build a client. No network here.
    pub fn new(config: EmbeddingsConfig) -> Result<Self, ProviderError> {
        if config.model.trim().is_empty() {
            return Err(ProviderError::Config {
                provider: config.id.clone(),
                detail: "no embedding model is configured, and the endpoint requires one"
                    .to_string(),
            });
        }
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(config.connect_timeout)
            .read_timeout(config.read_timeout)
            .build()
            .map_err(|source| ProviderError::Config {
                provider: config.id.clone(),
                detail: format!(
                    "an HTTPS client could not be built: {}",
                    source.without_url()
                ),
            })?;

        Ok(Self {
            endpoint: endpoint_under(&config.id, &config.base_url, &EMBEDDINGS_PATH)?,
            authorization: authorization_header(&config.id, config.api_key.as_ref())?,
            id: config.id,
            model: config.model,
            client,
        })
    }

    /// The model requests are made with — and the namespace vectors live in.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Embed `texts`, one vector per text, in the same order.
    ///
    /// The order guarantee is this method's whole contract: callers zip the
    /// result against chunk ids, and the wire format's `index` field is used
    /// to restore order rather than trusted to already be in it.
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, ProviderError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let mut request = self
            .client
            .post(self.endpoint.clone())
            .header(CONTENT_TYPE, "application/json")
            .json(&serde_json::json!({
                "model": self.model,
                "input": texts,
            }));
        if let Some(authorization) = &self.authorization {
            request = request.header(AUTHORIZATION, authorization.clone());
        }

        let response = request
            .send()
            .await
            .map_err(|source| ProviderError::Transport {
                provider: self.id.clone(),
                source,
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(refusal(&self.id, status, response.text().await.ok()));
        }

        let wire: WireResponse =
            response
                .json()
                .await
                .map_err(|source| ProviderError::Protocol {
                    provider: self.id.clone(),
                    detail: format!("the embeddings response is not the expected JSON: {source}"),
                })?;

        if wire.data.len() != texts.len() {
            return Err(ProviderError::Protocol {
                provider: self.id.clone(),
                detail: format!(
                    "{} texts were sent but {} embeddings came back",
                    texts.len(),
                    wire.data.len()
                ),
            });
        }

        // Restore request order via the index field. A slot left empty or
        // filled twice is a malformed response, not something to paper over.
        let mut ordered: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        for item in wire.data {
            let slot = ordered
                .get_mut(item.index)
                .ok_or_else(|| ProviderError::Protocol {
                    provider: self.id.clone(),
                    detail: format!(
                        "an embedding arrived for index {} of {}",
                        item.index,
                        texts.len()
                    ),
                })?;
            if slot.replace(item.embedding).is_some() {
                return Err(ProviderError::Protocol {
                    provider: self.id.clone(),
                    detail: format!("two embeddings arrived for index {}", item.index),
                });
            }
        }
        ordered
            .into_iter()
            .enumerate()
            .map(|(index, slot)| {
                slot.filter(|v| !v.is_empty())
                    .ok_or_else(|| ProviderError::Protocol {
                        provider: self.id.clone(),
                        detail: format!("no usable embedding arrived for index {index}"),
                    })
            })
            .collect()
    }
}

/// An HTTP refusal, with the provider's own words when it offered any.
fn refusal(provider: &str, status: StatusCode, body: Option<String>) -> ProviderError {
    let detail = body
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        // Bounded: an HTML error page must not become a screen-filling error.
        .map(|text| text.chars().take(300).collect::<String>())
        .unwrap_or_else(|| "the response body was empty".to_string());
    ProviderError::Http {
        provider: provider.to_string(),
        status: status.as_u16(),
        message: detail,
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::*;

    /// The same one-shot loopback server the chat client's tests use: real
    /// sockets, a real reqwest, nothing mocked but the provider.
    struct TestServer {
        base_url: String,
        request: mpsc::Receiver<Vec<u8>>,
    }

    impl TestServer {
        fn respond_with(body: &str) -> Self {
            let payload = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            Self::start(move |socket| {
                let _ = socket.write_all(payload.as_bytes());
            })
        }

        fn start<F>(respond: F) -> Self
        where
            F: FnOnce(&mut TcpStream) + Send + 'static,
        {
            let listener =
                TcpListener::bind("127.0.0.1:0").expect("a loopback port must be available");
            let port = listener.local_addr().expect("addr").port();
            let (sender, request) = mpsc::channel();

            thread::spawn(move || {
                let (mut socket, _) = listener.accept().expect("the client must connect");
                socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .expect("a read timeout must be settable");

                let mut raw = Vec::new();
                let mut buffer = [0u8; 4096];
                while let Ok(read) = socket.read(&mut buffer) {
                    if read == 0 {
                        break;
                    }
                    raw.extend_from_slice(&buffer[..read]);
                    // Stop once the body promised by content-length is here.
                    if let Some(headers_end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&raw[..headers_end]).to_lowercase();
                        let expected = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if raw.len() >= headers_end + 4 + expected {
                            break;
                        }
                    }
                }
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

        fn request_text(&self) -> String {
            let raw = self
                .request
                .recv_timeout(Duration::from_secs(10))
                .expect("the request must have arrived");
            String::from_utf8_lossy(&raw).to_string()
        }
    }

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime must build")
            .block_on(future)
    }

    #[test]
    fn embeddings_come_back_in_request_order_even_when_sent_shuffled() {
        let server = TestServer::respond_with(
            r#"{"data":[
                {"index":1,"embedding":[0.3,0.4]},
                {"index":0,"embedding":[0.1,0.2]}
            ]}"#,
        );
        let client = EmbeddingsClient::new(EmbeddingsConfig::new(
            "test",
            &server.base_url,
            "embed-model-1",
        ))
        .unwrap();

        let vectors = block_on(client.embed(&["first".to_string(), "second".to_string()])).unwrap();
        assert_eq!(vectors, vec![vec![0.1, 0.2], vec![0.3, 0.4]]);

        let request = server.request_text();
        assert!(
            request.contains("POST /v1/embeddings"),
            "the endpoint is base + /embeddings: {request}"
        );
        assert!(request.contains(r#""model":"embed-model-1""#));
        assert!(request.contains(r#""input":["first","second"]"#));
    }

    #[test]
    fn the_key_travels_as_a_bearer_header_and_absence_sends_none() {
        let server = TestServer::respond_with(r#"{"data":[{"index":0,"embedding":[1.0]}]}"#);
        let client = EmbeddingsClient::new(
            EmbeddingsConfig::new("test", &server.base_url, "m").with_api_key(ApiKey::new("sk-t")),
        )
        .unwrap();
        block_on(client.embed(&["x".to_string()])).unwrap();
        assert!(server.request_text().contains("authorization: Bearer sk-t"));

        let server = TestServer::respond_with(r#"{"data":[{"index":0,"embedding":[1.0]}]}"#);
        let client =
            EmbeddingsClient::new(EmbeddingsConfig::new("test", &server.base_url, "m")).unwrap();
        block_on(client.embed(&["x".to_string()])).unwrap();
        assert!(
            !server.request_text().contains("authorization"),
            "a keyless local provider must get no Authorization header at all"
        );
    }

    #[test]
    fn a_count_mismatch_and_a_duplicate_index_are_refused_as_malformed() {
        let server = TestServer::respond_with(r#"{"data":[{"index":0,"embedding":[1.0]}]}"#);
        let client =
            EmbeddingsClient::new(EmbeddingsConfig::new("test", &server.base_url, "m")).unwrap();
        let error =
            block_on(client.embed(&["a".to_string(), "b".to_string()])).expect_err("must refuse");
        assert!(
            error.to_string().contains("2 texts") && error.to_string().contains("1 embeddings"),
            "the mismatch must be countable in the message: {error}"
        );

        let server = TestServer::respond_with(
            r#"{"data":[{"index":0,"embedding":[1.0]},{"index":0,"embedding":[2.0]}]}"#,
        );
        let client =
            EmbeddingsClient::new(EmbeddingsConfig::new("test", &server.base_url, "m")).unwrap();
        let error =
            block_on(client.embed(&["a".to_string(), "b".to_string()])).expect_err("must refuse");
        assert!(error.to_string().contains("index 0"), "got {error}");
    }

    #[test]
    fn an_http_refusal_carries_the_providers_words_bounded() {
        let body = r#"{"error":{"message":"invalid api key"}}"#;
        let payload = format!(
            "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        let server = TestServer::start(move |socket| {
            let _ = socket.write_all(payload.as_bytes());
        });
        let client =
            EmbeddingsClient::new(EmbeddingsConfig::new("test", &server.base_url, "m")).unwrap();
        let error = block_on(client.embed(&["x".to_string()])).expect_err("401 must refuse");
        let text = error.to_string();
        assert!(
            text.contains("401") && text.contains("invalid api key"),
            "{text}"
        );
    }

    #[test]
    fn an_empty_input_makes_no_request_at_all() {
        // No server: a network call here would fail the test by timing out.
        let client =
            EmbeddingsClient::new(EmbeddingsConfig::new("test", "http://127.0.0.1:1/v1", "m"))
                .unwrap();
        assert_eq!(block_on(client.embed(&[])).unwrap(), Vec::<Vec<f32>>::new());
    }

    #[test]
    fn configuration_mistakes_fail_at_construction_with_words() {
        let error = EmbeddingsClient::new(EmbeddingsConfig::new("test", "http://x/v1", " "))
            .expect_err("an empty model must be refused");
        assert!(error.to_string().contains("model"), "{error}");

        let error = EmbeddingsClient::new(EmbeddingsConfig::new("test", "ftp://x/v1", "m"))
            .expect_err("a non-http scheme must be refused");
        assert!(error.to_string().contains("http"), "{error}");
    }
}
