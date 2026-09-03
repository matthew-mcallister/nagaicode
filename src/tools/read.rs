use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use anyhow::anyhow;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::cwd::Cwd;
use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::tools::{InterfaceToolOutput, Tool};
use crate::ui::render_item::{ErrorRenderItem, HelpRenderItem, RenderItem};

const MAX_LINE_BYTES: usize = 2000;
const TRUNCATION_SUFFIX: &str = "... (truncated at 2000 bytes)";

/// Reads lines from a UTF-8 text file.
#[derive(Debug)]
pub struct ReadTool {
    cwd: Arc<Cwd>,
    input_schema: Value,
}

impl ReadTool {
    /// Creates a tool that reads text files, rendering paths relative to
    /// `cwd`.
    pub fn new(cwd: Arc<Cwd>) -> Self {
        Self {
            cwd,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "filepath": { "type": "string" },
                    "start_line": { "type": "integer", "minimum": 1 },
                    "max_lines": { "type": "integer", "minimum": 1 },
                },
                "required": ["filepath", "start_line", "max_lines"],
                "additionalProperties": false,
            }),
        }
    }
}

impl DataQuery for ReadTool {
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

impl Tool for ReadTool {
    fn name(&self) -> &str {
        "read"
    }

    fn description(&self) -> &str {
        "Reads lines from a text file. Start line is 1-indexed. Output \
        includes next line offset for pagination. Lines truncated at 2000 \
        bytes max. UTF-8 only currently."
    }

    fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    fn call<'a>(&'a self, input: &'a Value) -> BoxFuture<'a, AnyResult<Value>> {
        Box::pin(async move {
            let invalid = || anyhow!("arguments don't match schema");
            let filepath = input
                .get("filepath")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let start_line = input
                .get("start_line")
                .and_then(Value::as_i64)
                .ok_or_else(invalid)?;
            let max_lines = input
                .get("max_lines")
                .and_then(Value::as_i64)
                .ok_or_else(invalid)?;
            if max_lines < 1 {
                return Err(anyhow!("max_lines mut be at least 1"));
            }

            let bytes = std::fs::read(filepath)?;
            let text = String::from_utf8(bytes)?;

            let lines: Vec<&str> = text.split_inclusive('\n').collect();
            let start = start_line.max(1) as usize - 1;
            if start > 0 && start >= lines.len() {
                return Err(anyhow!(
                    "'{filepath}': start line {start_line} is past end of file ({} lines)",
                    lines.len()
                ));
            }
            let end = (start + max_lines as usize).min(lines.len());

            let mut content = String::new();
            for line in &lines[start..end] {
                push_line(&mut content, line.strip_suffix('\n').unwrap_or(line));
                content.push('\n');
            }

            let mut output = json!({
                "content": BASE64_STANDARD.encode(&content),
                "num_lines": end - start,
            });
            if end < lines.len() {
                output["next_line"] = json!(end + 1);
            }
            Ok(output)
        })
    }

    fn render_to_ui(&self, input: &Value, output: &Value) -> AnyResult<Box<dyn RenderItem>> {
        if let Some(error) = output.get("error").and_then(Value::as_str) {
            return Ok(Box::new(ErrorRenderItem::new(error.to_owned())));
        }
        let filepath = input
            .get("filepath")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool input"))?;
        let num_lines = output
            .get("num_lines")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let path = display_path(&self.cwd, Path::new(filepath));
        let start_line = input.get("start_line").and_then(Value::as_i64).unwrap_or(1);
        let message = if start_line <= 1 {
            format!("Read {path} ({num_lines} lines)")
        } else {
            format!("Read {path} (start={start_line}, {num_lines} lines)")
        };
        Ok(Box::new(HelpRenderItem::new(message)))
    }

