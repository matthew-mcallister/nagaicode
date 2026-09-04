use std::borrow::Cow;
use std::sync::Arc;

use anyhow::anyhow;
use futures::future::BoxFuture;
use serde_json::{Value, json};

use crate::cwd::Cwd;
use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::tools::{InterfaceToolOutput, Tool};
use crate::ui::markdown::ResumePoint;
use crate::ui::render_item::RenderItem;
use crate::ui::style::{Style, Theme};
use crate::ui::styled_string::StyledString;
use crate::ui::text::{Row, SPACES, ellipsize, wrap_line_naive};

/// Runs shell commands on the host system.
#[derive(Debug)]
pub struct ShTool {
    cwd: Arc<Cwd>,
    input_schema: Value,
}

impl ShTool {
    /// Creates a tool that runs shell commands within `cwd`.
    pub fn new(cwd: Arc<Cwd>) -> Self {
        Self {
            cwd,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" },
                },
                "required": ["command"],
                "additionalProperties": false,
            }),
        }
    }
}

impl DataQuery for ShTool {
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

impl Tool for ShTool {
    fn name(&self) -> &str {
        "sh"
    }

    fn description(&self) -> &str {
        "Run a shell command on the host system. Equivalent to `sh -C 'command'`."
    }

    fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    fn call<'a>(&'a self, input: &'a Value) -> BoxFuture<'a, AnyResult<Value>> {
        Box::pin(async move {
            let cmd = input
                .get("command")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("invalid arguments for 'sh': expected {{\"command\": \"...\"}}"))?;
            let output = tokio::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .current_dir(&**self.cwd)
                .output()
                .await
                .map_err(|e| anyhow!("failed to run 'sh': {e}"))?;
            let return_code = output.status.code().unwrap_or(-1);
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            Ok(json!({
                "stdout": stdout,
                "stderr": stderr,
                "return_code": return_code,
            }))
        })
    }

    fn render_to_ui(&self, input: &Value, output: &Value) -> AnyResult<Box<dyn RenderItem>> {
        Ok(Box::new(ShRenderItem::from_output(input, output)?))
    }

    fn render_to_interface(
        &self,
        _input: &Value,
        output: &Value,
    ) -> AnyResult<InterfaceToolOutput> {
        let stdout = output.get("stdout").and_then(Value::as_str).unwrap_or("");
        let stderr = output.get("stderr").and_then(Value::as_str).unwrap_or("");
        let return_code = output.get("return_code").and_then(Value::as_i64).unwrap_or(-1);
        let mut contents = Vec::new();
        if !stdout.is_empty() {
            contents.push(ToolOutputContent::Text {
                text: Cow::Owned(format!("stdout:\n{stdout}")),
            });
        }
        if !stderr.is_empty() {
            contents.push(ToolOutputContent::Text {
                text: Cow::Owned(format!("stderr:\n{stderr}")),
            });
        }
        contents.push(ToolOutputContent::Text {
            text: Cow::Owned(format!("return code: {return_code}")),
        });
        Ok(InterfaceToolOutput { content: contents })
    }
}

/// Renders shell call output
#[derive(Debug)]
pub struct ShRenderItem {
    cmd_line: String,
    stdout: String,
}

impl ShRenderItem {
    fn from_output(input: &Value, output: &Value) -> AnyResult<Self> {
        let cmd_line = input
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool input"))?
            .to_owned();
        let stdout = output
            .get("stdout")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool output"))?
            .to_owned();
        Ok(Self { cmd_line, stdout })
    }
}

