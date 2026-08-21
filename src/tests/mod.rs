use crossterm::event::{Event, KeyCode, KeyEvent};
use serde_json::json;

use crate::app::{App, AppEvent};
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