    fn render_to_interface(
        &self,
        input: &Value,
        output: &Value,
    ) -> AnyResult<InterfaceToolOutput> {
        if let Some(error) = output.get("error").and_then(Value::as_str) {
            return Ok(InterfaceToolOutput {
                name: self.name().to_owned(),
                content: vec![ToolOutputContent::Text {
                    text: Cow::Owned(format!("error: {error}")),
                }],
            });
        }
        let filepath = input
            .get("filepath")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool input"))?;
        let content = output
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let num_lines = output
            .get("num_lines")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let next_line = match output.get("next_line").and_then(Value::as_i64) {
            Some(next_line) => format!("next line: {next_line}"),
            None => "reached end of file".to_owned(),
        };
        Ok(InterfaceToolOutput {
            name: self.name().to_owned(),
            content: vec![
                ToolOutputContent::Text {
                    text: Cow::Owned(format!("read {num_lines} lines\n{next_line}")),
                },
                ToolOutputContent::File {
                    filepath: Cow::Owned(filepath.to_owned()),
                    data: Cow::Owned(content.to_owned()),
                },
            ],
        })
    }
}

fn push_line(out: &mut String, line: &str) {
    if line.len() <= MAX_LINE_BYTES {
        out.push_str(line);
        return;
    }
    let mut end = MAX_LINE_BYTES;
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    out.push_str(&line[..end]);
    out.push_str(TRUNCATION_SUFFIX);
}

fn display_path(cwd: &Path, path: &Path) -> String {
    path.strip_prefix(cwd).unwrap_or(path).display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cwd::cwd;
    use crate::query::QueryError;
    use crate::ui::canvas::render_canvas;
    use crate::ui::render_item::{render_error, render_help};
    use crate::ui::style::THEME_DARK;

    fn write(dir: &Cwd, name: &str, contents: impl AsRef<[u8]>) -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, contents).expect("write file");
        path.to_string_lossy().into_owned()
    }

    fn decode(output: &Value) -> String {
        let encoded = output["content"].as_str().expect("base64 content");
        let bytes = BASE64_STANDARD.decode(encoded).expect("valid base64");
        String::from_utf8(bytes).expect("valid utf-8")
    }

    fn long_line(len: usize) -> String {
        "a".repeat(len)
    }

    #[tokio::test]
    async fn test_read_tool() {
        let dir = Arc::new(cwd());
        let path = write(&dir, "a.txt", "one\ntwo\nthree\n");
        let tool = ReadTool::new(dir.clone());

        assert_eq!(tool.name(), "read");
        assert!(tool.is_visible());
        assert_eq!(
            tool.input_schema(),
            &json!({
                "type": "object",
                "properties": {
                    "filepath": { "type": "string" },
                    "start_line": { "type": "integer" },
                    "max_lines": { "type": "integer" },
                },
                "required": ["filepath", "start_line", "max_lines"],
                "additionalProperties": false,
            })
        );

        // Whole file: no next_line since the last line was read.
        let out = tool.call(&json!({ "filepath": path, "start_line": 1, "max_lines": 10 }))
            .await
            .unwrap();
        assert_eq!(decode(&out), "one\ntwo\nthree\n");
        assert_eq!(out["num_lines"], json!(3));
        assert_eq!(out.get("next_line"), None);

        // Pagination reports the line after the last line read.
        let out = tool.call(&json!({ "filepath": path, "start_line": 1, "max_lines": 2 }))
            .await
            .unwrap();
        assert_eq!(decode(&out), "one\ntwo\n");
        assert_eq!(out["num_lines"], json!(2));
        assert_eq!(out["next_line"], json!(3));

        let out = tool.call(&json!({ "filepath": path, "start_line": 3, "max_lines": 2 }))
            .await
            .unwrap();
        assert_eq!(decode(&out), "three\n");
        assert_eq!(out["num_lines"], json!(1));
        assert_eq!(out.get("next_line"), None);

        // The final line is reported as EOF, not as another line to read.
        let out = tool.call(&json!({ "filepath": path, "start_line": 2, "max_lines": 2 }))
            .await
            .unwrap();
        assert_eq!(decode(&out), "two\nthree\n");
        assert_eq!(out.get("next_line"), None);

        // A missing trailing newline is added to the output.
        let no_newline = write(&dir, "b.txt", "x\ny");
        let out = tool.call(&json!({ "filepath": no_newline, "start_line": 1, "max_lines": 5 }))
            .await
            .unwrap();
        assert_eq!(decode(&out), "x\ny\n");
        assert_eq!(out["num_lines"], json!(2));
        assert_eq!(out.get("next_line"), None);

        // Empty files read as zero lines.
        let empty = write(&dir, "empty.txt", "");
        let out = tool.call(&json!({ "filepath": empty, "start_line": 1, "max_lines": 5 }))
            .await
            .unwrap();
        assert_eq!(decode(&out), "");
        assert_eq!(out["num_lines"], json!(0));
        assert_eq!(out.get("next_line"), None);

        // Reading past EOF is an error, but starting at the end is not.
        assert!(tool.call(&json!({ "filepath": path, "start_line": 4, "max_lines": 2 })).await.is_err());
        assert!(tool.call(&json!({ "filepath": empty, "start_line": 2, "max_lines": 2 })).await.is_err());

        // Start line 0 is treated as the first line.
        let out = tool.call(&json!({ "filepath": path, "start_line": 0, "max_lines": 1 }))
            .await
            .unwrap();
        assert_eq!(decode(&out), "one\n");
        assert_eq!(out["next_line"], json!(2));

        // Invalid arguments and unreadable files are errors.
        assert!(tool.call(&json!({})).await.is_err());
        assert!(tool.call(&json!({ "filepath": path, "start_line": "1", "max_lines": 1 })).await.is_err());
        assert!(tool.call(&json!({ "filepath": path, "start_line": 1, "max_lines": 0 })).await.is_err());
        assert!(tool.call(&json!({ "filepath": dir.path().join("missing.txt"), "start_line": 1, "max_lines": 1 })).await.is_err());

        let binary = write(&dir, "bin.txt", [0x00, 0xff, 0xfe]);
        let error = tool.call(&json!({ "filepath": binary, "start_line": 1, "max_lines": 1 }))
            .await
            .expect_err("invalid utf-8");
        assert!(error.to_string().contains("not valid UTF-8"), "{error}");
    }

    #[tokio::test]
    async fn test_read_tool_truncates_lines() {
        let dir = Arc::new(cwd());
        let tool = ReadTool::new(dir.clone());

        // Long lines are truncated at a byte boundary.
        let path = write(&dir, "long.txt", long_line(MAX_LINE_BYTES + 1000));
        let out = tool.call(&json!({ "filepath": path, "start_line": 1, "max_lines": 1 }))
            .await
            .unwrap();
        let content = decode(&out);
        assert_eq!(out["num_lines"], json!(1));
        assert_eq!(
            content,
            format!("{}{TRUNCATION_SUFFIX}\n", long_line(MAX_LINE_BYTES))
        );

        // Truncation backs up to the nearest codepoint boundary.
        let multibyte = format!("{}あ{}", long_line(MAX_LINE_BYTES - 2), long_line(10));
        assert!(!multibyte.is_char_boundary(MAX_LINE_BYTES));
        let path = write(&dir, "multibyte.txt", &multibyte);
        let out = tool.call(&json!({ "filepath": path, "start_line": 1, "max_lines": 1 }))
            .await
            .unwrap();
        assert_eq!(
            decode(&out),
            format!("{}{TRUNCATION_SUFFIX}\n", long_line(MAX_LINE_BYTES - 2))
        );

        // Lines exactly at the limit are not truncated.
        let path = write(&dir, "exact.txt", long_line(MAX_LINE_BYTES));
        let out = tool.call(&json!({ "filepath": path, "start_line": 1, "max_lines": 1 }))
            .await
            .unwrap();
        assert_eq!(decode(&out), format!("{}\n", long_line(MAX_LINE_BYTES)));
    }

    #[test]
    fn test_read_render_ui() {
        let dir = Arc::new(cwd());
        let tool = ReadTool::new(dir.clone());
        let theme = &THEME_DARK;

        let render = |input: &Value, output: &Value| -> String {
            let item = tool.render_to_ui(input, output).expect("render item");
            let (mut lines, _) = item.render(theme, 20, Default::default());
            render_canvas(&mut lines[..])
        };

        // Paths under cwd are rendered relative to it.
        let input = json!({ "filepath": dir.path().join("a.txt").to_string_lossy(), "start_line": 1, "max_lines": 10 });
        let output = json!({ "content": "", "num_lines": 3 });
        let item = tool.render_to_ui(&input, &output).unwrap();
        assert_eq!(item.query("/type").unwrap(), json!("help"));
        assert_eq!(item.query("/value").unwrap(), json!("Read a.txt (3 lines)"));
        let mut expected = render_help(theme, 20, "Read a.txt (3 lines)");
        let (mut lines, _) = item.render(theme, 20, Default::default());
        assert_eq!(render_canvas(&mut lines[..]), render_canvas(&mut expected[..]));

        // Paginated reads include the start line.
        let mut input = json!({ "filepath": dir.path().join("a.txt").to_string_lossy(), "start_line": 3, "max_lines": 10 });
        assert_eq!(
            render(&input, &output),
            render_canvas(&mut render_help(theme, 20, "Read a.txt (start=3, 3 lines)")[..])
        );

        // Paths outside cwd are rendered as-is.
        input["filepath"] = json!("/elsewhere/b.txt");
        assert_eq!(
            render(&input, &output),
            render_canvas(&mut render_help(theme, 20, "Read /elsewhere/b.txt (start=3, 3 lines)")[..])
        );

        // Errors render as error messages.
        let error_output = json!({ "error": "no such file" });
        let item = tool.render_to_ui(&input, &error_output).unwrap();
        assert_eq!(item.query("/type").unwrap(), json!("error"));
        let (mut lines, _) = item.render(theme, 20, Default::default());
        let mut expected = render_error(theme, 20, "no such file");
        assert_eq!(render_canvas(&mut lines[..]), render_canvas(&mut expected[..]));

        // Incomplete input or output is rejected.
        assert!(tool.render_to_ui(&json!({}), &output).is_err());
        assert!(tool.render_to_ui(&input, &json!({})).is_err());
    }

    #[test]
    fn test_read_render_interface() {
        let dir = Arc::new(cwd());
        let tool = ReadTool::new(dir.clone());
        let path = dir.path().join("a.txt").to_string_lossy().into_owned();

        let input = json!({ "filepath": path, "start_line": 1, "max_lines": 2 });
        let output = json!({ "content": BASE64_STANDARD.encode("one\ntwo\n"), "num_lines": 2, "next_line": 3 });
        let result = tool.render_to_interface(&input, &output).unwrap();
        assert_eq!(result.name, "read");
        assert_eq!(
            result.content,
            vec![
                ToolOutputContent::Text { text: Cow::Owned("read 2 lines\nnext line: 3".into()) },
                ToolOutputContent::File {
                    filepath: Cow::Owned(path.clone()),
                    data: Cow::Owned(BASE64_STANDARD.encode("one\ntwo\n")),
                },
            ]
        );

        // Reaching EOF is reported instead of a next line.
        let output = json!({ "content": "", "num_lines": 2 });
        let result = tool.render_to_interface(&input, &output).unwrap();
        assert_eq!(
            result.content,
            vec![
                ToolOutputContent::Text { text: Cow::Owned("read 2 lines\nreached end of file".into()) },
                ToolOutputContent::File { filepath: Cow::Owned(path.clone()), data: Cow::Owned("".into()) },
            ]
        );

        // Errors render as a single text item.
        let result = tool.render_to_interface(&input, &json!({ "error": "no such file" })).unwrap();
        assert_eq!(
            result.content,
            vec![ToolOutputContent::Text { text: Cow::Owned("error: no such file".into()) }]
        );

        assert!(tool.render_to_interface(&json!({}), &output).is_err());
        assert!(tool.render_to_interface(&input, &json!({})).is_err());
    }

    #[test]
    fn test_query() {
        let tool = ReadTool::new(Arc::new(cwd()));
        assert_eq!(tool.query("/").unwrap(), json!({ "name": "read", "is_visible": true }));
        assert_eq!(tool.query("/name").unwrap(), json!("read"));
        assert_eq!(tool.query("/is_visible").unwrap(), json!(true));
        assert!(matches!(tool.query("/missing"), Err(QueryError::InvalidField(_))));
    }
}
