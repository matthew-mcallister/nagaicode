use fnv::FnvHashMap;
use serde_json::json;

use crate::error::AnyResult;
use crate::query::{DataQuery, QueryError, QueryField};
use crate::session::{Item, ItemType};
use crate::session::ToolCallArgs;
use crate::ui::markdown::{MarkdownResult, ResumePoint};
use crate::ui::style::{Style, Theme};
use crate::ui::styled_string::StyledString;
use crate::ui::text::{SPACES, wrap_line, wrap_line_naive};
use crate::ui::tool_render_item::ToolRenderer;

/// Dynamic renderable content.
///
/// The DataQuery impl for each RenderItem should expose a field "type" which
/// identifies the RenderItem implementation being used, in addition to its
/// inner fields.
pub trait RenderItem: std::fmt::Debug + DataQuery {
    fn render(
        &self,
        theme: &Theme,
        width: usize,
        resume: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint);
}

// Transitional type, to be replaced entirely by RenderItem
#[derive(Debug)]
pub enum HistoryItemContent {
    Help(String),
    Error(String),
    User(String),
    Thought(String),
    Response(String),
    CommandPrompt(String),
    CommandOutput(String),
    Dynamic(Box<dyn RenderItem>),
}

impl DataQuery for HistoryItemContent {
    fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
        let (ty, content) = match self {
            Self::Help(content) => ("help", content),
            Self::Error(content) => ("error", content),
            Self::User(content) => ("user", content),
            Self::Thought(content) => ("thought", content),
            Self::Response(content) => ("response", content),
            Self::CommandPrompt(content) => ("command_prompt", content),
            Self::CommandOutput(content) => ("command_output", content),
            Self::Dynamic(render_item) => return render_item.query_field(field),
        };
        match field {
            "" => Ok(QueryField::Value(json!({
                "type": ty,
                "value": content,
            }))),
            "type" => Ok(QueryField::Value(json!(ty))),
            "value" => Ok(QueryField::Value(content.clone().into())),
            _ => Err(QueryError::InvalidField(field.to_string())),
        }
    }
}

pub fn get_item_content(
    tools: &ToolRenderer,
    tool_calls: &FnvHashMap<String, ToolCallArgs>,
    item: &Item,
) -> AnyResult<Option<HistoryItemContent>> {
    Ok(Some(match item.ty()? {
        ItemType::UserText => {
            let Some(text) = item.text.clone() else { return Ok(None) };
            HistoryItemContent::User(text)
        }
        ItemType::ResponseText => {
            let Some(text) = item.text.clone() else { return Ok(None) };
            HistoryItemContent::Response(text)
        }
        ItemType::Reasoning => {
            let Some(text) = item.text.clone()
                .or_else(|| item.summary.clone())
                else { return Ok(None) };
            HistoryItemContent::Thought(text)
        }
        ItemType::ToolCall => return Ok(None),
        ItemType::ToolOutput => {
            let call_id = item.upstream_call_id.as_ref()
                .ok_or_else(|| anyhow::anyhow!("item {} missing call id", item.id))?;
            let args = tool_calls.get(call_id)
                .ok_or_else(|| anyhow::anyhow!("missing args for item {}", item.id))?;
            let Some(output) = item.tool_output()? else { return Ok(None) };
            let render_item = tools.build_render_item(
                &args.name,
                &args.args,
                &output
            )?;
            HistoryItemContent::Dynamic(render_item)
        },
    }))
}

pub fn render_help(
    theme: &Theme,
    width: usize,
    content: &str,
) -> Vec<StyledString> {
    content.lines().flat_map(|line| {
        wrap_line(width - 4, line)
            .into_iter()
            .map(|row| {
                let style = Style::new(theme.text_subtle, theme.bg_base);
                let mut s = StyledString::new(style, width + 4);
                s.push("▐ ", 2);
                s.set_text(theme.text_quote);
                s.push(&row.to_padded_string(width - 4), width - 4);
                s.push("  ", 2);
                s
            })
        })
        .collect()
}

pub fn render_error(
    theme: &Theme,
    width: usize,
    content: &str,
) -> Vec<StyledString> {
    content.lines().flat_map(|line| {
        wrap_line(width - 4, line)
            .into_iter()
            .map(|row| {
                let style = Style::new(theme.text_error, theme.bg_base);
                let mut s = StyledString::new(style, width + 4);
                s.push("▐ ", 2);
                s.set_text(theme.text_subtle);
                s.push(&row.to_padded_string(width - 4), width - 4);
                s.push("  ", 2);
                s
            })
        })
        .collect()
}

pub fn render_prompt(
    theme: &Theme,
    width: usize,
    content: &str,
) -> Vec<StyledString> {
    if content.is_empty() {
        return Vec::new();
    }
    let style = Style::new(theme.text_base, theme.bg_prompt);

    let make_padding = || {
        let mut s = StyledString::new(style, width + 4);
        s.push(&SPACES[..width], width);
        s
    };

    let mut rows = vec![make_padding()];
    rows.extend(content.lines().flat_map(|line| {
        wrap_line(width - 4, line).into_iter().map(|row| {
            let mut s = StyledString::new(style, width + 4);
            s.push("  ", 2);
            s.push(&row.to_padded_string(width - 4), width - 4);
            s.push("  ", 2);
            s
        })
    }));
    rows.push(make_padding());
    rows
}

