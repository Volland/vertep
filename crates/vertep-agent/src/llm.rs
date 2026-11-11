//! LLM actors: the provider contract, a deterministic scripted provider, prompt templates and
//! the `<invoke type="vt:llm">` specification.
//!
//! An LLM invocation is read from `vt:` extension attributes on the `<invoke>`:
//!
//! | Attribute | Meaning |
//! |---|---|
//! | `vt:provider="claude"` | the provider binding (required) |
//! | `vt:model="claude-opus-5-5"` | the model identifier |
//! | `vt:template="Summarise {{context.ticket.text}}"` | the prompt template (or the `<content>` body) |
//! | `vt:output="{ category: string, urgency: int }"` | the structured output type (type language; named types from the chart) |
//! | `vt:temperature`, `vt:maxTokens` | generation parameters |
//! | `vt:stream="true"` | deliver `vt.llm.partial` events while the model streams |
//! | `vt:retry="3"`, `vt:backoff="1s"` | at most 3 attempts, durable exponential backoff from 1 s |
//!
//! Templates substitute `{{context.<path>}}` (the data items) and `{{event.<path>}}`
//! (`event.name`, `event.data.<path>`) deterministically: strings verbatim, other values as
//! canonical JSON. A missing path is a template error and no call is made.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use vertep_core::datamodel::{Event, Value};
use vertep_core::ir::{Extension, ExtensionValue, Invoke, VT_NS};

use crate::json::{at_path, canonical, sha256, to_json};

/// A rendered prompt and its content hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenderedPrompt {
    /// The prompt text.
    pub text: String,
    /// `sha256:<hex>` of the text's UTF-8 bytes.
    pub hash: String,
}

impl RenderedPrompt {
    /// A prompt with its hash.
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        RenderedPrompt {
            hash: sha256(text.as_bytes()),
            text,
        }
    }
}

/// Generation parameters.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GenerationParams {
    /// Sampling temperature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    /// Maximum completion tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
}

/// A tool offered to the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSchema {
    /// The tool IRI.
    pub name: String,
    /// What it does.
    pub description: String,
    /// The JSON Schema of its arguments.
    pub input_schema: Json,
}

/// One provider call.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderRequest {
    /// The rendered prompt.
    pub prompt: RenderedPrompt,
    /// The model identifier.
    pub model: String,
    /// Generation parameters.
    pub params: GenerationParams,
    /// The structured-output schema, if any.
    pub output_schema: Option<Json>,
    /// The tools the model may call (exactly the granted ones).
    pub tools: Vec<ToolSchema>,
}

/// Token usage of a call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Prompt tokens.
    pub prompt_tokens: u64,
    /// Completion tokens.
    pub completion_tokens: u64,
}

/// A call's cost in a declared currency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    /// The amount.
    pub amount: f64,
    /// The ISO 4217 currency code.
    pub currency: String,
}

/// A tool call the model asked for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCallRequest {
    /// The tool IRI.
    pub tool: String,
    /// The arguments.
    pub args: Json,
}

/// A successful provider call.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderResult {
    /// The completion text (JSON text for structured output).
    pub content: String,
    /// Token usage.
    pub usage: Usage,
    /// The model identifier the provider reports.
    pub model: String,
    /// The cost, when the provider reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    /// Tool calls the model asked for.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCallRequest>,
}

impl ProviderResult {
    /// A text result with usage.
    pub fn text(content: impl Into<String>, model: impl Into<String>, usage: Usage) -> Self {
        ProviderResult {
            content: content.into(),
            usage,
            model: model.into(),
            cost: None,
            tool_calls: Vec::new(),
        }
    }
}

/// The class of a failed call, as carried by `error.llm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ErrorClass {
    /// Too many requests; retry after a while.
    RateLimited,
    /// The output does not conform to the requested schema.
    Schema,
    /// The network or HTTP layer failed.
    Transport,
    /// Credentials were refused.
    Auth,
    /// The request was malformed.
    InvalidRequest,
    /// The provider failed.
    Provider,
    /// The binding names no registered provider.
    UnknownProvider,
}

