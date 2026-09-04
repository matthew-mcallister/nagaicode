use serde_json::json;

use crate::query::{DataQuery, QueryError, QueryField};
use crate::item::{Item, ItemContent, ReasoningContent};
use crate::ui::markdown::{MarkdownResult, ResumePoint};
use crate::ui::style::{Style, Theme};
use crate::ui::styled_string::StyledString;
use crate::ui::text::{SPACES, wrap_line, wrap_line_naive};
use crate::tools::ToolRegistry;

/// Dynamic renderable content.
///
/// The DataQuery impl for each RenderItem should expose a field "type" which
/// identifies the RenderItem implementation being used, in addition to its
/// inner fields.
pub trait RenderItem: std::fmt::Debug + DataQuery {
    /// Whether a vertical padding row is rendered after the item's rows.
    fn trailing_padding(&self) -> bool {
        true
    }

    /// Renders (or partially renders) the item, returning rows and a resume
    /// point for future incremental renders. Not all implementations must
    /// support incremental renders, but those that do must not be updated
    /// except for appends.
    fn render(
        &self,
        theme: &Theme,
        width: usize,
        resume: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint);
}

macro_rules! string_render_item {
    ($name:ident, $ty:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug)]
        pub struct $name(String);

        impl $name {
            /// Creates a new item.
            pub fn new(content: impl Into<String>) -> Self {
                Self(content.into())
            }
        }

        impl DataQuery for $name {
            fn query_field<'a>(&'a self, field: &str) -> Result<QueryField<'a>, QueryError> {
                match field {
                    "" => Ok(QueryField::Value(json!({
                        "type": $ty,
                        "value": self.0,
                    }))),
                    "type" => Ok(QueryField::Value(json!($ty))),
                    "value" => Ok(QueryField::Value(self.0.clone().into())),
                    _ => Err(QueryError::InvalidField(field.to_string())),
                }
            }
        }
    };
}

string_render_item!(HelpRenderItem, "help", "Renders help text.");
string_render_item!(ErrorRenderItem, "error", "Renders an error message.");
string_render_item!(UserRenderItem, "user", "Renders user input.");
string_render_item!(ThoughtRenderItem, "thought", "Renders model reasoning.");
string_render_item!(ResponseRenderItem, "response", "Renders model response text.");
string_render_item!(CommandPromptRenderItem, "command_prompt", "Renders a command prompt.");
string_render_item!(CommandOutputRenderItem, "command_output", "Renders command output.");

impl RenderItem for HelpRenderItem {
    fn render(
        &self,
        theme: &Theme,
        width: usize,
        _resume: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint) {
        (render_help(theme, width, &self.0), Default::default())
    }
}

impl RenderItem for ErrorRenderItem {
    fn render(
        &self,
        theme: &Theme,
        width: usize,
        _resume: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint) {
        (render_error(theme, width, &self.0), Default::default())
    }
}

impl RenderItem for UserRenderItem {
    fn render(
        &self,
        theme: &Theme,
        width: usize,
        _resume: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint) {
        (render_prompt(theme, width, &self.0), Default::default())
    }
}

impl RenderItem for ThoughtRenderItem {
    fn render(
        &self,
        theme: &Theme,
        width: usize,
        resume: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint) {
        let result = render_thought(theme, width, &self.0, resume);
        (result.rows, result.resume_point)
    }
}

impl RenderItem for ResponseRenderItem {
    fn render(
        &self,
        theme: &Theme,
        width: usize,
        resume: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint) {
        let result = render_markdown(theme, width, &self.0, resume);
        (result.rows, result.resume_point)
    }
}

impl RenderItem for CommandPromptRenderItem {
    fn trailing_padding(&self) -> bool {
        false
    }

    fn render(
        &self,
        theme: &Theme,
        width: usize,
        _resume: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint) {
        (render_command_prompt(theme, width, &self.0), Default::default())
    }
}

impl RenderItem for CommandOutputRenderItem {
    fn render(
        &self,
        theme: &Theme,
        width: usize,
        _resume: ResumePoint,
    ) -> (Vec<StyledString>, ResumePoint) {
        (render_command_output(theme, width, &self.0), Default::default())
    }
}

pub fn get_item_content(
    tools: &ToolRegistry,
    item: &Item,
) -> Option<Box<dyn RenderItem>> {
    Some(match &item.content {
        ItemContent::UserText(text) => Box::new(UserRenderItem::new(text.clone())),
        ItemContent::ResponseText(text) => {
            Box::new(ResponseRenderItem::new(text.clone()))
        }
        ItemContent::Reasoning(ReasoningContent { text, summary, .. }) => {
            let text = text.clone().or_else(|| summary.clone())?;
            Box::new(ThoughtRenderItem::new(text))
        }
        ItemContent::ToolCall(content) => tools.render_to_ui(content)?,
    })
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
    fn test_render_item_query() {
        use super::*;
        use std::sync::Arc;

        use crate::query::QueryError;
        use crate::tools::Tool;
        use crate::tools::sh::ShTool;

        let user = UserRenderItem::new("hello");
        assert_eq!(user.query("/").unwrap(), json!({"type": "user", "value": "hello"}));
        assert_eq!(user.query("/type").unwrap(), json!("user"));
        assert_eq!(user.query("/value").unwrap(), json!("hello"));
        assert!(matches!(user.query("/missing"), Err(QueryError::InvalidField(_))));
        assert!(user.trailing_padding());

        let command = CommandPromptRenderItem::new("!foo");
        assert_eq!(command.query("/").unwrap(), json!({"type": "command_prompt", "value": "!foo"}));
        assert!(!command.trailing_padding());

        let tool = ShTool::new(Arc::new(crate::cwd::cwd()));
        let item = tool.render_to_ui(
            &json!({"command": "echo hi"}),
            &json!({"stdout": "hi\n", "stderr": "", "return_code": 0}),
        ).unwrap();
        assert_eq!(
            item.query("/").unwrap(),
            json!({"type": "sh", "cmd_line": "echo hi", "stdout": "hi\n"})
        );
        assert_eq!(item.query("/type").unwrap(), json!("sh"));
        assert_eq!(item.query("/stdout").unwrap(), json!("hi\n"));
    }
}
