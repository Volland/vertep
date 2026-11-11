//! The reference provider: the Anthropic Messages API (`POST /v1/messages`), as request and
//! response mapping over an [`HttpClient`] the application supplies.
//!
//! Vertep carries no HTTP stack and no vendor SDK: the provider builds the request body and
//! headers, hands them to the client, and classifies the response. Structured output uses
//! `output_config.format` with a `json_schema`; granted tools become `tools` entries whose
//! names are derived from the tool IRIs (the API restricts tool names to `[A-Za-z0-9_-]`) and
//! mapped back on the way out. A 429 becomes `rate-limited` with the `retry-after` hint, 401
//! and 403 `auth`, other 4xx `invalid-request`, 5xx and 529 `provider`, a client failure
//! `transport`.
//!
//! The API version this mapping targets (`2026-10-21`) rejects sampling parameters, so
//! [`GenerationParams::temperature`](crate::llm::GenerationParams) is not sent.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use serde_json::{Value as Json, json};

use crate::llm::{
    DEFAULT_ANTHROPIC_MODEL, ErrorClass, Provider, ProviderError, ProviderRequest, ProviderResult,
    ToolCallRequest, Usage,
};

/// The API version header value.
pub const API_VERSION: &str = "2026-10-21";
/// The default endpoint.
pub const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
/// `max_tokens` when the invocation declares none.
pub const DEFAULT_MAX_TOKENS: u32 = 16_000;

/// An HTTP response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpResponse {
    /// The status code.
    pub status: u16,
    /// The headers (names in any case).
    pub headers: Vec<(String, String)>,
    /// The body.
    pub body: Vec<u8>,
}

/// The HTTP client the provider posts through.
pub trait HttpClient: Send + Sync {
    /// Posts `body` to `url` with `headers`; an `Err` is a transport failure.
    fn post(
        &self,
        url: &str,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> BoxFuture<'static, Result<HttpResponse, String>>;
}

/// The Anthropic Messages API provider.
#[derive(Clone)]
pub struct AnthropicProvider {
    http: Arc<dyn HttpClient>,
    api_key: String,
    base_url: String,
}

impl std::fmt::Debug for AnthropicProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicProvider")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

/// The API tool name of a tool IRI: its characters outside `[A-Za-z0-9_-]` replaced by `_`,
/// at most 64 characters.
pub fn tool_name(iri: &str) -> String {
    let mut out: String = iri
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    out.truncate(64);
    out
}

/// The request body for `req`, and the map from API tool names back to tool IRIs.
pub fn request_body(req: &ProviderRequest) -> (Json, BTreeMap<String, String>) {
    let model = if req.model.is_empty() {
        DEFAULT_ANTHROPIC_MODEL
    } else {
        req.model.as_str()
    };
    let mut body = json!({
        "model": model,
        "max_tokens": req.params.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        "messages": [{ "role": "user", "content": req.prompt.text }],
    });
    if let Some(schema) = &req.output_schema {
        body["output_config"] = json!({ "format": { "type": "json_schema", "schema": schema } });
    }
    let mut names = BTreeMap::new();
    if !req.tools.is_empty() {
        let mut tools = Vec::new();
        for (i, t) in req.tools.iter().enumerate() {
            let mut name = tool_name(&t.name);
            if names.contains_key(&name) {
                name.truncate(58);
                name.push_str(&format!("_{i}"));
            }
            names.insert(name.clone(), t.name.clone());
            tools.push(json!({
                "name": name,
                "description": if t.description.is_empty() { t.name.clone() } else { t.description.clone() },
                "input_schema": t.input_schema,
            }));
        }
        body["tools"] = Json::Array(tools);
    }
    (body, names)
}

fn header<'a>(r: &'a HttpResponse, name: &str) -> Option<&'a str> {
    r.headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// Classifies a response to a request whose tools were named by `names`.
