//! Tool actors: registered by IRI with an argument type and a result type in the IR type
//! language, from which their JSON Schemas are derived.
//!
//! A chart calls a tool with `<invoke type="vt:tool" src="vt:tool/search">` (the arguments are
//! the invoke's `<param>`s or `<content>`; the result arrives as `done.invoke.<id>`) or with a
//! `<send>` whose target is the tool IRI (the result arrives as `vt:reply`). Arguments are
//! checked against the argument type before the tool runs; a failure is `error.tool` with the
//! class `arguments` and the violated paths. Tool results are journaled in the caller's
//! journal.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use vertep_core::datamodel::Value;
use vertep_core::types::{Declarations, Type, parse_declarations};

use crate::llm::ToolSchema;
use crate::schema::{Violation, check_type, type_schema};

/// The function behind a tool.
pub type ToolFn = Arc<dyn Fn(Value) -> BoxFuture<'static, Result<Value, String>> + Send + Sync>;

/// A registered tool.
#[derive(Clone)]
pub struct Tool {
    iri: String,
    description: String,
    decls: Declarations,
    args: Type,
    result: Type,
    args_schema: Json,
    result_schema: Json,
    f: ToolFn,
}

impl std::fmt::Debug for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tool")
            .field("iri", &self.iri)
            .finish_non_exhaustive()
    }
}

/// Why a tool could not be defined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolDefError(pub String);

impl core::fmt::Display for ToolDefError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "invalid tool: {}", self.0)
    }
}

impl std::error::Error for ToolDefError {}

impl Tool {
    /// A tool at `iri` taking `args` and returning `result` (type-language text, with named
    /// types from `declarations`, e.g. `type Hit = { title: string };`), run by `f`.
    ///
    /// # Errors
    ///
    /// [`ToolDefError`] for a malformed type or declaration.
    // @tg: implements:: [[openspec:llm-actors#Tool Actors]]
    pub fn new<F, Fut>(
        iri: impl Into<String>,
        declarations: &str,
        args: &str,
        result: &str,
        f: F,
    ) -> Result<Tool, ToolDefError>
    where
        F: Fn(Value) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Value, String>> + Send + 'static,
    {
        let decls = if declarations.trim().is_empty() {
            Declarations::default()
        } else {
            parse_declarations(declarations).map_err(|e| ToolDefError(e.message))?
        };
        let (args_t, args_schema) = type_schema(&decls, args).map_err(ToolDefError)?;
        let (result_t, result_schema) = type_schema(&decls, result).map_err(ToolDefError)?;
        let f = Arc::new(f);
        Ok(Tool {
            iri: iri.into(),
            description: String::new(),
            decls,
            args: args_t,
            result: result_t,
            args_schema,
            result_schema,
            f: Arc::new(move |v| {
                let f = f.clone();
                Box::pin(async move { f(v).await })
            }),
        })
    }

    /// The tool with a description offered to models.
    pub fn with_description(mut self, text: impl Into<String>) -> Self {
        self.description = text.into();
        self
    }

    /// The tool IRI.
    pub fn iri(&self) -> &str {
        &self.iri
    }

    /// The argument schema.
    pub fn args_schema(&self) -> &Json {
        &self.args_schema
    }

    /// The result schema.
    pub fn result_schema(&self) -> &Json {
        &self.result_schema
    }

    /// The schema offered to a model.
    pub fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.iri.clone(),
            description: self.description.clone(),
            input_schema: self.args_schema.clone(),
        }
    }

    /// Checks `args` against the argument type.
    ///
    /// # Errors
    ///
    /// The violations, with paths.
    pub fn check_args(&self, args: &Value) -> Result<(), Vec<Violation>> {
        check_type(&self.decls, &self.args, args)
    }

    /// Checks a result against the result type.
    ///
    /// # Errors
    ///
    /// The violations, with paths.
    pub fn check_result(&self, result: &Value) -> Result<(), Vec<Violation>> {
        check_type(&self.decls, &self.result, result)
    }

    /// Runs the tool on checked arguments.
    pub fn run(&self, args: Value) -> BoxFuture<'static, Result<Value, String>> {
        (self.f)(args)
    }

    /// Validates `args`, runs the tool and validates its result.
    pub fn call(&self, args: Value) -> BoxFuture<'static, ToolOutcome> {
        if let Err(v) = self.check_args(&args) {
            let out = ToolOutcome::error(ToolErrorClass::Arguments, "invalid arguments", &v);
            return Box::pin(async move { out });
        }
        let run = self.run(args);
        let this = self.clone();
        Box::pin(async move {
            match run.await {
                Ok(v) => match this.check_result(&v) {
                    Ok(()) => ToolOutcome::Ok(v),
                    Err(viol) => ToolOutcome::error(
                        ToolErrorClass::Result,
                        "the tool's result does not have its declared type",
                        &viol,
                    ),
                },
                Err(e) => ToolOutcome::error(ToolErrorClass::Execution, e, &[]),
            }
        })
    }
}

/// The class of a failed tool call, as carried by `error.tool`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolErrorClass {
    /// The arguments do not have the argument type; the tool did not run.
    Arguments,
    /// The tool failed.
    Execution,
    /// The result does not have the result type.
    Result,
    /// No tool is registered at the IRI.
    Unknown,
}

impl ToolErrorClass {
    /// The class name used in events.
    pub fn as_str(self) -> &'static str {
        match self {
            ToolErrorClass::Arguments => "arguments",
            ToolErrorClass::Execution => "execution",
            ToolErrorClass::Result => "result",
            ToolErrorClass::Unknown => "unknown-tool",
        }
    }
}

/// The outcome of a tool call, as journaled.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToolOutcome {
    /// The result.
    Ok(Value),
    /// A failure.
    Err {
        /// The class.
        class: ToolErrorClass,
        /// A description.
        message: String,
        /// The violated paths.
        paths: Vec<String>,
    },
}

impl ToolOutcome {
    /// A failure with violations.
    pub fn error(class: ToolErrorClass, message: impl Into<String>, v: &[Violation]) -> Self {
        ToolOutcome::Err {
            class,
            message: message.into(),
            paths: v.iter().map(|x| x.path.clone()).collect(),
        }
    }
}

/// Tools by IRI.
#[derive(Clone, Debug, Default)]
pub struct ToolRegistry {
    map: BTreeMap<String, Arc<Tool>>,
}

impl ToolRegistry {
    /// Registers `tool` under its IRI.
    pub fn insert(&mut self, tool: Tool) {
        self.map.insert(tool.iri.clone(), Arc::new(tool));
    }

    /// The tool at `iri`.
    pub fn get(&self, iri: &str) -> Option<&Arc<Tool>> {
        self.map.get(iri)
    }

    /// The schemas of the tools among `iris`, in IRI order, skipping unknown ones.
    pub fn schemas<'a, I: IntoIterator<Item = &'a String>>(&self, iris: I) -> Vec<ToolSchema> {
        iris.into_iter()
            .filter_map(|i| self.map.get(i))
            .map(|t| t.schema())
            .collect()
    }
}