impl DataQuery for ShRenderItem {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "type": self.query("/type")?,
                "cmd_line": self.query("/cmd_line")?,
                "stdout": self.query("/stdout")?,
            }))),
            "type" => Ok(QueryField::Value(json!("sh"))),
            "cmd_line" => Ok(QueryField::Value(self.cmd_line.clone().into())),
            "stdout" => Ok(QueryField::Value(self.stdout.clone().into())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

impl RenderItem for ShRenderItem {
    fn render(
        &self,
        theme: &Theme,
        width: usize,
        _resume_point: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint) {
        let rows = render_sh_stdout(theme, width, &self.cmd_line, &self.stdout);
        (rows, Default::default())
    }
}

const PADDING: usize = 2;
const MAX_ROWS: usize = 11;

fn render_sh_stdout(
    theme: &Theme,
    width: usize,
    cmd_line: &str,
    stdout: &str,
) -> Vec<StyledString> {
    if stdout.is_empty() {
        return Vec::new();
    }

    if width < 8 {
        return vec![];
    }

    let style = Style::new(theme.text_base, theme.bg_prompt);
    let ellipsis_style = Style::new(theme.text_subtle, theme.bg_prompt);

    let mut padding = StyledString::new(style, width);
    padding.push(&SPACES[..width], width);

    let inner_width = width - 2 * PADDING;

    let mut rows = vec![padding.clone()];

    let push_rows = |rows: &mut Vec<StyledString>, new_rows: &[Row]| {
        for row in new_rows {
            let mut s = StyledString::new(style, 2 * row.graphemes.len());
            s.push(&SPACES[..PADDING], PADDING);
            for g in &row.graphemes {
                s.push(g.formatted(), g.width as usize);
            }
            s.pad_to_width(width - PADDING);
            s.push(&SPACES[..PADDING], PADDING);
            rows.push(s);
        }
    };

    // Command line
    let prompt = format!("$ {cmd_line}");
    push_rows(&mut rows, &wrap_line_naive(inner_width, &prompt));

    // Output
    let (head, tail) = ellipsize(inner_width, MAX_ROWS, stdout);
    push_rows(&mut rows, &head);
    if let Some(tail) = tail {
        let mut s = StyledString::new(ellipsis_style, width);
        s.push(&SPACES[..PADDING], PADDING);
        s.push("...", 3);
        s.push(&SPACES[..inner_width - 3], inner_width - 3);
        s.push(&SPACES[..PADDING], PADDING);
        rows.push(s);
        push_rows(&mut rows, &tail);
    }

    rows.push(padding);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::QueryError;
    use crate::cwd::cwd;
    use crate::ui::canvas::render_canvas;
    use crate::ui::style::THEME_DARK;

    fn render(width: usize, input: &Value, output: &Value) -> AnyResult<String> {
        let tool = ShTool::new(Arc::new(cwd()));
        let item = tool.render_to_ui(input, output)?;
        let (mut lines, _) = item.render(&THEME_DARK, width, Default::default());
        Ok(render_canvas(&mut lines[..]))
    }

    #[test]
    fn test_sh_render_item_query() {
        let item = ShRenderItem {
            cmd_line: "echo hi".into(),
            stdout: "hello\n".into(),
        };
        let expected = json!({
            "type": "sh",
            "cmd_line": "echo hi",
            "stdout": "hello\n",
        });
        assert_eq!(item.query("/").unwrap(), expected);
        assert_eq!(item.query("/type").unwrap(), json!("sh"));
        assert_eq!(item.query("/cmd_line").unwrap(), json!("echo hi"));
        assert_eq!(item.query("/stdout").unwrap(), json!("hello\n"));
        assert!(matches!(item.query("/missing"), Err(QueryError::InvalidField(_))));
    }

    #[test]
    fn test_sh_renderer() {
        let theme = &THEME_DARK;
        let style = Style::new(theme.text_base, theme.bg_prompt);
        let ok_output = || json!({"stdout": "", "stderr": "", "return_code": 0});

        let output = json!({"stdout": "hello\n", "stderr": "", "return_code": 0});
        assert_eq!(
            render(14, &json!({"command": "echo hi"}), &output).unwrap(),
            format!(
                "{style}              \n{style}  $ echo hi   \n{style}  hello       \n{style}              "
            )
        );
        let output = json!({"stdout": "hi\n", "stderr": "", "return_code": 0});
        assert_eq!(
            render(14, &json!({"command": "echo hello world"}), &output).unwrap(),
            format!(
                "{style}              \n{style}  $ echo hel  \n{style}  lo world    \n{style}  hi          \n{style}              "
            )
        );

        // Long stdout is ellipsized; the final row is filled to the full width
        let ellipsis_style = Style::new(theme.text_subtle, theme.bg_prompt);
        let output = json!({"stdout": "x".repeat(120), "stderr": "", "return_code": 0});
        let line = || format!("{style}  xxxxxxxxxx  ");
        assert_eq!(
            render(14, &json!({"command": "echo hi"}), &output).unwrap(),
            [
                format!("{style}              "),
                format!("{style}  $ echo hi   "),
                line(), line(), line(), line(), line(),
                format!("{ellipsis_style}  ...         "),
                line(), line(), line(), line(), line(),
                format!("{style}              "),
            ].join("\n")
        );

        assert_eq!(
            render(14, &json!({"command": "echo hi"}), &ok_output()).unwrap(),
            ""
        );
        assert_eq!(
            render(7, &json!({"command": "echo hi"}), &output).unwrap(),
            ""
        );

        assert!(render(14, &json!({}), &ok_output()).is_err());
        assert!(render(14, &json!({"command": "echo hi"}), &json!({})).is_err());
    }

    #[tokio::test]
    async fn test_sh_tool() {
        let dir = Arc::new(cwd());
        let tool = ShTool::new(dir.clone());

        assert_eq!(tool.name(), "sh");
        assert_eq!(
            tool.input_schema(),
            &json!({
                "type": "object",
                "properties": { "command": { "type": "string" } },
                "required": ["command"],
                "additionalProperties": false,
            })
        );

        assert!(tool.call(&json!({})).await.is_err());
        assert!(tool.call(&json!({"command": 123})).await.is_err());

        let out = tool.call(&json!({"command": "printf 'hello'"})).await.unwrap();
        assert_eq!(
            out,
            json!({"stdout": "hello", "stderr": "", "return_code": 0})
        );

        let err = tool.call(&json!({"command": "printf 'err' >&2; exit 1"})).await.unwrap();
        assert_eq!(err["stderr"], json!("err"));
        assert_eq!(err["return_code"], json!(1));

        // Commands run within the tool's cwd.
        let out = tool.call(&json!({"command": "pwd"})).await.unwrap();
        let pwd = out["stdout"].as_str().unwrap().trim();
        let expected = std::fs::canonicalize(dir.path()).unwrap();
        assert_eq!(pwd, expected.to_string_lossy().as_ref());

        // Interface content is built from stdout, stderr, and return code.
        let input = json!({});
        let output = json!({"stdout": "hi", "stderr": "", "return_code": 0});
        let result = tool.render_to_interface(&input, &output).unwrap();
        assert_eq!(
            result.content,
            vec![
                ToolOutputContent::Text { text: Cow::Owned("stdout:\nhi".into()) },
                ToolOutputContent::Text { text: Cow::Owned("return code: 0".into()) },
            ]
        );
    }
}