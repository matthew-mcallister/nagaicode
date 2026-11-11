use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

use anyhow::anyhow;
use futures::future::BoxFuture;
use serde_json::{Value, json};
use similar::TextDiff;

use crate::cwd::Cwd;
use crate::error::AnyResult;
use crate::interface::ToolOutputContent;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::tools::{InterfaceToolOutput, Tool, ToolCallContext, ToolCreateArgs};
use crate::ui::markdown::ResumePoint;
use crate::ui::render_item::{ErrorRenderItem, RenderItem};
use crate::ui::style::{Style, Theme};
use crate::ui::styled_string::StyledString;
use crate::ui::text::{Row, SPACES, ellipsize, wrap_line_naive};

/// Lines of context included around each change in the diff.
const CONTEXT_RADIUS: usize = 3;
const PADDING: usize = 2;
const MAX_ROWS: usize = 11;

/// How many bytes to inspect during line ending detection
const LINE_ENDING_WINDOW: usize = 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LineEnding {
    Lf,
    Crlf,
}

impl LineEnding {
    const SYSTEM: Self = if cfg!(windows) {
        Self::Crlf
    } else {
        Self::Lf
    };

    fn name(self) -> &'static str {
        match self {
            Self::Lf => "lf",
            Self::Crlf => "crlf",
        }
    }

    fn normalize(self, s: &str) -> Cow<'_, str> {
        match self {
            Self::Lf if s.contains("\r\n") => Cow::Owned(s.replace("\r\n", "\n")),
            Self::Crlf if s.contains('\n') => Cow::Owned(s.replace('\n', "\r\n")),
            _ => Cow::Borrowed(s),
        }
    }
}

impl std::fmt::Display for LineEnding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

impl Default for LineEnding {
    fn default() -> Self {
        Self::SYSTEM
    }
}

/// Replaces occurrences of a string within a text file.
#[derive(Debug)]
pub struct EditTool {
    cwd: Arc<Cwd>,
    input_schema: Value,
}

impl EditTool {
    pub fn new(args: &ToolCreateArgs<'_>) -> Self {
        Self {
            cwd: Arc::clone(args.cwd),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "filepath": { "type": "string" },
                    "old_string": { "type": "string", "minLength": 1 },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" },
                },
                "required": ["filepath", "old_string", "new_string", "replace_all"],
                "additionalProperties": false,
            }),
        }
    }
}

impl DataQuery for EditTool {
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

impl Tool for EditTool {
    fn name(&self) -> &str {
        "edit"
    }

    fn description(&self) -> &str {
        "Finds and replaces text in a file. The old string must be unique \
        unless `replace_all` is true."
    }

    fn input_schema(&self) -> &Value {
        &self.input_schema
    }

