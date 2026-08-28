use anyhow::anyhow;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use reqwest::{Method, Request};
use reqwest_eventsource::Event as SseEvent;
use serde_json::json;

use crate::app::{App, AppEvent};
use crate::error::AnyResult;
use crate::interface::InterfaceId;
use crate::model::Model;
use crate::provider::Provider;
use crate::query::DataQuery;
use crate::request::test_client::ResponseData;
use crate::testing::QueueStream;
use crate::tools::ToolResult;
use crate::tools::mock::ToolCall;
use crate::ui::canvas::render_canvas;
use crate::ui::chat::Chat;
use crate::ui::style::THEME_DARK;

fn create_message_event(data: &str) -> SseEvent {
    SseEvent::Message(eventsource_stream::Event {
        event: "message".to_string(),
        data: data.to_string(),
        id: "".to_string(),
        retry: None,
    })
}

fn request_body_value(req: &Request) -> serde_json::Value {
    let bytes = req.body().and_then(|b| b.as_bytes()).unwrap_or(&[]);
    serde_json::from_slice(bytes).unwrap()
}

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
        .filter(|item| item["content"].as_str() == Some("Interrupted."))
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
        Ok(ToolResult::success(json!({
            "stdout": "output line\n",
            "stderr": "",
            "return_code": 0,
        }))),
    );

    app.process_command("!echo test").await.unwrap();

    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        ToolCall {
            name: "sh".to_owned(),
            args: json!("echo test"),
        }
    );

    app.tools_mut()
        .add_result("sh", Ok(ToolResult::success(json!("string output"))));
    app.process_command("!pwd").await.unwrap();
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
    app.process_command("!false").await.unwrap();
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
    app.process_command("!true").await.unwrap();
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 4);

    app.tools_mut().add_result("sh", Err(anyhow!("tool error")));
    app.process_event(AppEvent::Command("!failing_tool".to_string()))
        .await;
    let calls = app.tools().get_calls();
    assert_eq!(calls.len(), 5);

    assert!(app.process_command("").await.is_ok());
    assert!(app.process_command("   ").await.is_ok());
}

