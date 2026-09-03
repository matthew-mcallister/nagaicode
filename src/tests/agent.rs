use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use reqwest::{Method, Request};
use reqwest_eventsource::Event as SseEvent;
use serde_json::json;

use anyhow::anyhow;

use crate::app::App;
use crate::error::AnyResult;
use crate::interface::InterfaceId;
use crate::model::Model;
use crate::provider::Provider;
use crate::query::DataQuery;
use crate::request::test_client::ResponseData;
use crate::testing::QueueStream;

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
        .map(|item| item["content"]["value"].as_str().unwrap().to_owned())
        .collect();
    assert!(history.iter().any(|c| c == "hello"));
    assert_eq!(
        history.last().map(String::as_str),
        Some("Hello there, how can I help?")
    );

    // The prompt and the response should both be persisted in the database.
    // Session was created
    assert_eq!(app.query("/session/name").unwrap(), json!("hello"));
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

    let items = crate::session::DbItem::list_by_session(app.conn(), session_id).unwrap();
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
async fn test_agent_stream_error() {
    use crate::session::{DbItem, ItemType, Response, Turn, TurnType};

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

    // The stream fails mid-response after some output has been emitted.
    app.client_mut().add_response(
        url,
        ResponseData::Sse(QueueStream::from(vec![
            Ok(SseEvent::Open),
            Ok(create_message_event(
                r#"{"type":"response.created","response":{"id":"resp-1","status":"in_progress"}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"msg_1","type":"message","status":"in_progress","role":"assistant","content":[]}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"delta":"Hello"}"#,
            )),
            Err(anyhow!("network error")),
        ])),
    );

    for c in "hello".chars() {
        app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char(c))))
            .await;
    }
    app.handle_input(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT))).await;
    app.await_task().await.expect("await agent task");
    app.process_pending_events().await;

    // The error should be reported in the chat history.
    let history = app
        .query("/chat/stacked/inner/history/history/items")
        .unwrap();
    let history = history.as_array().unwrap();
    let last = history.last().unwrap();
    assert_eq!(last["content"]["type"], json!("error"));
    assert_eq!(last["content"]["value"], json!("network error"));

    // The partial output should be persisted and the response marked failed.
    let session_id = app.query("/session/id").unwrap().as_i64().unwrap() as i32;

    let turns = Turn::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].ty().unwrap(), TurnType::User);
    assert_eq!(turns[1].ty().unwrap(), TurnType::Assistant);

    let responses = Response::list_by_turn(app.conn(), turns[1].id).unwrap();
    assert_eq!(responses.len(), 1);
    assert_eq!(responses[0].upstream_status.as_deref(), Some("failed"));

    let items = DbItem::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].ty().unwrap(), ItemType::UserText);
    assert_eq!(items[1].ty().unwrap(), ItemType::ResponseText);
    assert_eq!(items[1].text.as_deref(), Some("Hello"));
}

#[tokio::test]
async fn test_agent_out_of_order_items() {
    use crate::session::{DbItem, ItemType};

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

    app.client_mut().add_response(
        url,
        ResponseData::Sse(QueueStream::from(vec![
            Ok(SseEvent::Open),
            Ok(create_message_event(
                r#"{"type":"response.created","response":{"id":"resp-1","status":"in_progress"}}"#,
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
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.reasoning_text.delta","item_id":"rs_1","output_index":0,"delta":"I should add."}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1","type":"reasoning","summary":[]}}"#,
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
    app.await_task().await.expect("await agent task");
    app.process_pending_events().await;

    let session_id = app.query("/session/id").unwrap().as_i64().unwrap() as i32;
    let items = DbItem::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].ty().unwrap(), ItemType::UserText);

    let reasoning = &items[1];
    assert_eq!(reasoning.ty().unwrap(), ItemType::Reasoning);
    assert_eq!(reasoning.upstream_id.as_deref(), Some("rs_1"));
    assert_eq!(reasoning.text.as_deref(), Some("I should add."));

    let answer = &items[2];
    assert_eq!(answer.ty().unwrap(), ItemType::ResponseText);
    assert_eq!(answer.upstream_id.as_deref(), Some("msg_1"));
    assert_eq!(answer.text.as_deref(), Some("The answer is 2."));

    assert!(
        answer.id < reasoning.id,
        "expected items inserted in arrival order"
    );
    assert_eq!(
        reasoning.seqno,
        items[0].seqno + 1,
        "expected seqno to follow output_index, not arrival order"
    );
    assert_eq!(answer.seqno, reasoning.seqno + 1);
}

#[tokio::test]
async fn test_agent_history() {
    use crate::session::{DbItem, ItemType, Turn, TurnType};

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
    let items = DbItem::list_by_session(app.conn(), session_id).unwrap();
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
            {"type": "message", "role": "user", "content": "what is 1+1?"},
            {"type": "reasoning", "content": [{"type": "reasoning_text", "text": "I should add."}]},
            {"type": "message", "role": "assistant", "content": "The answer is 2."},
            {"type": "message", "role": "user", "content": "thanks!"}
        ])
    );
}