    fn call<'a>(
        &'a self,
        _context: &'a ToolCallContext,
        input: &'a Value,
    ) -> BoxFuture<'a, AnyResult<Value>> {
        Box::pin(async move {
            let invalid = || anyhow!("arguments don't match schema");
            let filepath = input
                .get("filepath")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let old_string = input
                .get("old_string")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let new_string = input
                .get("new_string")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let replace_all = input
                .get("replace_all")
                .and_then(Value::as_bool)
                .ok_or_else(invalid)?;

            if old_string.is_empty() {
                return Err(anyhow!("old_string must not be empty"));
            }

            let text = std::fs::read_to_string(filepath)
                .map_err(|e| anyhow!("{filepath}: {e}"))?;

            // Models don't natively handle CRLF so do conversion if needed
            let detected = detect_line_ending(text.as_bytes());
            let line_ending = detected.unwrap_or_default();
            let old_string = line_ending.normalize(old_string);
            let new_string = line_ending.normalize(new_string);

            let matches = text.match_indices(old_string.as_ref()).count();
            if !replace_all {
                if matches > 1 {
                    return Err(anyhow!(
                        "{filepath}: replacement failed: old string not unique, \
                        try again with more context"
                    ));
                }
                if matches == 0 {
                    return Err(anyhow!(
                        "{filepath}: replacement failed: no matches found, \
                        file may have changed"));
                }
            }

            let edited = if replace_all {
                text.replace(old_string.as_ref(), new_string.as_ref())
            } else {
                text.replacen(old_string.as_ref(), new_string.as_ref(), 1)
            };
            let diff = unified_diff(filepath, &text, &edited);
            if edited != text {
                std::fs::write(filepath, &edited).map_err(|e| anyhow!("{filepath}: {e}"))?;
            }

            Ok(json!({
                "matches": matches,
                "line_ending": match detected {
                    Some(le) => Value::String(le.name().into()),
                    None => Value::Null,
                },
                "diff": diff,
            }))
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
        let diff = output
            .get("diff")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let path = display_path(&self.cwd, Path::new(filepath));
        Ok(Box::new(EditRenderItem {
            header: format!("Edited {path}"),
            diff: diff.to_owned(),
        }))
    }

    fn render_to_interface(
        &self,
        _input: &Value,
        output: &Value,
    ) -> AnyResult<InterfaceToolOutput> {
        if let Some(error) = output.get("error").and_then(Value::as_str) {
            return Ok(InterfaceToolOutput {
                content: vec![ToolOutputContent::Text {
                    text: Cow::Owned(format!("error: {error}")),
                }],
            });
        }
        let matches = output
            .get("matches")
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow!("invalid tool output"))?;
        let line_ending = output
            .get("line_ending")
            .and_then(Value::as_str);
        let message = if let Some(le) = line_ending {
            format!("detected line ending '{le}'\nmatches replaced: {matches}")
        } else {
            let le = LineEnding::SYSTEM;
            format!("unknown line ending, default to '{le}'\nmatches replaced: {matches}")
        };
        Ok(InterfaceToolOutput {
            content: vec![ToolOutputContent::Text {
                text: Cow::Owned(message),
            }],
        })
    }
}

fn detect_line_ending(text: &[u8]) -> Option<LineEnding> {
    // Search tail before head
    let tail_start = text.len().saturating_sub(LINE_ENDING_WINDOW);
    let newline = match text[tail_start..].iter().rposition(|&b| b == b'\n') {
        Some(i) => tail_start + i,
        None => {
            let head_end = text.len().min(LINE_ENDING_WINDOW);
            text[..head_end].iter().position(|&b| b == b'\n')?
        }
    };
    if newline > 0 && text[newline - 1] == b'\r' {
        Some(LineEnding::Crlf)
    } else {
        Some(LineEnding::Lf)
    }
}

/// Builds a git format diff between two versions of a file. File contents are
/// compared by line.
fn unified_diff(filepath: &str, old: &str, new: &str) -> String {
    // Diffs are rendered line by line, so normalize line endings to keep stray
    // carriage returns out of the output.
    let old = old.replace("\r\n", "\n");
    let new = new.replace("\r\n", "\n");
    TextDiff::from_lines(&old, &new)
        .unified_diff()
        .context_radius(CONTEXT_RADIUS)
        .header(&format!("a/{filepath}"), &format!("b/{filepath}"))
        .to_string()
}

fn display_path(cwd: &Path, path: &Path) -> String {
    path.strip_prefix(cwd).unwrap_or(path).display().to_string()
}

/// Renders edit call output
#[derive(Debug)]
pub struct EditRenderItem {
    header: String,
    diff: String,
}