#[tokio::test]
async fn test_app_prompt_agent() {
    use crate::session::{ItemType, Response, Turn, TurnType};

    let mut app = App::new().unwrap();

    let provider = Provider::create(
        app.conn(),
        "test",
        InterfaceId::Openai,
        "sk-test",
        Some("https://example.test/v1"),
    )
    .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");
    app.switch_model(provider.clone(), model).unwrap();

    let url = "https://example.test/v1/responses";
    let events: Vec<AnyResult<SseEvent>> = vec![
        Ok(SseEvent::Open),
        Ok(create_message_event(
            r#"{"type":"response.created","response":{"id":"resp-1","status":"in_progress"}}"#,
        )),
        Ok(create_message_event(
            r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_1","type":"message","status":"in_progress","role":"assistant","content":[]}}"#,
        )),
        Ok(create_message_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"delta":"Hello there, "}"#,
        )),
        Ok(create_message_event(
            r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"delta":"how can I help?"}"#,
        )),
        Ok(create_message_event(
            r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_1","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Hello there, how can I help?"}]}}"#,
        )),
        Ok(create_message_event(
            r#"{"type":"response.completed","response":{"id":"resp-1","status":"completed","usage":{"input_tokens":12,"output_tokens":18,"total_tokens":30,"input_tokens_details":{"cached_tokens":4},"output_tokens_details":{"reasoning_tokens":7}}}}"#,
        )),
        Ok(create_message_event("[DONE]")),
    ];
    app.client_mut()
        .add_response(url, ResponseData::Sse(QueueStream::from(events)));

    // Drive the terminal to input a prompt and submit it.
    for c in "hello".chars() {
        app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char(c))))
            .await;
    }
    app.handle_input(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT))).await;

    // Await the spawned agent, then fold its events into the UI.
    app.await_task().await.expect("await agent task");
    app.process_pending_events().await;

    // The agent should have made a single inference request.
    let requests = app.client_mut().get_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method(), Method::POST);
    assert_eq!(requests[0].url().as_str(), url);
    let body = request_body_value(&requests[0]);
    assert_eq!(body["model"], "gpt-4");
    assert_eq!(body["stream"], true);
    assert_eq!(body["input"][0]["role"], "user");
    assert_eq!(body["input"][0]["content"], "hello");

    // The prompt should be recorded in the app state.
    assert_eq!(
        app.query("/chat/stacked/inner/input/command_history")
            .unwrap(),
        json!(["hello"])
    );

    // The model response (distinct from the prompt) should appear in the chat
    // history alongside the prompt. The initial help message is also present.
    let history: Vec<String> = app
        .query("/chat/stacked/inner/history/history/items")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["content"].as_str().unwrap().to_owned())
        .collect();
    assert!(history.iter().any(|c| c == "hello"));
    assert_eq!(
        history.last().map(String::as_str),
        Some("Hello there, how can I help?")
    );

    // The prompt and the response should both be persisted in the database.
    let session_id = app.query("/session/id").unwrap().as_i64().unwrap() as i32;

    let turns = Turn::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(
        turns.len(),
        2,
        "expected one user turn and one assistant turn"
    );
    assert_eq!(turns[0].ty().unwrap(), TurnType::User);
    assert_eq!(turns[0].provider_id, None);
    assert_eq!(turns[0].provider_name, None);
    assert_eq!(turns[0].model_id, None);
    let assistant_turn = &turns[1];
    assert_eq!(assistant_turn.ty().unwrap(), TurnType::Assistant);
    assert_eq!(assistant_turn.provider_id, Some(provider.id));
    assert_eq!(assistant_turn.provider_name.as_deref(), Some("test"));
    assert_eq!(assistant_turn.model_id.as_deref(), Some("gpt-4"));

    let items = crate::session::Item::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(
        items.len(),
        2,
        "expected one user item and one response item"
    );

    assert_eq!(items[0].ty().unwrap(), ItemType::UserText);
    assert_eq!(items[0].turn_id, turns[0].id);
    assert_eq!(items[0].response_id, None);
    assert_eq!(items[0].text.as_deref(), Some("hello"));

    let response_item = &items[1];
    assert_eq!(response_item.ty().unwrap(), ItemType::ResponseText);
    assert_eq!(response_item.turn_id, assistant_turn.id);
    assert_eq!(response_item.upstream_id.as_deref(), Some("msg_1"));
    assert_eq!(response_item.upstream_type.as_deref(), Some("message"));
    assert_eq!(
        response_item.text.as_deref(),
        Some("Hello there, how can I help?")
    );
    assert!(response_item.raw_data.is_some());

    let responses = Response::list_by_turn(app.conn(), assistant_turn.id).unwrap();
    assert_eq!(responses.len(), 1);
    let response = &responses[0];
    assert_eq!(response.session_id, session_id);
    assert_eq!(response.upstream_id.as_deref(), Some("resp-1"));
    assert_eq!(response.upstream_status.as_deref(), Some("completed"));
    assert_eq!(response.input_tokens, Some(12));
    assert_eq!(response.cached_input_tokens, Some(4));
    assert_eq!(response.output_tokens, Some(18));
    assert_eq!(response.reasoning_tokens, Some(7));
    assert_eq!(response.total_tokens, Some(30));
    assert_eq!(response.raw_request, None);
    assert!(response.raw_response.is_some());
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(response.raw_response.as_ref().unwrap())
            .unwrap()["status"],
        "completed"
    );
}