///
/// # Errors
///
/// [`ProviderError`] with its class.
pub fn parse_response(
    r: &HttpResponse,
    names: &BTreeMap<String, String>,
) -> Result<ProviderResult, ProviderError> {
    let text = String::from_utf8_lossy(&r.body).into_owned();
    match r.status {
        200..=299 => {}
        429 => {
            let retry = header(r, "retry-after")
                .and_then(|v| v.trim().parse::<f64>().ok())
                .map(|s| (s * 1000.0) as i64);
            let mut e = ProviderError::rate_limited(retry);
            e.message = text;
            return Err(e);
        }
        401 | 403 => return Err(ProviderError::new(ErrorClass::Auth, text)),
        400..=499 => return Err(ProviderError::new(ErrorClass::InvalidRequest, text)),
        _ => return Err(ProviderError::new(ErrorClass::Provider, text)),
    }
    let j: Json = serde_json::from_str(&text).map_err(|e| {
        ProviderError::new(ErrorClass::Provider, format!("response is not JSON: {e}"))
    })?;
    if j["stop_reason"] == "refusal" {
        return Err(ProviderError::new(
            ErrorClass::Provider,
            format!("the model refused: {}", j["stop_details"]),
        ));
    }
    let mut content = String::new();
    let mut tool_calls = Vec::new();
    for block in j["content"].as_array().into_iter().flatten() {
        match block["type"].as_str() {
            Some("text") => content.push_str(block["text"].as_str().unwrap_or_default()),
            Some("tool_use") => {
                let name = block["name"].as_str().unwrap_or_default();
                tool_calls.push(ToolCallRequest {
                    tool: names.get(name).cloned().unwrap_or_else(|| name.to_owned()),
                    args: block["input"].clone(),
                });
            }
            _ => {}
        }
    }
    Ok(ProviderResult {
        content,
        usage: Usage {
            prompt_tokens: j["usage"]["input_tokens"].as_u64().unwrap_or(0),
            completion_tokens: j["usage"]["output_tokens"].as_u64().unwrap_or(0),
        },
        model: j["model"].as_str().unwrap_or_default().to_owned(),
        cost: None,
        tool_calls,
    })
}

impl AnthropicProvider {
    /// A provider posting through `http` with `api_key` to the default endpoint.
    pub fn new(http: Arc<dyn HttpClient>, api_key: impl Into<String>) -> Self {
        AnthropicProvider {
            http,
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.into(),
        }
    }

    /// Another endpoint (a proxy or a test server).
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// The headers of every request.
    pub fn headers(&self) -> Vec<(String, String)> {
        vec![
            ("x-api-key".into(), self.api_key.clone()),
            ("anthropic-version".into(), API_VERSION.into()),
            ("content-type".into(), "application/json".into()),
        ]
    }
}

