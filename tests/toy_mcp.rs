use assert_cmd::Command;
use serde_json::{Value, json};

fn echo_request(id: i64, arguments: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "method": "tools/call",
        "params": { "name": "echo", "arguments": arguments },
        "id": id,
    })
    .to_string()
}

#[test]
fn test_stdio_echo() {
    let input = format!(
        "{}\n{}\n",
        echo_request(1, json!({ "a": [1, 2, { "b": null }] })),
        echo_request(2, json!("hello")),
    );

    let output = Command::cargo_bin("toy_mcp")
        .unwrap()
        .arg("stdio")
        .write_stdin(input)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let responses: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();

    assert_eq!(responses.len(), 2, "stdout: {stdout}");

    let expected = [json!({ "a": [1, 2, { "b": null }] }), json!("hello")];

    for (index, (response, arguments)) in responses.iter().zip(expected).enumerate() {
        assert_eq!(response["jsonrpc"], json!("2.0"));
        assert_eq!(response["id"], json!(index as i64 + 1));
        assert!(response.get("error").is_none());
        assert_eq!(response["result"]["resultType"], json!("complete"));
        assert_eq!(response["result"]["isError"], json!(false));
        assert_eq!(response["result"]["structuredContent"], arguments);
        assert_eq!(
            response["result"]["content"][0]["text"],
            json!(arguments.to_string()),
        );
    }
}