#[tokio::test]
async fn test_agent_stream_without_item_events() {
    use crate::session::{Item, ItemType, Response, Turn};

    let mut app = App::new().unwrap();

    let provider = Provider::create(
        app.conn(),
        "test",
        InterfaceId::Openai,
        "sk-test",
        Some("https://example.test/v1"),
    )
    .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");
    app.switch_model(provider, model).unwrap();

    let url = "https://example.test/v1/responses";

    // A provider which streams deltas without output item lifecycle events.
    // The agent must create the item on demand from the first delta.
    app.client_mut().add_response(
        url,
        ResponseData::Sse(QueueStream::from(vec![
            Ok(SseEvent::Open),
            Ok(create_message_event(
                r#"{"type":"response.created","response":{"id":"resp-1","status":"in_progress"}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","delta":"Hi"}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","delta":" there"}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.completed","response":{"id":"resp-1","status":"completed","usage":{"input_tokens":5,"output_tokens":6,"total_tokens":11}}}"#,
            )),
            Ok(create_message_event("[DONE]")),
        ])),
    );

    for c in "hello".chars() {
        app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char(c))))
            .await;
    }
    app.handle_input(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT))).await;
    app.await_task().await.expect("await agent task");
    app.process_pending_events().await;

    let session_id = app.query("/session/id").unwrap().as_i64().unwrap() as i32;
    let items = Item::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].ty().unwrap(), ItemType::UserText);

    let answer = &items[1];
    assert_eq!(answer.ty().unwrap(), ItemType::ResponseText);
    assert_eq!(answer.upstream_id, None);
    assert_eq!(answer.upstream_type, None);
    assert_eq!(answer.text.as_deref(), Some("Hi there"));
    assert_eq!(answer.raw_data, None);

    let turns = Turn::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(turns.len(), 2);
    let responses = Response::list_by_turn(app.conn(), turns[1].id).unwrap();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].upstream_id.as_deref(), Some("resp-1"));
    assert_eq!(responses[0].upstream_status.as_deref(), Some("completed"));
    assert_eq!(responses[0].input_tokens, Some(5));
    assert_eq!(responses[0].cached_input_tokens, Some(0));
    assert_eq!(responses[0].output_tokens, Some(6));
    assert_eq!(responses[0].total_tokens, Some(11));
    assert!(responses[0].raw_response.is_some());
}

#[tokio::test]
async fn test_agent_history() {
    use crate::session::{Item, ItemType, Turn, TurnType};

    let mut app = App::new().unwrap();

    let provider = Provider::create(
        app.conn(),
        "test",
        InterfaceId::Openai,
        "sk-test",
        Some("https://example.test/v1"),
    )
    .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");
    app.switch_model(provider, model).unwrap();

    let url = "https://example.test/v1/responses";

    // First turn: the model streams a reasoning item followed by its answer.
    // Both are persisted to the session as separate items.
    app.client_mut().add_response(
        url,
        ResponseData::Sse(QueueStream::from(vec![
            Ok(SseEvent::Open),
            Ok(create_message_event(
                r#"{"type":"response.created","response":{"id":"resp-1","status":"in_progress"}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.reasoning_text.delta","item_id":"rs_1","output_index":0,"delta":"I should add."}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.added","output_index":1,"item":{"id":"msg_1","type":"message","status":"in_progress","role":"assistant","content":[]}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":1,"delta":"The answer is 2."}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.done","output_index":1,"item":{"id":"msg_1","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"The answer is 2."}]}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.completed","response":{"id":"resp-1","status":"completed"}}"#,
            )),
            Ok(create_message_event("[DONE]")),
        ])),
    );

    for c in "what is 1+1?".chars() {
        app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char(c))))
            .await;
    }
    app.handle_input(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT))).await;
    app.await_task().await.expect("await first agent");
    app.process_pending_events().await;

    // The first assistant turn should have recorded one reasoning item and
    // one response item, in that order, after the user prompt item.
    let session_id = app.query("/session/id").unwrap().as_i64().unwrap() as i32;
    let items = Item::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].ty().unwrap(), ItemType::UserText);
    assert_eq!(items[1].ty().unwrap(), ItemType::Reasoning);
    assert_eq!(items[1].upstream_id.as_deref(), Some("rs_1"));
    assert_eq!(items[1].upstream_type.as_deref(), Some("reasoning"));
    assert_eq!(items[1].text.as_deref(), Some("I should add."));
    assert_eq!(items[2].ty().unwrap(), ItemType::ResponseText);
    assert_eq!(items[2].text.as_deref(), Some("The answer is 2."));

    let turns = Turn::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].ty().unwrap(), TurnType::User);
    assert_eq!(turns[1].ty().unwrap(), TurnType::Assistant);

    // Second turn: the request must include the full conversation history,
    // with reasoning folded in where the interface supports it.
    app.client_mut().add_response(
        url,
        ResponseData::Sse(QueueStream::from(vec![
            Ok(SseEvent::Open),
            Ok(create_message_event(
                r#"{"type":"response.created","response":{"id":"resp-2","status":"in_progress"}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_2","type":"message","status":"in_progress","role":"assistant","content":[]}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_2","output_index":0,"delta":"You're welcome!"}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_2","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"You're welcome!"}]}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.completed","response":{"id":"resp-2","status":"completed"}}"#,
            )),
            Ok(create_message_event("[DONE]")),
        ])),
    );

    for c in "thanks!".chars() {
        app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char(c))))
            .await;
    }
    app.handle_input(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT))).await;
    app.await_task().await.expect("await second task");
    app.process_pending_events().await;

    let requests = app.client_mut().get_requests();
    assert_eq!(requests.len(), 2);
    let body = request_body_value(&requests[1]);
    assert_eq!(
        body["input"],
        json!([
            {"role": "user", "content": "what is 1+1?"},
            {"type": "reasoning", "summary": [{"type": "summary_text", "text": "I should add."}]},
            {"role": "assistant", "content": "The answer is 2."},
            {"role": "user", "content": "thanks!"}
        ])
    );
}