impl DataQuery for EditRenderItem {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        match field {
            "" => Ok(QueryField::Value(json!({
                "type": self.query("/type")?,
                "header": self.query("/header")?,
                "diff": self.query("/diff")?,
            }))),
            "type" => Ok(QueryField::Value(json!("edit"))),
            "header" => Ok(QueryField::Value(self.header.clone().into())),
            "diff" => Ok(QueryField::Value(self.diff.clone().into())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

impl RenderItem for EditRenderItem {
    fn render(
        &self,
        theme: &Theme,
        width: usize,
        _resume_point: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint) {
        let rows = render_edit_diff(theme, width, &self.header, &self.diff);
        (rows, Default::default())
    }
}

fn render_edit_diff(
    theme: &Theme,
    width: usize,
    header: &str,
    diff: &str,
) -> Vec<StyledString> {
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

    // Header
    push_rows(&mut rows, &wrap_line_naive(inner_width, header));

    // Diff
    if !diff.is_empty() {
        let (head, tail) = ellipsize(inner_width, MAX_ROWS, diff);
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
    }

    rows.push(padding);
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::query::QueryError;
    use crate::ui::canvas::render_canvas;
    use crate::ui::render_item::render_error;
    use crate::ui::style::THEME_DARK;

    fn write(dir: &Cwd, name: &str, contents: impl AsRef<[u8]>) -> String {
        let path = dir.path().join(name);
        std::fs::write(&path, contents).expect("write file");
        path.to_string_lossy().into_owned()
    }

    fn read(path: &str) -> String {
        std::fs::read_to_string(path).expect("read file")
    }

    fn call(app: &App, input: &Value) -> anyhow::Result<Value> {
        let tool = EditTool::new(&app.tool_create_args());
        let context = ToolCallContext { session_id: 1 };
        futures::executor::block_on(tool.call(&context, input))
    }

    fn render(tool: &EditTool, width: usize, input: &Value, output: &Value) -> AnyResult<String> {
        let item = tool.render_to_ui(input, output)?;
        let (mut lines, _) = item.render(&THEME_DARK, width, Default::default());
        Ok(render_canvas(&mut lines[..]))
    }

    #[test]
    fn test_edit_tool() {
        let app = App::new().unwrap();
        let dir = Arc::clone(app.cwd());
        let path = write(&dir, "a.txt", "one\ntwo\nthree\n");
        let tool = EditTool::new(&app.tool_create_args());

        assert_eq!(tool.name(), "edit");
        assert!(tool.is_visible());
        assert_eq!(
            tool.input_schema(),
            &json!({
                "type": "object",
                "properties": {
                    "filepath": { "type": "string" },
                    "old_string": { "type": "string", "minLength": 1 },
                    "new_string": { "type": "string" },
                    "replace_all": { "type": "boolean" },
                },
                "required": ["filepath", "old_string", "new_string", "replace_all"],
                "additionalProperties": false,
            })
        );

        // A unique match is replaced and reported with its diff.
        let out = call(&app, &json!({
            "filepath": path,
            "old_string": "two",
            "new_string": "TWO",
            "replace_all": false,
        })).unwrap();
        assert_eq!(out["matches"], json!(1));
        assert!(out["diff"].as_str().unwrap().contains("-two\n+TWO\n"));
        assert_eq!(read(&path), "one\nTWO\nthree\n");

        // Replacements may span lines.
        let out = call(&app, &json!({
            "filepath": path,
            "old_string": "one\nTWO",
            "new_string": "ONE\ntwo",
            "replace_all": false,
        })).unwrap();
        assert_eq!(out["matches"], json!(1));
        assert_eq!(read(&path), "ONE\ntwo\nthree\n");

        // Deleting text is a replacement with an empty string.
        let out = call(&app, &json!({
            "filepath": path,
            "old_string": "three\n",
            "new_string": "",
            "replace_all": false,
        })).unwrap();
        assert_eq!(out["matches"], json!(1));
        assert_eq!(read(&path), "ONE\ntwo\n");

        // replace_all replaces every occurrence.
        let path = write(&dir, "b.txt", "one\ntwo\ntwo\nthree\n");
        let out = call(&app, &json!({
            "filepath": path,
            "old_string": "two",
            "new_string": "TWO",
            "replace_all": true,
        })).unwrap();
        assert_eq!(out["matches"], json!(2));
        assert_eq!(read(&path), "one\nTWO\nTWO\nthree\n");

        // Zero matches is not an error when replacing all.
        let out = call(&app, &json!({
            "filepath": path,
            "old_string": "nope",
            "new_string": "TWO",
            "replace_all": true,
        })).unwrap();
        assert_eq!(out["matches"], json!(0));
        assert_eq!(out["diff"], json!(""));
        assert_eq!(read(&path), "one\nTWO\nTWO\nthree\n");
    }

    #[test]
    fn test_edit_tool_line_endings() {
        let app = App::new().unwrap();
        let dir = Arc::clone(app.cwd());

        // A CRLF file is edited with the LF strings models produce.
        let path = write(&dir, "crlf.txt", "one\r\ntwo\r\nthree\r\n");
        let out = call(&app, &json!({
            "filepath": path,
            "old_string": "two\nthree",
            "new_string": "TWO\nTHREE",
            "replace_all": false,
        })).unwrap();
        assert_eq!(out["matches"], json!(1));
        assert_eq!(out["line_ending"], json!("crlf"));
        // The reported diff uses LF, which renders and reads better.
        let diff = out["diff"].as_str().unwrap();
        assert!(!diff.contains('\r'));
        assert!(diff.contains("-two\n-three\n+TWO\n+THREE\n"));
        // The file keeps its CRLF line endings.
        assert_eq!(read(&path), "one\r\nTWO\r\nTHREE\r\n");

        // CRLF arguments are accepted for a file with LF line endings.
        let path = write(&dir, "lf.txt", "one\ntwo\nthree\n");
        let out = call(&app, &json!({
            "filepath": path,
            "old_string": "two\r\nthree",
            "new_string": "TWO\r\nTHREE",
            "replace_all": false,
        })).unwrap();
        assert_eq!(out["matches"], json!(1));
        assert_eq!(out["line_ending"], json!("lf"));
        assert_eq!(read(&path), "one\nTWO\nTHREE\n");

        // A file with no line endings falls back on the host convention.
        let path = write(&dir, "no_eol.txt", "one");
        let out = call(&app, &json!({
            "filepath": path,
            "old_string": "one",
            "new_string": "ONE\nTWO",
            "replace_all": false,
        })).unwrap();
        assert_eq!(out["matches"], json!(1));
        let expected = if cfg!(windows) { "ONE\r\nTWO" } else { "ONE\nTWO" };
        assert_eq!(read(&path), expected);
    }

    #[test]
    fn test_detect_line_ending() {
        assert_eq!(detect_line_ending(b""), None);
        assert_eq!(detect_line_ending(b"one line"), None);
        assert_eq!(detect_line_ending(b"one\ntwo\n"), Some(LineEnding::Lf));
        assert_eq!(detect_line_ending(b"one\r\ntwo\r\n"), Some(LineEnding::Crlf));

        // The trailing line ending takes precedence over earlier ones.
        assert_eq!(detect_line_ending(b"one\ntwo\r\n"), Some(LineEnding::Crlf));

        // Files without a trailing newline are detected from the head.
        assert_eq!(detect_line_ending(b"one\r\ntwo"), Some(LineEnding::Crlf));

        // Line endings outside of the inspected windows are not detected.
        let x = "x".repeat(LINE_ENDING_WINDOW);
        let text = format!("{x}\r\n{x}");
        assert_eq!(detect_line_ending(text.as_bytes()), None);

    }

    #[test]
    fn test_edit_tool_errors() {
        let app = App::new().unwrap();
        let dir = Arc::clone(app.cwd());
        let path = write(&dir, "a.txt", "one\ntwo\ntwo\nthree\n");

        // Invalid arguments are errors.
        assert_eq!(
            call(&app, &json!({})).unwrap_err().to_string(),
            "arguments don't match schema"
        );
        assert!(call(&app, &json!({
            "filepath": path, "old_string": "two", "new_string": "x", "replace_all": "no",
        })).is_err());
        assert!(call(&app, &json!({
            "filepath": path, "old_string": 1, "new_string": "x", "replace_all": true,
        })).is_err());

        // Unreadable files are errors.
        let missing = dir.path().join("missing.txt").to_string_lossy().into_owned();
        let err = call(&app, &json!({
            "filepath": missing, "old_string": "x", "new_string": "y", "replace_all": true,
        })).unwrap_err();
        assert!(err.to_string().starts_with(&format!("{missing}: ")));

        // Without replace_all, old_string must be unique and present.
        let err = call(&app, &json!({
            "filepath": path, "old_string": "two", "new_string": "TWO", "replace_all": false,
        })).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!(
                "{path}: replacement failed: old string not unique, try again \
                with more context"
            )
        );

        let err = call(&app, &json!({
            "filepath": path, "old_string": "nope", "new_string": "TWO", "replace_all": false,
        })).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("{path}: replacement failed: no matches found, file may have changed")
        );

        // Failed edits leave the file untouched.
        assert_eq!(read(&path), "one\ntwo\ntwo\nthree\n");
    }