#[tokio::test]
async fn test_agent_tool_call_loop() {
    use crate::session::{DbItem, ItemType, Response, Turn, TurnType};

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

    // First response: the model requests a tool call. The arguments stream in
    // as deltas and are only persisted once the item is done.
    app.client_mut().add_response(
        url,
        ResponseData::Sse(QueueStream::from(vec![
            Ok(SseEvent::Open),
            Ok(create_message_event(
                r#"{"type":"response.created","response":{"id":"resp-1","status":"in_progress"}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"fc_1","type":"function_call","status":"in_progress","name":"sh","call_id":"call_1","arguments":""}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":"{\"command\": \"printf"}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.function_call_arguments.delta","item_id":"fc_1","output_index":0,"delta":" 3\"}"}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"fc_1","type":"function_call","status":"completed","name":"sh","call_id":"call_1","arguments":"{\"command\": \"printf 3\"}"}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.completed","response":{"id":"resp-1","status":"completed"}}"#,
            )),
            Ok(create_message_event("[DONE]")),
        ])),
    );

    // Second response: the model consumes the tool output and answers.
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
                r#"{"type":"response.output_text.delta","item_id":"msg_2","output_index":0,"delta":"The answer is 3."}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"msg_2","type":"message","status":"completed","role":"assistant","content":[{"type":"output_text","text":"The answer is 3."}]}}"#,
            )),
            Ok(create_message_event(
                r#"{"type":"response.completed","response":{"id":"resp-2","status":"completed"}}"#,
            )),
            Ok(create_message_event("[DONE]")),
        ])),
    );

    for c in "call the sh tool".chars() {
        app.handle_input(Event::Key(KeyEvent::from(KeyCode::Char(c))))
            .await;
    }
    app.handle_input(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT))).await;
    app.await_task().await.expect("await agent task");
    app.process_pending_events().await;

    // The agent should have made two inference requests within a single turn.
    let requests = app.client_mut().get_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].url().as_str(), url);
    assert_eq!(requests[1].url().as_str(), url);

    // The second request replays the tool call and its output.
    let body = request_body_value(&requests[1]);
    assert_eq!(
        body["input"],
        json!([
            {"type": "message", "role": "user", "content": "call the sh tool"},
            {
                "type": "function_call",
                "call_id": "call_1",
                "name": "sh",
                "arguments": r#"{"command": "printf 3"}"#,
            },
            {
                "type": "function_call_output",
                "call_id": "call_1",
                "output": [
                    {"type": "input_text", "text": "stdout:\n3"},
                    {"type": "input_text", "text": "return code: 0"},
                ],
            },
        ])
    );

    // Both responses live in one assistant turn.
    let session_id = app.query("/session/id").unwrap().as_i64().unwrap() as i32;
    let turns = Turn::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(turns.len(), 2);
    assert_eq!(turns[0].ty().unwrap(), TurnType::User);
    assert_eq!(turns[1].ty().unwrap(), TurnType::Assistant);

    let responses = Response::list_by_turn(app.conn(), turns[1].id).unwrap();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0].upstream_id.as_deref(), Some("resp-1"));
    assert_eq!(responses[0].upstream_status.as_deref(), Some("completed"));
    assert_eq!(responses[1].upstream_id.as_deref(), Some("resp-2"));
    assert_eq!(responses[1].upstream_status.as_deref(), Some("completed"));

    let items = DbItem::list_by_session(app.conn(), session_id).unwrap();
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].ty().unwrap(), ItemType::UserText);
    assert_eq!(items[1].ty().unwrap(), ItemType::ToolCall);
    assert_eq!(items[1].upstream_id.as_deref(), Some("fc_1"));
    assert_eq!(items[1].upstream_call_id.as_deref(), Some("call_1"));
    assert_eq!(items[1].text.as_deref(), Some("sh"));
    assert_eq!(items[1].tool_args.as_deref(), Some(r#"{"command": "printf 3"}"#));
    assert_eq!(
        items[1].tool_output().unwrap(),
        Some(json!({ "stdout": "3", "stderr": "", "return_code": 0 }))
    );
    assert_eq!(items[2].ty().unwrap(), ItemType::ResponseText);
    assert_eq!(items[2].upstream_id.as_deref(), Some("msg_2"));
    assert_eq!(items[2].text.as_deref(), Some("The answer is 3."));
}