#[tokio::test]
async fn test_agent_tool_call() {
    use crate::session::{Item, ItemType, Response, Turn, TurnType};

    let mut app = App::new().unwrap();

    let provider = Provider::create(
        app.conn(),
        "test",
        InterfaceId::Openai,
        "sk-test",
        Some("https://example.test/v1"),
    )
    .expect("create provider");
    let model = Model::create(app.conn(), provider.id, "gpt-4").expect("create model");
    app.switch_model(provider, model).unwrap();

    let url = "https://example.test/v1/responses";

    // The model requests a tool call. The arguments stream in as deltas and
    // are only persisted once the item is done.
    app.client_mut().add_response(
        url,
        ResponseData::Sse(QueueStream::from(vec![
            Ok(SseEvent::Open),
            Ok(create_message_event(
                r#"{"type":"response.created","response":{"id":"resp-1","status":"in_progress"}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"fc_1","type":"function_call","status":"in_progress","name":"add","call_id":"call_1","arguments":""}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"{\"a\": 1"}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":", \"b\": 2}"}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"fc_1","type":"function_call","status":"completed","name":"add","call_id":"call_1","arguments":"{\"a\": 1, \"b\": 2}"}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.completed","response":{"id":"resp-1","status":"completed"}}"#,
            )),
            Ok(create_message_event("[DONE]")),
        ])),
    );

    for c in "call the add tool".chars() {
        app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char(c))))
            .await;
    }
    app.handle_input(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT))).await;
    app.await_task().await.expect("await agent task");
    app.process_pending_events().await;

    let session_id = app.query("/session/id").unwrap().as_i64().unwrap() as i32;
    let items = Item::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].ty().unwrap(), ItemType::UserText);

    let tool_call = &items[1];
    assert_eq!(tool_call.ty().unwrap(), ItemType::ToolCall);
    assert_eq!(tool_call.upstream_id.as_deref(), Some("fc_1"));
    assert_eq!(tool_call.upstream_type.as_deref(), Some("function_call"));
    assert_eq!(tool_call.text, None);
    assert_eq!(tool_call.summary, None);
    assert_eq!(tool_call.json.as_deref(), Some(r#"{"a": 1, "b": 2}"#));
    assert_eq!(tool_call.json().unwrap(), Some(json!({"a": 1, "b": 2})));
    assert!(tool_call.raw_data.is_some());
    let raw = serde_json::from_str::<serde_json::Value>(tool_call.raw_data.as_deref().unwrap())
        .unwrap();
    assert_eq!(raw["name"], "add");
    assert_eq!(raw["call_id"], "call_1");
    assert_eq!(raw["arguments"], r#"{"a": 1, "b": 2}"#);

    let turns = Turn::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].ty().unwrap(), TurnType::User);
    assert_eq!(turns[1].ty().unwrap(), TurnType::Assistant);
    let responses = Response::list_by_turn(app.conn(), turns[1].id).unwrap();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].upstream_status.as_deref(), Some("completed"));
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

    // Nested query into chat.
    let expected_chat = Chat::new(80, 24, &THEME_DARK).query("/").unwrap();
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