    #[test]
    fn test_unified_diff() {
        let old = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
        let new = "one\ntwo\nTHREE\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
        assert_eq!(
            unified_diff("a.txt", old, new),
            [
                "--- a/a.txt",
                "+++ b/a.txt",
                "@@ -1,6 +1,6 @@",
                " one",
                " two",
                "-three",
                "+THREE",
                " four",
                " five",
                " six",
                "",
            ].join("\n")
        );

        // Identical content produces no hunks.
        assert_eq!(unified_diff("a.txt", old, old), "");
    }

    #[test]
    fn test_edit_render_ui() {
        let app = App::new().unwrap();
        let dir = Arc::clone(app.cwd());
        let tool = EditTool::new(&app.tool_create_args());
        let style = Style::new(THEME_DARK.text_base, THEME_DARK.bg_prompt);

        let input = json!({ "filepath": dir.path().join("a.txt").to_string_lossy() });
        let output = json!({ "matches": 1, "diff": "-two\n+TWO\n" });

        // Paths under cwd are rendered relative to it.
        let item = tool.render_to_ui(&input, &output).unwrap();
        assert_eq!(item.query("/type").unwrap(), json!("edit"));
        assert_eq!(item.query("/header").unwrap(), json!("Edited a.txt"));
        assert_eq!(item.query("/diff").unwrap(), json!("-two\n+TWO\n"));
        assert_eq!(
            render(&tool, 14, &input, &output).unwrap(),
            [
                format!("{style}              "),
                format!("{style}  Edited a.t  "),
                format!("{style}  xt          "),
                format!("{style}  -two        "),
                format!("{style}  +TWO        "),
                format!("{style}              "),
            ].join("\n")
        );

        // Paths outside cwd are rendered as-is.
        let input = json!({ "filepath": "/elsewhere/b.txt" });
        let item = tool.render_to_ui(&input, &output).unwrap();
        assert_eq!(item.query("/header").unwrap(), json!("Edited /elsewhere/b.txt"));

        // An empty diff renders only the header row.
        let empty = json!({ "matches": 0, "diff": "" });
        assert_eq!(
            render(&tool, 20, &input, &empty).unwrap(),
            [
                format!("{style}                    "),
                format!("{style}  Edited /elsewher  "),
                format!("{style}  e/b.txt           "),
                format!("{style}                    "),
            ].join("\n")
        );

        // Narrow widths render nothing.
        assert_eq!(render(&tool, 7, &input, &output).unwrap(), "");

        // Errors render as error messages.
        let error_output = json!({ "error": "boom" });
        let item = tool.render_to_ui(&input, &error_output).unwrap();
        assert_eq!(item.query("/type").unwrap(), json!("error"));
        let (mut lines, _) = item.render(&THEME_DARK, 20, Default::default());
        let mut expected = render_error(&THEME_DARK, 20, "boom");
        assert_eq!(render_canvas(&mut lines[..]), render_canvas(&mut expected[..]));

        // Incomplete input or output is rejected.
        assert!(tool.render_to_ui(&json!({}), &output).is_err());
        assert!(tool.render_to_ui(&input, &json!({})).is_err());
    }