pub fn render_markdown(
    theme: &Theme,
    width: usize,
    content: &str,
    resume: ResumePoint,
) -> MarkdownResult {
    let content = &content[resume.offset..];
    let mut result = crate::ui::markdown::render_markdown(theme, width - 4, content);
    for row in result.rows.iter_mut() {
        let mut padded = StyledString::new(theme.base_style(), width + 4);
        padded.push("  ", 2);
        padded.push_styled(row);
        padded.pad_to_width(width);
        *row = padded;
    }
    result.resume_point = resume.add(result.resume_point);
    result
}

pub fn render_thought(
    theme: &Theme,
    width: usize,
    content: &str,
    resume: ResumePoint,
) -> MarkdownResult {
    let content = &content[resume.offset..];
    let bar_style = Style::new(theme.text_thought, theme.bg_base);
    let mut result = crate::ui::markdown::render_markdown(theme, width - 4, content);
    for row in result.rows.iter_mut() {
        let mut padded = StyledString::new(bar_style, width + 4);
        padded.push("▐ ", 2);
        padded.push_styled(row);
        padded.pad_to_width(width);
        *row = padded;
    }
    result.resume_point = resume.add(result.resume_point);
    result
}

pub fn render_command_prompt(
    theme: &Theme,
    width: usize,
    content: &str,
) -> Vec<StyledString> {
    let style = Style::new(theme.text_base, theme.bg_base);
    content
        .lines()
        .flat_map(|line| {
            wrap_line_naive(width - 4, line).into_iter().map(|row| {
                let mut s = StyledString::new(style, width + 4);
                s.push("  ", 2);
                s.push(&row.to_padded_string(width - 4), width - 4);
                s.push("  ", 2);
                s
            })
        })
        .collect()
}

pub fn render_command_output(theme: &Theme, width: usize, content: &str) -> Vec<StyledString> {
    let style = Style::new(theme.text_subtle, theme.bg_base);
    content
        .lines()
        .flat_map(|line| {
            wrap_line_naive(width - 4, line).into_iter().map(|row| {
                let mut s = StyledString::new(style, width + 4);
                s.push("  ", 2);
                s.push(&row.to_padded_string(width - 4), width - 4);
                s.push("  ", 2);
                s
            })
        })
        .collect()
}

