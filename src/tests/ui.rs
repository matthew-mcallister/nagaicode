use crossterm::event::{Event, KeyCode, KeyEvent};
use serde_json::json;

use crate::app::{App, AppEvent};
use crate::interface::InterfaceId;
use crate::model::Model;
use crate::provider::Provider;
use crate::query::DataQuery;
use crate::tool::mock::ToolCall;
use crate::ui::canvas::render_canvas;
use crate::ui::chat::Chat;
use crate::ui::style::THEME_DARK;

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
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[38;2;168;162;158m▐ \x1b[3mWelcome to NagaiCode!                                                    \x1b[38;2;87;83;78m\x1b[23m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[38;2;168;162;158m▐ \x1b[3m                                                                         \x1b[38;2;87;83;78m\x1b[23m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[38;2;168;162;158m▐ \x1b[3mType /help for a list of commands.                                       \x1b[38;2;87;83;78m\x1b[23m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                             \x1b[38;2;87;83;78m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[48;2;41;37;36m                                                                           \x1b[38;2;56;189;248m\x1b[48;2;12;10;9m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[48;2;41;37;36m                                                                           \x1b[38;2;56;189;248m\x1b[48;2;12;10;9m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m  \x1b[48;2;41;37;36m                                                                           \x1b[38;2;56;189;248m\x1b[48;2;12;10;9m▐  \n",
    "\x1b[48;2;12;10;9m\x1b[38;5;15m                                                                                ",
);

#[tokio::test]
async fn test_app_e2e() {
    let mut app = App::new().expect("failed to create app");

    let mut canvas = app.make_canvas();
    app.draw(&mut canvas);
    let frame = render_canvas(&mut canvas);

    assert_eq!(frame, EXPECTED_INITIAL_FRAME);
    assert!(!app.quit());

    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char('/'))))
        .await;
    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char('q'))))
        .await;
    app.handle_input(Event::Key(KeyEvent::from(KeyCode::Enter)))
        .await;

    assert!(app.quit());
}

/// Counts "Interrupted." help messages in the chat history.
fn interrupted_count(app: &App) -> usize {
    app.query("/chat/stacked/inner/history/history/items")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["content"]["value"].as_str() == Some("Interrupted."))
        .count()
}

#[tokio::test]
async fn test_app_interrupt() {
    let mut app = App::new().unwrap();

    let provider = Provider::create(app.conn(), "test", InterfaceId::Openai, "key", None)
        .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");
    app.switch_model(provider, model).unwrap();

    app.process_event(AppEvent::Interrupt).await;
    assert_eq!(app.query("/current_task").unwrap(), json!(null));

    app.process_command("hello").await.unwrap();
    assert_eq!(app.query("/current_task").unwrap(), json!(0));

    app.process_event(AppEvent::Interrupt).await;
    assert_eq!(app.query("/current_task").unwrap(), json!(null));
    app.process_pending_events().await;
    assert_eq!(app.query("/task_count").unwrap(), json!(0));

    // Interrupting an active task emits a single help message.
    assert_eq!(interrupted_count(&app), 1);

    // Interrupting with no active task shows nothing.
    app.process_event(AppEvent::Interrupt).await;
    app.process_pending_events().await;
    assert_eq!(interrupted_count(&app), 1);
}

#[tokio::test]
async fn test_app_task_complete() {
    let mut app = App::new().unwrap();

    // Completing a task clears it once completion events are processed.
    let dummy = app.spawn_dummy_task().await;
    tokio::task::yield_now().await;
    app.process_pending_events().await;
    assert_eq!(app.query("current_task").unwrap(), json!(0));
    assert_eq!(app.query("task_count").unwrap(), json!(1));
    dummy.complete();
    app.await_task().await.expect("await dummy task");
    app.process_pending_events().await;
    assert_eq!(app.query("current_task").unwrap(), json!(null));
    assert_eq!(app.query("task_count").unwrap(), json!(0));

    // Canceling a task ends it and reports the cancelation once pending
    // events are processed.
    let _dummy = app.spawn_dummy_task().await;
    app.process_event(AppEvent::Interrupt).await;
    assert_eq!(app.query("current_task").unwrap(), json!(null));
    app.process_pending_events().await;
    assert_eq!(app.query("task_count").unwrap(), json!(0));
    assert_eq!(interrupted_count(&app), 1);
}

#[test]
fn test_app_persists_selected_model() {
    use crate::settings::{ModelRef, Settings};

    let mut app = App::new().unwrap();

    let provider = Provider::create(app.conn(), "test", InterfaceId::Openai, "key", None)
        .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");

    let reloaded = Settings::open(app.db_url()).expect("open settings");
    assert_eq!(reloaded.current_model(), None);

    app.switch_model(provider, model.clone()).unwrap();
    let expected = ModelRef {
        provider: "test".to_string(),
        model: "gpt-4".to_string(),
    };
    assert_eq!(
        app.selected_model().map(|(_, m)| m.id.as_str()),
        Some("gpt-4")
    );
    assert_eq!(app.selected_model().map(|(p, _)| p.name.as_str()), Some("test"));
    let reloaded = Settings::open(app.db_url()).expect("reopen settings");
    assert_eq!(reloaded.current_model(), Some(&expected));
}

#[tokio::test]
async fn test_app_provider_rm_resets_selected_model() {
    use crate::settings::{ModelRef, Settings};

    let mut app = App::new().unwrap();

    let provider = Provider::create(app.conn(), "test", InterfaceId::Openai, "key", None)
        .expect("create provider");
    let _other = Provider::create(app.conn(), "other", InterfaceId::Openai, "key", None)
        .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");

    // Deleting an unrelated provider leaves the selection alone.
    app.switch_model(provider, model.clone()).unwrap();
    app.process_command("/provider rm other").await.unwrap();
    assert!(app.selected_model().is_some());
    let reloaded = Settings::open(app.db_url()).expect("reopen settings");
    assert_eq!(
        reloaded.current_model(),
        Some(&ModelRef {
            provider: "test".to_string(),
            model: "gpt-4".to_string()
        })
    );

    // Deleting the selected provider clears the selection and the setting.
    app.process_command("/provider rm test").await.unwrap();
    assert!(app.selected_model().is_none());
    let reloaded = Settings::open(app.db_url()).expect("reopen settings");
    assert_eq!(reloaded.current_model(), None);

    // After a fresh start on a db where the provider was removed elsewhere,
    // a stale persisted ref resolves to no selection.
    let provider2 =
        Provider::create(app.conn(), "test2", InterfaceId::Openai, "key", None).unwrap();
    let model2 = Model::create(app.conn(), provider2.id, "gpt-5").unwrap();
    app.switch_model(provider2, model2).unwrap();
    Provider::delete_by_name(app.conn(), "test2").expect("delete failed");
    assert_eq!(app.selected_model().map(|(_, m)| m.id.as_str()), Some("gpt-5"));
}

#[tokio::test]
async fn test_app_process_command() {
    let mut app = App::new().unwrap();

    app.tools_mut().add_result(
        "sh",
        json!({
            "stdout": "output line\n",
            "stderr": "",
            "return_code": 0,
        }),
    );

    app.process_command("!echo test").await.unwrap();
    app.await_task().await.unwrap();
    app.process_pending_events().await;

    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        ToolCall {
            name: "sh".to_owned(),
            args: json!({ "command": "echo test" }),
        }
    );

    let history = app
        .query("/chat/stacked/inner/history/history/items")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    let last_two = &history[history.len() - 2..];
    assert_eq!(last_two[0]["content"]["type"], json!("command_prompt"));
    assert_eq!(last_two[0]["content"]["value"], json!("$ echo test"));
    assert_eq!(last_two[1]["content"]["type"], json!("command_output"));
    assert_eq!(last_two[1]["content"]["value"], json!("output line\n"));

    app.tools_mut().add_result(
        "sh",
        json!({
            "stdout": "string output\n",
            "stderr": "",
            "return_code": 0,
        }),
    );
    app.process_command("!pwd").await.unwrap();
    app.await_task().await.unwrap();
    app.process_pending_events().await;
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].args, json!({ "command": "pwd" }));

    app.tools_mut().add_result(
        "sh",
        json!({
            "stdout": "",
            "stderr": "error message\n",
            "return_code": 1,
        }),
    );
    app.process_command("!false").await.unwrap();
    app.await_task().await.unwrap();
    app.process_pending_events().await;
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 3);

    let history = app
        .query("/chat/stacked/inner/history/history/items")
        .unwrap();
    let last = history.as_array().unwrap().last().unwrap();
    assert_eq!(last["content"]["type"], json!("error"));
    assert_eq!(last["content"]["value"], json!("command exited with code 1: error message\n"));

    app.tools_mut().add_result(
        "sh",
        json!({
            "stdout": "",
            "stderr": "",
            "return_code": 0,
        }),
    );
    app.process_command("!true").await.unwrap();
    app.await_task().await.unwrap();
    app.process_pending_events().await;
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 4);

    app.tools_mut().add_result("sh", json!({"error": "tool error"}));
    app.process_event(AppEvent::SubmitPrompt("!failing_tool".to_string()))
        .await;
    app.await_task().await.unwrap();
    app.process_pending_events().await;
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 5);

    let history = app
        .query("/chat/stacked/inner/history/history/items")
        .unwrap();
    let last = history.as_array().unwrap().last().unwrap();
    assert_eq!(last["content"]["type"], json!("error"));
    assert_eq!(last["content"]["value"], json!("tool error"));

    assert!(app.process_command("").await.is_ok());
    assert!(app.process_command("   ").await.is_ok());
}

#[tokio::test]
async fn test_app_query() {
    let mut app = App::new().unwrap();

    // Primitive and unset fields.
    let db_url = app.query("/db_url").unwrap();
    assert!(db_url.as_str().unwrap().starts_with("file:"));
    assert!(db_url.as_str().unwrap().contains("mode=memory"));
    assert_eq!(app.query("/selected_model").unwrap(), json!(null));
    assert_eq!(app.query("/session").unwrap(), json!(null));
    assert_eq!(app.query("/current_task").unwrap(), json!(null));
    assert_eq!(app.query("/task_count").unwrap(), json!(0));

    // Nested query into chat. The app's chat starts with the greeting.
    use crate::ui::chat::Update;
    use crate::ui::Component;

    let ui = crate::testing::ui_context();
    let mut chat = Chat::new(&ui, 80, 24, &THEME_DARK);
    chat.handle_update(Update::HelpMessage(
        "Welcome to NagaiCode!\n\nType /help for a list of commands.",
    ));
    let expected_chat = chat.query("/").unwrap();
    assert_eq!(app.query("/chat").unwrap(), expected_chat);
    assert_eq!(app.query("/chat/stacked/h_padding").unwrap(), json!(2));
    assert_eq!(app.query("/chat/stacked/v_padding").unwrap(), json!(1));
    assert_eq!(
        app.query("/chat/stacked/inner/focus_state").unwrap(),
        json!("command_editor")
    );

    assert_eq!(
        app.query("/").unwrap(),
        json!({
            "chat": expected_chat,
            "selected_model": null,
            "db_url": db_url,
            "session": null,
            "current_task": null,
            "task_count": 0,
        })
    );

    // Selecting a model exposes it as a nested query.
    let provider =
        Provider::create(app.conn(), "test", InterfaceId::Openai, "key123", None).unwrap();
    let model = Model::create(app.conn(), provider.id, "gpt-4").unwrap();
    app.switch_model(provider.clone(), model.clone()).unwrap();
    assert_eq!(app.query("/selected_model/id").unwrap(), json!("gpt-4"));
    assert_eq!(
        app.query("/selected_model").unwrap(),
        model.query("/").unwrap()
    );
}

/// Returns the content of the most recent history item.
fn last_content(app: &App) -> String {
    app.query("/chat/stacked/inner/history/history/items")
        .unwrap()
        .as_array()
        .unwrap()
        .last()
        .unwrap()["content"]["value"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn test_app_session_ls() {
    use crate::session::Session;

    let mut app = App::new().unwrap();

    let date = |s: &Session| s.created_at.format("%Y-%m-%d %H:%M").to_string();

    // No sessions yet.
    app.process_command("/session ls").await.unwrap();
    assert_eq!(last_content(&app), "No sessions yet.");

    // Stored sessions are listed with no marker while none is active. Long
    // names are truncated to 30 columns.
    let s1 = Session::create(app.conn(), "first").unwrap();
    let s2 = Session::create(
        app.conn(),
        "this session name is definitely longer than thirty columns",
    )
    .unwrap();
    app.process_command("/s").await.unwrap();
    let expected = format!(
        "     1  {:<30}  {}\n     2  {}  {}\n",
        "first",
        date(&s1),
        "this session name is definitel",
        date(&s2),
    );
    assert_eq!(last_content(&app), expected);

    // Submitting a prompt lazily creates the active session, marked with '*'.
    let provider = Provider::create(app.conn(), "test", InterfaceId::Openai, "key", None)
        .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");
    app.switch_model(provider, model).unwrap();
    app.process_command("hello world").await.unwrap();
    app.process_event(AppEvent::Interrupt).await;
    app.process_pending_events().await;

    let s3_id = app.query("/session/id").unwrap().as_i64().unwrap() as i32;
    let s3 = Session::get_by_id(app.conn(), s3_id)
        .unwrap()
        .expect("active session");
    assert_eq!(s3.name, "hello world");
    assert_eq!(s3.id, s2.id + 1);
    app.process_command("/session ls").await.unwrap();
    let expected = format!(
        "     1  {:<30}  {}\n     2  {}  {}\n*    3  {:<30}  {}\n",
        "first",
        date(&s1),
        "this session name is definitel",
        date(&s2),
        "hello world",
        date(&s3),
    );
    assert_eq!(last_content(&app), expected);
}

#[tokio::test]
async fn test_app_session_new() {
    use crate::session::Session;

    let mut app = App::new().unwrap();

    let provider = Provider::create(app.conn(), "test", InterfaceId::Openai, "key", None)
        .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");
    app.switch_model(provider, model).unwrap();

    // Submitting a prompt creates a session and a running agent task.
    app.process_command("hello world").await.unwrap();
    assert_eq!(app.query("/current_task").unwrap(), json!(0));
    let old_id = app.query("/session/id").unwrap().as_i64().unwrap() as i32;
    let old = Session::get_by_id(app.conn(), old_id).unwrap().unwrap();
    assert_eq!(old.name, "hello world");

    // /session new cancels the task, purges the event queue, resets the
    // session, and rebuilds the UI from scratch.
    app.process_command("/session new").await.unwrap();
    assert_eq!(app.query("/session").unwrap(), json!(null));
    assert_eq!(app.query("/current_task").unwrap(), json!(null));
    assert_eq!(app.query("/task_count").unwrap(), json!(0));
    let mut canvas = app.make_canvas();
    app.draw(&mut canvas);
    assert_eq!(render_canvas(&mut canvas), EXPECTED_INITIAL_FRAME);

    // Interrupt feedback from the canceled task was purged with the queue.
    assert_eq!(interrupted_count(&app), 0);

    // The next prompt lazily creates a fresh, starred session.
    app.process_command("round two").await.unwrap();
    let new_id = app.query("/session/id").unwrap().as_i64().unwrap() as i32;
    assert_eq!(new_id, old_id + 1);
    let fresh = Session::get_by_id(app.conn(), new_id).unwrap().unwrap();
    assert_eq!(fresh.name, "round two");
    app.process_command("/session ls").await.unwrap();
    let date = |s: &Session| s.created_at.format("%Y-%m-%d %H:%M").to_string();
    let expected = format!(
        "  {old_id:>4}  {:<30}  {}\n* {new_id:>4}  {:<30}  {}\n",
        "hello world",
        date(&old),
        "round two",
        date(&fresh),
    );
    assert_eq!(last_content(&app), expected);
}

#[tokio::test]
async fn test_app_session_switch() {
    use crate::session::{Item, ItemType, NewItem, Session, Turn, TurnType};

    let mut app = App::new().unwrap();

    let provider = Provider::create(app.conn(), "test", InterfaceId::Openai, "key", None)
        .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");
    app.switch_model(provider, model).unwrap();

    // Fake completed session with a prompt and a response.
    let session = Session::create(app.conn(), "restored").unwrap();
    let turn = Turn::create(app.conn(), session.id, TurnType::User, None, None, None)
        .expect("create turn");
    for (ty, text) in [
        (ItemType::UserText, "hello from the past"),
        (ItemType::ResponseText, "restored reply"),
    ] {
        Item::create(
            app.conn(),
            NewItem {
                session_id: Some(session.id),
                turn_id: Some(turn.id),
                ty: Some(ty),
                text: Some(text),
                ..Default::default()
            },
        )
        .expect("create item");
    }

    // Switching cancels the running task and purges its events.
    app.process_command("hello world").await.unwrap();
    assert_eq!(app.query("/current_task").unwrap(), json!(0));
    app.process_command(&format!("/session switch {}", session.id))
        .await
        .unwrap();
    assert_eq!(app.query("/session/id").unwrap(), json!(session.id));
    assert_eq!(app.query("/current_task").unwrap(), json!(null));
    assert_eq!(app.query("/task_count").unwrap(), json!(0));
    assert_eq!(interrupted_count(&app), 0);

    // The history is rebuilt from the session's items, without the greeting.
    let history = app
        .query("/chat/stacked/inner/history/history/items")
        .unwrap();
    let items = history
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["content"]["type"].as_str().unwrap(),
                item["content"]["value"].as_str().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        items,
        [("user", "hello from the past"), ("response", "restored reply")]
    );
    let mut canvas = app.make_canvas();
    app.draw(&mut canvas);
    assert!(!render_canvas(&mut canvas).contains("Welcome to NagaiCode!"));

    // The next prompt continues in the switched session.
    app.process_command("one more thing").await.unwrap();
    assert_eq!(app.query("/session/id").unwrap(), json!(session.id));
}

#[tokio::test]
async fn test_app_session_switch_missing() {
    let mut app = App::new().unwrap();

    // Switching to a session that does not exist errors and leaves the
    // initial UI untouched.
    assert!(app.process_command("/session switch 999").await.is_err());
    assert_eq!(app.query("/session").unwrap(), json!(null));
    let mut canvas = app.make_canvas();
    app.draw(&mut canvas);
    assert_eq!(render_canvas(&mut canvas), EXPECTED_INITIAL_FRAME);
}