impl ErrorClass {
    /// The class name used in events.
    pub fn as_str(self) -> &'static str {
        match self {
            ErrorClass::RateLimited => "rate-limited",
            ErrorClass::Schema => "schema",
            ErrorClass::Transport => "transport",
            ErrorClass::Auth => "auth",
            ErrorClass::InvalidRequest => "invalid-request",
            ErrorClass::Provider => "provider",
            ErrorClass::UnknownProvider => "unknown-provider",
        }
    }

    /// Whether a declared `vt:retry` retries this class.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorClass::RateLimited
                | ErrorClass::Schema
                | ErrorClass::Transport
                | ErrorClass::Provider
        )
    }
}

impl core::fmt::Display for ErrorClass {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A classified provider failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderError {
    /// The class.
    pub class: ErrorClass,
    /// A description.
    pub message: String,
    /// The provider's retry-after hint, in milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<i64>,
}

impl ProviderError {
    /// An error of `class` with `message`.
    pub fn new(class: ErrorClass, message: impl Into<String>) -> Self {
        ProviderError {
            class,
            message: message.into(),
            retry_after_ms: None,
        }
    }

    /// A rate-limit error with a retry-after hint.
    pub fn rate_limited(retry_after_ms: Option<i64>) -> Self {
        ProviderError {
            class: ErrorClass::RateLimited,
            message: "rate limited".into(),
            retry_after_ms,
        }
    }
}

/// A streamed chunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamChunk {
    /// A piece of the completion text.
    Delta(String),
}

/// The provider contract: a rendered prompt, a model, parameters and an optional output
/// schema in; content, usage and the provider's model id out, or a classified error. No vendor
/// is required to build or run Vertep; concrete providers are optional features.
// @tg: implements:: [[openspec:llm-actors#Provider Contract]]
pub trait Provider: Send + Sync {
    /// Completes `request`.
    fn complete(
        &self,
        request: ProviderRequest,
    ) -> BoxFuture<'static, Result<ProviderResult, ProviderError>>;

    /// Streams `request` as chunks followed by the final result, when the provider can.
    fn stream(
        &self,
        _request: ProviderRequest,
    ) -> Option<BoxStream<'static, Result<StreamItem, ProviderError>>> {
        None
    }
}

/// An item of a streamed completion.
#[derive(Clone, Debug, PartialEq)]
pub enum StreamItem {
    /// A partial chunk.
    Chunk(StreamChunk),
    /// The complete result; the stream ends after it.
    Done(ProviderResult),
}

/// Providers by binding name.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    map: BTreeMap<String, Arc<dyn Provider>>,
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRegistry")
            .field("bindings", &self.map.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl ProviderRegistry {
    /// Registers `provider` as `binding`.
    pub fn insert(&mut self, binding: impl Into<String>, provider: Arc<dyn Provider>) {
        self.map.insert(binding.into(), provider);
    }

    /// The provider bound to `binding`.
    pub fn get(&self, binding: &str) -> Option<&Arc<dyn Provider>> {
        self.map.get(binding)
    }
}

/// A deterministic provider for tests: answers from a script, in order, then repeats a default;
/// records every request.
#[derive(Clone, Default)]
pub struct ScriptedProvider {
    inner: Arc<Mutex<Scripted>>,
}

#[derive(Default)]
struct Scripted {
    script: VecDeque<Result<ProviderResult, ProviderError>>,
    default: Option<Result<ProviderResult, ProviderError>>,
    chunks: Vec<String>,
    requests: Vec<ProviderRequest>,
}

impl std::fmt::Debug for ScriptedProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptedProvider")
            .field("calls", &self.calls())
            .finish()
    }
}

