use std::borrow::Cow;

use crate::interface::ToolOutputContent;
use crate::tools::InterfaceToolOutput;
use crate::ui::render_item::{HelpRenderItem, RenderItem};

pub fn render_unknown_to_ui(tool_name: &str) -> Box<dyn RenderItem> {
    Box::new(HelpRenderItem::new(format!("Called '{}'", tool_name)))
}

pub fn render_unknown_to_interface() -> InterfaceToolOutput {
    InterfaceToolOutput {
        content: vec![ToolOutputContent::Text {
            text: Cow::Owned("error: could not parse output".into()),
        }],
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::ui::canvas::render_canvas;
    use crate::ui::render_item::render_help;
    use crate::ui::style::THEME_DARK;

    #[test]
    fn test_unknown_ui() {
        let result = render_unknown_to_interface();
        assert_eq!(
            result.content,
            vec![ToolOutputContent::Text { text: Cow::Owned("error: could not parse output".into()) }],
        );
    }

    #[test]
    fn test_unknown_interface() {
        let item = render_unknown_to_ui("sh");
        assert_eq!(item.query("/type").unwrap(), json!("help"));
        let (mut lines, _) = item.render(&THEME_DARK, 20, Default::default());
        let mut expected = render_help(&THEME_DARK, 20, "Called 'sh'");
        assert_eq!(render_canvas(&mut lines[..]), render_canvas(&mut expected[..]));
    }
}