/// Performs a full or partial rerender. Newly rendered rows are returned, as
/// well as a resume point for future rerenders.
pub fn render(
    theme: &Theme,
    width: usize,
    content: &HistoryItemContent,
    resume: ResumePoint,
) -> (Vec<StyledString>, ResumePoint) {
    match content {
        HistoryItemContent::Help(content) => (render_help(theme, width, content), Default::default()),
        HistoryItemContent::Error(content) => (render_error(theme, width, content), Default::default()),
        HistoryItemContent::User(content) => (render_prompt(theme, width, content), Default::default()),
        HistoryItemContent::CommandPrompt(content) => (render_command_prompt(theme, width, content), Default::default()),
        HistoryItemContent::CommandOutput(content) => (render_command_output(theme, width, content), Default::default()),
        HistoryItemContent::Response(content) => {
            let result = render_markdown(theme, width, content, resume);
            (result.rows, result.resume_point)
        }
        HistoryItemContent::Thought(content) => {
            let result = render_thought(theme, width, content, resume);
            (result.rows, result.resume_point)
        }
        HistoryItemContent::Dynamic(render_item) => {
            let (rows, resume) = render_item.render(theme, width, resume);
            (rows, resume)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::canvas::render_canvas;
    use crate::ui::style::testing::SetItalic;
    use crate::ui::style::{Style, THEME_DARK};

    #[test]
    fn test_render_help() {
        let theme = &THEME_DARK;
        let help_style = Style::new(theme.text_subtle, theme.bg_base);
        let italic = SetItalic;

        fn render(content: &str, width: usize) -> String {
            let mut lines = super::render_help(&THEME_DARK, width, content);
            render_canvas(&mut lines[..])
        }

        assert_eq!(render("hello", 14), format!("{help_style}▐ {italic}hello       "));
        assert_eq!(render("foo\nbar", 12), format!("{help_style}▐ {italic}foo       \n{help_style}▐ {italic}bar       "));
        assert_eq!(render("hello world", 12), format!("{help_style}▐ {italic}hello     \n{help_style}▐ {italic}world     "));
        assert_eq!(render("", 8), "");
    }

    #[test]
    fn test_render_error() {
        use crate::ui::style::UpdateStyle;

        let theme = &THEME_DARK;
        let error_style = Style::new(theme.text_error, theme.bg_base);
        let subtle_style = Style::new(theme.text_subtle, theme.bg_base);
        let transition = UpdateStyle(error_style, subtle_style);

        fn render(content: &str, width: usize) -> String {
            let mut lines = super::render_error(&THEME_DARK, width, content);
            render_canvas(&mut lines[..])
        }

        assert_eq!(render("hello", 12), format!("{error_style}▐ {transition}hello     "));
        assert_eq!(render("foo\nbar", 12), format!("{error_style}▐ {transition}foo       \n{error_style}▐ {transition}bar       "));
        assert_eq!(render("hello world", 12), format!("{error_style}▐ {transition}hello     \n{error_style}▐ {transition}world     "));
        assert_eq!(render("", 8), "");
    }

    #[test]
    fn test_render_thought() {
        use crate::ui::style::UpdateStyle;

        let theme = &THEME_DARK;
        let base_style = theme.base_style();
        let thought_style = Style::new(theme.text_thought, theme.bg_base);
        let transition = UpdateStyle(thought_style, base_style);

        fn render(content: &str, width: usize) -> String {
            let mut lines = super::render_thought(&THEME_DARK, width, content, Default::default()).rows;
            render_canvas(&mut lines[..])
        }

        assert_eq!(render("hello", 14), format!("{thought_style}▐ {transition}hello       "));
        assert_eq!(render("foo\nbar", 12), format!("{thought_style}▐ {transition}foo bar   "));
        assert_eq!(render("hello world", 12), format!("{thought_style}▐ {transition}hello     \n{thought_style}▐ {transition}world     "));
        assert_eq!(render("", 8), "");
    }

    #[test]
    fn test_render_prompt() {
        let theme = &THEME_DARK;
        let prompt_style = Style::new(theme.text_base, theme.bg_prompt);

        fn render(content: &str, width: usize) -> String {
            let mut lines = super::render_prompt(&THEME_DARK, width, content);
            render_canvas(&mut lines[..])
        }

        assert_eq!(
            render("hello", 14),
            format!(
                "{prompt_style}              \n{prompt_style}  hello       \n{prompt_style}              "
            )
        );
        assert_eq!(
            render("foo\nbar", 12),
            format!(
                "{prompt_style}            \n{prompt_style}  foo       \n{prompt_style}  bar       \n{prompt_style}            "
            )
        );
        assert_eq!(
            render("hello world", 12),
            format!(
                "{prompt_style}            \n{prompt_style}  hello     \n{prompt_style}  world     \n{prompt_style}            "
            )
        );
        assert_eq!(render("", 8), "");
    }

    #[test]
    fn test_render_command() {
        let theme = &THEME_DARK;
        let prompt_style = Style::new(theme.text_base, theme.bg_base);
        let output_style = Style::new(theme.text_subtle, theme.bg_base);

        fn render_prompt(content: &str, width: usize) -> String {
            let mut lines = super::render_command_prompt(&THEME_DARK, width, content);
            render_canvas(&mut lines[..])
        }
        fn render_output(content: &str, width: usize) -> String {
            let mut lines = super::render_command_output(&THEME_DARK, width, content);
            render_canvas(&mut lines[..])
        }

        assert_eq!(
            render_prompt("!foo", 14),
            format!("{prompt_style}  !foo        ")
        );
        assert_eq!(
            render_prompt("!foo\n!bar", 14),
            format!("{prompt_style}  !foo        \n{prompt_style}  !bar        ")
        );
        assert_eq!(
            render_prompt("!hello world foo", 14),
            format!("{prompt_style}  !hello wor  \n{prompt_style}  ld foo      ")
        );
        assert_eq!(
            render_output("ok", 14),
            format!("{output_style}  ok          ")
        );
        assert_eq!(
            render_output("line1\nline2", 14),
            format!("{output_style}  line1       \n{output_style}  line2       ")
        );
        assert_eq!(render_prompt("", 8), "");
        assert_eq!(render_output("", 8), "");
    }

    #[test]
    fn test_history_item_content_query() {
        use super::*;
        use crate::query::QueryError;
        use crate::tools::ToolResult;
        use crate::ui::tool_render_item::load_tool_renderers;

        let user = HistoryItemContent::User("hello".into());
        assert_eq!(user.query("/").unwrap(), json!({"type": "user", "value": "hello"}));
        assert_eq!(user.query("/type").unwrap(), json!("user"));
        assert_eq!(user.query("/value").unwrap(), json!("hello"));
        assert!(matches!(user.query("/missing"), Err(QueryError::InvalidField(_))));

        let command = HistoryItemContent::CommandPrompt("!foo".into());
        assert_eq!(command.query("/").unwrap(), json!({"type": "command_prompt", "value": "!foo"}));

        let tools = load_tool_renderers();
        let item = tools.build_render_item(
            "sh",
            &json!({"command": "echo hi"}),
            &ToolResult::Json(json!({"stdout": "hi\n", "stderr": "", "return_code": 0})),
        ).unwrap();
        let dynamic = HistoryItemContent::Dynamic(item);
        assert_eq!(
            dynamic.query("/").unwrap(),
            json!({"type": "sh", "cmd_line": "echo hi", "stdout": "hi\n"})
        );
        assert_eq!(dynamic.query("/type").unwrap(), json!("sh"));
        assert_eq!(dynamic.query("/stdout").unwrap(), json!("hi\n"));
    }
}