impl Provider for AnthropicProvider {
    // @tg: implements:: [[openspec:llm-actors#Provider Contract]]
    fn complete(
        &self,
        request: ProviderRequest,
    ) -> BoxFuture<'static, Result<ProviderResult, ProviderError>> {
        let (body, names) = request_body(&request);
        let url = format!("{}/v1/messages", self.base_url.trim_end_matches('/'));
        let post = self
            .http
            .post(&url, self.headers(), body.to_string().into_bytes());
        Box::pin(async move {
            match post.await {
                Ok(r) => parse_response(&r, &names),
                Err(e) => Err(ProviderError::new(ErrorClass::Transport, e)),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::llm::{GenerationParams, RenderedPrompt, ToolSchema};

    type Seen = (String, Vec<(String, String)>, Json);

    #[derive(Default)]
    struct Canned {
        response: Mutex<Option<HttpResponse>>,
        seen: Mutex<Vec<Seen>>,
    }

    impl HttpClient for Canned {
        fn post(
            &self,
            url: &str,
            headers: Vec<(String, String)>,
            body: Vec<u8>,
        ) -> BoxFuture<'static, Result<HttpResponse, String>> {
            let body: Json = serde_json::from_slice(&body).unwrap();
            self.seen
                .lock()
                .unwrap()
                .push((url.to_owned(), headers, body));
            let r = self.response.lock().unwrap().take();
            Box::pin(async move { r.ok_or_else(|| "connection refused".to_owned()) })
        }
    }

    fn request() -> ProviderRequest {
        ProviderRequest {
            prompt: RenderedPrompt::new("Classify: refund please"),
            model: "claude-opus-5-5".into(),
            params: GenerationParams {
                temperature: Some(0.2),
                max_tokens: Some(512),
            },
            output_schema: Some(json!({"type": "object", "properties": {"c": {"type": "string"}}})),
            tools: vec![ToolSchema {
                name: "https://example.org/tools/search".into(),
                description: "Search".into(),
                input_schema: json!({"type": "object"}),
            }],
        }
    }

    fn run(r: Option<HttpResponse>) -> (Result<ProviderResult, ProviderError>, Arc<Canned>) {
        let http = Arc::new(Canned {
            response: Mutex::new(r),
            ..Canned::default()
        });
        let p = AnthropicProvider::new(http.clone(), "k");
        (
            futures::FutureExt::now_or_never(p.complete(request())).expect("ready"),
            http,
        )
    }

    fn ok(body: Json) -> Option<HttpResponse> {
        Some(HttpResponse {
            status: 200,
            headers: Vec::new(),
            body: body.to_string().into_bytes(),
        })
    }

    #[test]
    fn maps_request_and_response() {
        let (r, http) = run(ok(json!({
            "model": "claude-opus-5-5",
            "stop_reason": "tool_use",
            "content": [
                {"type": "thinking", "thinking": ""},
                {"type": "text", "text": "{\"c\":\"billing\"}"},
                {"type": "tool_use", "id": "t1", "name": "https___example_org_tools_search", "input": {"q": "refund"}}
            ],
            "usage": {"input_tokens": 1200, "output_tokens": 300}
        })));
        let r = r.unwrap();
        assert_eq!(r.content, "{\"c\":\"billing\"}");
        assert_eq!(
            (r.usage.prompt_tokens, r.usage.completion_tokens),
            (1200, 300)
        );
        assert_eq!(r.tool_calls[0].tool, "https://example.org/tools/search");
        let seen = http.seen.lock().unwrap();
        let (url, headers, body) = &seen[0];
        assert_eq!(url, "https://api.anthropic.com/v1/messages");
        assert!(headers.contains(&("anthropic-version".into(), API_VERSION.into())));
        assert_eq!(body["max_tokens"], 512);
        assert!(body.get("temperature").is_none());
        assert_eq!(body["output_config"]["format"]["type"], "json_schema");
        assert_eq!(body["tools"][0]["name"], "https___example_org_tools_search");
        assert_eq!(body["messages"][0]["content"], "Classify: refund please");
    }

    #[test]
    fn classifies_errors() {
        let status = |code: u16, headers: Vec<(String, String)>| {
            run(Some(HttpResponse {
                status: code,
                headers,
                body: b"{}".to_vec(),
            }))
            .0
            .unwrap_err()
        };
        let e = status(429, vec![("Retry-After".into(), "2".into())]);
        assert_eq!(
            (e.class, e.retry_after_ms),
            (ErrorClass::RateLimited, Some(2_000))
        );
        assert_eq!(status(401, Vec::new()).class, ErrorClass::Auth);
        assert_eq!(status(400, Vec::new()).class, ErrorClass::InvalidRequest);
        assert_eq!(status(529, Vec::new()).class, ErrorClass::Provider);
        assert_eq!(run(None).0.unwrap_err().class, ErrorClass::Transport);
        let refused = run(ok(json!({"stop_reason": "refusal", "content": []}))).0;
        assert_eq!(refused.unwrap_err().class, ErrorClass::Provider);
    }
}
