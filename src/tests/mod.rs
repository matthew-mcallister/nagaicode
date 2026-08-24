use crossterm::event::{Event, KeyCode, KeyEvent};
use serde_json::json;

use crate::app::{App, AppEvent};
use crate::interface::InterfaceId;
use crate::model::Model;
use crate::provider::Provider;
use crate::tools::ToolResult;
use crate::tools::mock::ToolCall;
use crate::ui::canvas::render_canvas;

const EXPECTED_INITIAL_FRAME: &str = concat!(
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[38;2;168;162;158m  ▐ \x1b[3mWelcome to NagaiCode!                                                  \x1b[38;2;87;83;78m\x1b[23m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[38;2;168;162;158m  ▐ \x1b[3m                                                                       \x1b[38;2;87;83;78m\x1b[23m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[38;2;168;162;158m  ▐ \x1b[3mType /help for a list of commands.                                     \x1b[38;2;87;83;78m\x1b[23m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                             \x1b[38;2;87;83;78m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[48;2;41;37;36m                                                                           \x1b[38;2;56;189;248m\x1b[48;2;12;10;9m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[48;2;41;37;36m                                                                           \x1b[38;2;56;189;248m\x1b[48;2;12;10;9m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[48;2;41;37;36m                                                                           \x1b[38;2;56;189;248m\x1b[48;2;12;10;9m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                ",
);

#[test]
fn test_app_e2e() {
    let mut app = App::new().expect("failed to create app");

    let mut canvas = app.make_canvas();
    app.draw(&mut canvas);
    let frame = render_canvas(&mut canvas);

    assert_eq!(frame, EXPECTED_INITIAL_FRAME);
    assert!(!app.quit());

    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char('/'))));
    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char('q'))));
    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Enter)));

    assert!(app.quit());
}

#[tokio::test]
async fn test_app_interrupt() {
    let mut app = App::new().unwrap();

    let provider = Provider::create(app.conn(), "test", InterfaceId::Openai, "key", None)
        .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");
    app.switch_model(model);

    app.process_event(AppEvent::Interrupt);
    assert!(app.task_canceled());

    app.process_command("hello").unwrap();
    assert!(!app.task_canceled());

    app.process_event(AppEvent::Interrupt);
    assert!(app.task_canceled());
}

#[test]
fn test_app_process_command() {
    let mut app = App::new().unwrap();

    app.tools_mut().add_result(
        "sh",
        Ok(ToolResult::success(json!({
            "stdout": "output line\n",
            "stderr": "",
            "return_code": 0,
        }))),
    );

    app.process_command("!echo test").unwrap();

    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        ToolCall {
            name: "sh".to_owned(),
            args: json!("echo test"),
        }
    );

    app.tools_mut().add_result(
        "sh",
        Ok(ToolResult::success(json!("string output"))),
    );
    app.process_command("!pwd").unwrap();
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].args, json!("pwd"));

    app.tools_mut().add_result(
        "sh",
        Ok(ToolResult::error(json!({
            "stdout": "",
            "stderr": "error message\n",
            "return_code": 1,
        }))),
    );
    app.process_command("!false").unwrap();
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 3);

    app.tools_mut().add_result(
        "sh",
        Ok(ToolResult::success(json!({
            "stdout": "",
            "stderr": "",
            "return_code": 0,
        }))),
    );
    app.process_command("!true").unwrap();
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 4);

    app.tools_mut().add_result(
        "sh",
        Err("tool error".into()),
    );
    app.process_event(AppEvent::Command("!failing_tool".to_string()));
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 5);

    assert!(app.process_command("").is_ok());
    assert!(app.process_command("   ").is_ok());
}

#[cfg(test)]
mod tests {
    use crate::interface::InterfaceId;
    use crate::model::Model;
    use crate::provider::Provider;
    use crate::query::DataQuery;
    use crate::ui::chat::Chat;
    use crate::ui::style::THEME_DARK;
    use super::*;

    #[tokio::test]
    async fn test_app_query() {
        let mut app = App::new().unwrap();

        // Primitive and unset fields.
        let db_url = app.query("/db_url").unwrap();
        assert!(db_url.as_str().unwrap().starts_with("file:"));
        assert!(db_url.as_str().unwrap().contains("mode=memory"));
        assert_eq!(app.query("/selected_model").unwrap(), json!(null));
        assert_eq!(app.query("/session").unwrap(), json!(null));

        // Nested query into chat.
        let expected_chat = Chat::new(80, 24, &THEME_DARK).query("/").unwrap();
        assert_eq!(app.query("/chat").unwrap(), expected_chat);
        assert_eq!(app.query("/chat/stacked/h_padding").unwrap(), json!(2));
        assert_eq!(app.query("/chat/stacked/v_padding").unwrap(), json!(1));
        assert_eq!(app.query("/chat/stacked/inner/focus_state").unwrap(), json!("command_editor"));

        assert_eq!(app.query("/").unwrap(), json!({
            "chat": expected_chat,
            "selected_model": null,
            "db_url": db_url,
            "session": null,
        }));

        // Selecting a model exposes it as a nested query.
        let provider = Provider::create(app.conn(), "test", InterfaceId::Openai, "key123", None).unwrap();
        let model = Model::create(app.conn(), provider.id, "gpt-4").unwrap();
        app.switch_model(model.clone());
        assert_eq!(app.query("/selected_model/id").unwrap(), json!("gpt-4"));
        assert_eq!(app.query("/selected_model").unwrap(), model.query("/").unwrap());
    }
}
