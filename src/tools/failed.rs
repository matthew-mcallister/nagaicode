use std::borrow::Cow;

use crate::interface::ToolOutputContent;
use crate::tools::InterfaceToolOutput;
use crate::ui::render_item::{ErrorRenderItem, RenderItem};

pub fn render_failure_to_ui(tool_name: &str, error: &str) -> Box<dyn RenderItem> {
    Box::new(ErrorRenderItem::new(format!("Called '{tool_name}': {error}")))
}

pub fn render_failure_to_interface(tool_name: &str, error: &str) -> InterfaceToolOutput {
    InterfaceToolOutput {
        name: tool_name.to_owned(),
        content: vec![ToolOutputContent::Text {
            text: Cow::Owned(format!("error: {error}")),
        }],
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::ui::canvas::render_canvas;
    use crate::ui::render_item::render_error;
    use crate::ui::style::THEME_DARK;

    #[test]
    fn test_failure_interface() {
        let result = render_failure_to_interface("sh", "invalid arguments");
        assert_eq!(
            result.content,
            vec![
                ToolOutputContent::Text { text: Cow::Owned("error: invalid arguments".into()) }
            ]
        );
    }

    #[test]
    fn test_failure_ui() {
        let failed = render_failure_to_ui("sh", "invalid arguments");
        assert_eq!(failed.query("/type").unwrap(), json!("error"));
        let (mut lines, _) = failed.render(&THEME_DARK, 20, Default::default());
        let mut expected = render_error(&THEME_DARK, 20, "Called 'sh': invalid arguments");
        assert_eq!(render_canvas(&mut lines[..]), render_canvas(&mut expected[..]));
    }
}