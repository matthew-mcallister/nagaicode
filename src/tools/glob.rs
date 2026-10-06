use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use anyhow::anyhow;
use futures::future::BoxFuture;
use nglob::walker::tokio::glob;
use nglob::{GlobConfig, Pattern};
use serde_json::{Value, json};

use crate::cwd::Cwd;
use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::tools::{InterfaceToolOutput, Tool};
use crate::ui::render_item::{HelpRenderItem, RenderItem};

/// Searches for files and directories using glob patterns.
#[derive(Debug)]
pub struct GlobTool {
    cwd: Arc<Cwd>,
    input_schema: Value,
}

impl GlobTool {
    /// Creates a tool that searches for files, rooting relative patterns at
    /// `cwd`.
    pub fn new(cwd: Arc<Cwd>) -> Self {
        Self {
            cwd,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string" },
                    "match": { "enum": ["files", "directories", "all"] },
                    "max_results": { "type": "integer", "minimum": 1 },
                },
                "required": ["pattern", "match", "max_results"],
                "additionalProperties": false,
            }),
        }
    }
}

impl DataQuery for GlobTool {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "name": self.name(),
                "is_visible": self.is_visible(),
            }))),
            "name" => Ok(QueryField::Value(json!(self.name()))),
            "is_visible" => Ok(QueryField::Value(json!(self.is_visible()))),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Searches for files using glob syntax:\n\
        - `\\`: escape character\n\
        - `{a,b,c}`: alternatives\n\
        - `?`: match any one character\n\
        - `*`: match zero or more characters except path separators\n\
        - `**` match zero or more characters, including path separators"
    }

    fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    fn call<'a>(&'a self, input: &'a Value) -> BoxFuture<'a, AnyResult<Value>> {
        Box::pin(async move {
            let invalid = || anyhow!("arguments don't match schema");
            let pattern = input
                .get("pattern")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let match_type = input
                .get("match")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let max_results = input
                .get("max_results")
                .and_then(Value::as_u64)
                .ok_or_else(invalid)?;

            let config = match match_type {
                "files" => GlobConfig::new()
                    .with_match_files(true)
                    .with_match_directories(false)
                    .with_match_other(true),
                "directories" => GlobConfig::new()
                    .with_match_files(false)
                    .with_match_directories(true)
                    .with_match_other(false),
                "all" => GlobConfig::new()
                    .with_match_files(true)
                    .with_match_directories(true)
                    .with_match_other(true),
                _ => return Err(invalid()),
            };

            let rooted = if Path::new(pattern).is_absolute() {
                pattern.to_owned()
            } else {
                let mut rooted = escape_glob(&self.cwd.to_string_lossy());
                if !rooted.ends_with(std::path::MAIN_SEPARATOR) {
                    rooted.push(std::path::MAIN_SEPARATOR);
                }
                rooted.push_str(pattern);
                rooted
            };
            let compiled = Pattern::compile(&rooted)
                .map_err(|e| anyhow!("invalid glob pattern '{pattern}': {e}"))?;

            let result = glob(&config, &compiled).await;
            let mut results = Vec::with_capacity(result.results().len());
            for entry in result.results() {
                match entry {
                    Ok(entry) => {
                        let path = display_path(&**self.cwd, &entry.path);
                        if path.is_empty() {
                            continue;
                        }
                        results.push(json!({ "path": path }));
                    }
                    Err(e) => results.push(json!({ "error": e.to_string() })),
                }
            }

            let truncated = results.len() > max_results as usize;
            results.truncate(max_results as usize);

            let mut output = json!({ "results": results });
            if truncated {
                output["truncated"] = json!(true);
            }
            Ok(output)
        })
    }

    fn render_to_ui(&self, input: &Value, _output: &Value) -> AnyResult<Box<dyn RenderItem>> {
        let pattern = input
            .get("pattern")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool input"))?;
        Ok(Box::new(HelpRenderItem::new(format!("Glob {pattern}"))))
    }

    // Output looks like this:
    // ```
    // file1.txt
    // file2.txt
    // error: Error 1
    // dir1/
    // dir2/
    // dir3/file3.txt
    // error: Error 2
    // truncated at 7 results
    // ```
    fn render_to_interface(
        &self,
        input: &Value,
        output: &Value,
    ) -> AnyResult<InterfaceToolOutput> {
        let results = output
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let mut lines = Vec::with_capacity(results.len() + 1);
        for entry in results {
            if let Some(path) = entry.get("path").and_then(Value::as_str) {
                lines.push(path.to_owned());
            } else if let Some(error) = entry.get("error").and_then(Value::as_str) {
                lines.push(format!("error: {error}"));
            }
        }
        if output.get("truncated").and_then(Value::as_bool).unwrap_or(false) {
            let max_results = input.get("max_results").and_then(Value::as_u64).unwrap_or(0);
            lines.push(format!("truncated at {max_results} results"));
        }
        Ok(InterfaceToolOutput {
            content: vec![ToolOutputContent::Text { text: Cow::Owned(lines.join("\n")) }],
        })
    }
}

/// Escapes glob metacharacters in a literal path.
fn escape_glob(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '?' | '*' | '{' | '}' | ',') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Strips `cwd` from a matched path, preserving nglob's trailing separator on
/// directories. Paths outside `cwd` are returned unchanged.
fn display_path(cwd: &Path, path: &str) -> String {
    let is_dir = path.ends_with(std::path::MAIN_SEPARATOR);
    match Path::new(path).strip_prefix(cwd) {
        Ok(rel) => {
            let mut s = rel.display().to_string();
            if is_dir && !s.is_empty() {
                s.push(std::path::MAIN_SEPARATOR);
            }
            s
        }
        Err(_) => path.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cwd::cwd;

    fn write(dir: &Cwd, name: &str) {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();
    }

    async fn glob_paths(tool: &GlobTool, match_type: &str) -> Vec<String> {
        let out = tool
            .call(&json!({ "pattern": "**", "match": match_type, "max_results": 100 }))
            .await
            .unwrap();
        let mut paths: Vec<String> = out["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["path"].as_str().unwrap().to_owned())
            .collect();
        paths.sort();
        paths
    }

    #[tokio::test]
    async fn test_glob() {
        let dir = Arc::new(cwd());
        write(&dir, "a.txt");
        write(&dir, "src/b.jpg");
        write(&dir, "src/c.txt");
        let tool = GlobTool::new(dir.clone());

        assert_eq!(
            glob_paths(&tool, "files").await,
            ["a.txt", "src/b.jpg", "src/c.txt"]
        );
        assert_eq!(glob_paths(&tool, "directories").await, ["src/"]);
        assert_eq!(
            glob_paths(&tool, "all").await,
            ["a.txt", "src/", "src/b.jpg", "src/c.txt"]
        );
    }
}