    #[test]
    fn test_edit_render_long_diff() {
        let app = App::new().unwrap();
        let tool = EditTool::new(&app.tool_create_args());
        let style = Style::new(THEME_DARK.text_base, THEME_DARK.bg_prompt);
        let ellipsis_style = Style::new(THEME_DARK.text_subtle, THEME_DARK.bg_prompt);

        let input = json!({ "filepath": "a.txt" });
        let output = json!({ "matches": 1, "diff": format!("{}\n", "x".repeat(200)) });
        let line = || format!("{style}  xxxxxxxxxx  ");
        assert_eq!(
            render(&tool, 14, &input, &output).unwrap(),
            [
                format!("{style}              "),
                format!("{style}  Edited a.t  "),
                format!("{style}  xt          "),
                line(), line(), line(), line(), line(),
                format!("{ellipsis_style}  ...         "),
                line(), line(), line(), line(), line(),
                format!("{style}              "),
            ].join("\n")
        );
    }

    #[test]
    fn test_edit_render_interface() {
        let app = App::new().unwrap();
        let dir = Arc::clone(app.cwd());
        let tool = EditTool::new(&app.tool_create_args());
        let input = json!({ "filepath": dir.path().join("a.txt").to_string_lossy() });

        let output = json!({ "matches": 3, "line_ending": "crlf", "diff": "-two\n+TWO\n" });
        let result = tool.render_to_interface(&input, &output).unwrap();
        assert_eq!(
            result.content,
            vec![ToolOutputContent::Text {
                text: Cow::Owned("detected line ending 'crlf'\nmatches replaced: 3".into())
            }]
        );

        // Zero matches is reported to the model.
        let result = tool
            .render_to_interface(&input, &json!({ "matches": 0, "line_ending": "lf", "diff": "" }))
            .unwrap();
        assert_eq!(
            result.content,
            vec![ToolOutputContent::Text {
                text: Cow::Owned("detected line ending 'lf'\nmatches replaced: 0".into())
            }]
        );

        // Errors render as a single text item.
        let result = tool
            .render_to_interface(&input, &json!({ "error": "boom" }))
            .unwrap();
        assert_eq!(
            result.content,
            vec![ToolOutputContent::Text { text: Cow::Owned("error: boom".into()) }]
        );

        assert!(tool.render_to_interface(&input, &json!({})).is_err());
    }

    #[test]
    fn test_query() {
        let app = App::new().unwrap();
        let tool = EditTool::new(&app.tool_create_args());
        assert_eq!(tool.query("/").unwrap(), json!({ "name": "edit", "is_visible": true }));
        assert_eq!(tool.query("/name").unwrap(), json!("edit"));
        assert_eq!(tool.query("/is_visible").unwrap(), json!(true));
        assert!(matches!(tool.query("/missing"), Err(QueryError::InvalidField(_))));

        let item = EditRenderItem {
            header: "Edited a.txt".into(),
            diff: "-two\n+TWO\n".into(),
        };
        assert_eq!(
            item.query("/").unwrap(),
            json!({
                "type": "edit",
                "header": "Edited a.txt",
                "diff": "-two\n+TWO\n",
            })
        );
        assert_eq!(item.query("/type").unwrap(), json!("edit"));
        assert_eq!(item.query("/header").unwrap(), json!("Edited a.txt"));
        assert_eq!(item.query("/diff").unwrap(), json!("-two\n+TWO\n"));
        assert!(matches!(item.query("/missing"), Err(QueryError::InvalidField(_))));
    }
}