impl ScriptedProvider {
    /// An empty script (calls fail with `provider` until answers are pushed).
    pub fn new() -> Self {
        ScriptedProvider::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Scripted> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Appends an answer.
    pub fn push(&self, answer: Result<ProviderResult, ProviderError>) -> &Self {
        self.lock().script.push_back(answer);
        self
    }

    /// Appends a text answer with the given usage.
    pub fn push_text(&self, content: &str, prompt_tokens: u64, completion_tokens: u64) -> &Self {
        self.push(Ok(ProviderResult::text(
            content,
            "scripted-model",
            Usage {
                prompt_tokens,
                completion_tokens,
            },
        )))
    }

    /// The answer given once the script is exhausted.
    pub fn with_default(self, answer: Result<ProviderResult, ProviderError>) -> Self {
        self.lock().default = Some(answer);
        self
    }

    /// Streams these chunks before each streamed answer.
    pub fn with_chunks<I: IntoIterator<Item = S>, S: Into<String>>(self, chunks: I) -> Self {
        self.lock().chunks = chunks.into_iter().map(Into::into).collect();
        self
    }

    /// The number of calls made.
    pub fn calls(&self) -> usize {
        self.lock().requests.len()
    }

    /// Every request received.
    pub fn requests(&self) -> Vec<ProviderRequest> {
        self.lock().requests.clone()
    }

    fn next(&self, request: ProviderRequest) -> Result<ProviderResult, ProviderError> {
        let mut g = self.lock();
        g.requests.push(request);
        g.script
            .pop_front()
            .or_else(|| g.default.clone())
            .unwrap_or_else(|| Err(ProviderError::new(ErrorClass::Provider, "script exhausted")))
    }
}

impl Provider for ScriptedProvider {
    fn complete(
        &self,
        request: ProviderRequest,
    ) -> BoxFuture<'static, Result<ProviderResult, ProviderError>> {
        let out = self.next(request);
        Box::pin(async move { out })
    }

    fn stream(
        &self,
        request: ProviderRequest,
    ) -> Option<BoxStream<'static, Result<StreamItem, ProviderError>>> {
        let chunks = self.lock().chunks.clone();
        let out = self.next(request);
        let mut items: Vec<Result<StreamItem, ProviderError>> = chunks
            .into_iter()
            .map(|c| Ok(StreamItem::Chunk(StreamChunk::Delta(c))))
            .collect();
        items.push(out.map(StreamItem::Done));
        Some(Box::pin(futures::stream::iter(items)))
    }
}

// ----- templates ------------------------------------------------------------------------

/// A template that could not be rendered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateError {
    /// The missing or malformed path.
    pub path: String,
}

impl core::fmt::Display for TemplateError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "template path {} does not exist", self.path)
    }
}

impl std::error::Error for TemplateError {}

fn event_value(e: &Event) -> Value {
    crate::json::obj([
        ("name", Value::String(e.name.clone())),
        ("data", e.data.clone()),
        (
            "invokeid",
            e.invokeid.clone().map_or(Value::Undefined, Value::String),
        ),
    ])
}

/// Renders `template` against the data items `context` and the triggering `event`.
/// Deterministic: equal inputs give equal bytes.
///
/// # Errors
///
/// [`TemplateError`] naming the first path that does not exist.
// @tg: implements:: [[openspec:llm-actors#Prompt Templates]]
pub fn render(
    template: &str,
    context: &Value,
    event: Option<&Event>,
) -> Result<RenderedPrompt, TemplateError> {
    let ev = event.map(event_value).unwrap_or(Value::Undefined);
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(i) = rest.find("{{") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        let Some(j) = after.find("}}") else {
            return Err(TemplateError {
                path: after.trim().to_owned(),
            });
        };
        let path = after[..j].trim();
        let value = if let Some(p) = path.strip_prefix("context.") {
            at_path(context, p)
        } else if path == "context" {
            Some(context)
        } else if let Some(p) = path.strip_prefix("event.") {
            at_path(&ev, p)
        } else {
            None
        };
        match value {
            Some(Value::Undefined) | None => {
                return Err(TemplateError {
                    path: path.to_owned(),
                });
            }
            Some(Value::String(s)) => out.push_str(s),
            Some(v) => out.push_str(&canonical(&to_json(v))),
        }
        rest = &after[j + 2..];
    }
    out.push_str(rest);
    Ok(RenderedPrompt::new(out))
}

// ----- invocation spec --------------------------------------------------------------------

/// A `vt:retry` policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Attempts in total (the first call included).
    pub attempts: u32,
    /// The first backoff in milliseconds; attempt n waits `backoff * 2^(n-2)`.
    pub backoff_ms: i64,
}

impl RetryPolicy {
    /// The wait before attempt `next` (2-based).
    pub fn backoff_before(&self, next: u32) -> i64 {
        self.backoff_ms
            .saturating_mul(1i64 << next.saturating_sub(2).min(30))
    }
}

/// An `<invoke type="vt:llm">` as declared.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LlmSpec {
    /// The provider binding.
    pub provider: String,
    /// The model identifier.
    pub model: String,
    /// The template text.
    pub template: String,
    /// The output type text, if any.
    pub output: Option<String>,
    /// Generation parameters.
    pub params: GenerationParams,
    /// Whether partials are streamed.
    pub stream: bool,
    /// The retry policy.
    pub retry: Option<RetryPolicy>,
}

fn attr<'a>(exts: &'a [Extension], local: &str) -> Option<&'a str> {
    exts.iter().find_map(|e| match &e.value {
        ExtensionValue::Attribute(v) if e.ns == VT_NS && e.local == local => Some(v.as_str()),
        _ => None,
    })
}

/// The default model of the Anthropic provider.
pub const DEFAULT_ANTHROPIC_MODEL: &str = "claude-opus-5-5";

impl LlmSpec {
    /// Reads the spec of `invoke`; `content` is the evaluated inline content, used as the
    /// template when `vt:template` is absent.
    // @tg: implements:: [[openspec:llm-actors#LLM Invocation]]
    pub fn of(invoke: &Invoke, content: Option<&Value>) -> LlmSpec {
        let x = &invoke.extensions;
        let template = attr(x, "template")
            .map(str::to_owned)
            .or_else(|| match content {
                Some(Value::String(s)) => Some(s.clone()),
                Some(Value::Undefined) | None => None,
                Some(v) => Some(canonical(&to_json(v))),
            })
            .unwrap_or_default();
        let retry = attr(x, "retry")
            .and_then(|r| r.trim().parse::<u32>().ok())
            .filter(|n| *n > 1)
            .map(|attempts| RetryPolicy {
                attempts,
                backoff_ms: attr(x, "backoff")
                    .and_then(vertep_runtime::chart::parse_duration)
                    .unwrap_or(1_000),
            });
        LlmSpec {
            provider: attr(x, "provider").unwrap_or_default().trim().to_owned(),
            model: attr(x, "model")
                .unwrap_or(DEFAULT_ANTHROPIC_MODEL)
                .trim()
                .to_owned(),
            template,
            output: attr(x, "output").map(str::to_owned),
            params: GenerationParams {
                temperature: attr(x, "temperature").and_then(|t| t.trim().parse().ok()),
                max_tokens: attr(x, "maxTokens").and_then(|t| t.trim().parse().ok()),
            },
            stream: attr(x, "stream").is_some_and(|s| s.trim() == "true"),
            retry,
        }
    }
}

/// The JSON text of a result as a value, for structured output.
pub fn parse_output(content: &str) -> Result<Value, String> {
    serde_json::from_str::<Json>(content)
        .map(|j| crate::json::from_json(&j))
        .map_err(|e| format!("the output is not JSON: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::{obj, s};

    #[test]
    fn templates_render_context_and_event() {
        let ctx = obj([("customer", obj([("name", s("Ada"))]))]);
        let ev = Event {
            name: "task".into(),
            data: obj([("n", Value::Number(2.0))]),
            ..Event::default()
        };
        let p = render(
            "Hi {{context.customer.name}}, {{event.name}} {{ event.data }}",
            &ctx,
            Some(&ev),
        )
        .unwrap();
        assert_eq!(p.text, "Hi Ada, task {\"n\":2}");
        assert!(p.hash.starts_with("sha256:"));
        assert_eq!(
            render("{{context.nope}}", &ctx, None).unwrap_err().path,
            "context.nope"
        );
    }

    #[test]
    fn backoff_doubles() {
        let r = RetryPolicy {
            attempts: 3,
            backoff_ms: 100,
        };
        assert_eq!((r.backoff_before(2), r.backoff_before(3)), (100, 200));
    }
}
