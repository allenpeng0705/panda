use serde_json::json;

pub fn is_anthropic_provider(cfg: &panda_config::PandaConfig, ingress_path: &str) -> bool {
    cfg.effective_adapter_provider(ingress_path) == "anthropic"
}

pub fn openai_chat_to_anthropic(body: &[u8]) -> anyhow::Result<(Vec<u8>, bool)> {
    let v: serde_json::Value = serde_json::from_slice(body)?;
    let streaming = v.get("stream").and_then(|x| x.as_bool()).unwrap_or(false);
    let model = v
        .get("model")
        .and_then(|x| x.as_str())
        .unwrap_or("claude-3-5-sonnet-latest");
    let max_tokens = v.get("max_tokens").and_then(|x| x.as_u64()).unwrap_or(1024);
    let mut msgs = Vec::new();
    if let Some(messages) = v.get("messages").and_then(|m| m.as_array()) {
        for m in messages {
            let role = m.get("role").and_then(|x| x.as_str()).unwrap_or("user");
            let content = m
                .get("content")
                .and_then(|x| x.as_str())
                .unwrap_or_default()
                .to_string();
            msgs.push(json!({
                "role": if role == "assistant" { "assistant" } else { "user" },
                "content": content
            }));
        }
    }
    let mut out = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": msgs
    });
    if let Some(tools) = v.get("tools").and_then(|t| t.as_array()) {
        let mapped_tools: Vec<serde_json::Value> = tools
            .iter()
            .filter_map(|t| {
                let f = t.get("function")?;
                let name = f.get("name")?.as_str()?;
                let input_schema = f
                    .get("parameters")
                    .cloned()
                    .unwrap_or_else(|| json!({"type":"object","properties":{}}));
                Some(json!({
                    "name": name,
                    "input_schema": input_schema
                }))
            })
            .collect();
        if !mapped_tools.is_empty() {
            if let Some(obj) = out.as_object_mut() {
                obj.insert("tools".to_string(), serde_json::Value::Array(mapped_tools));
            }
        }
    }
    if let Some(tc) = v.get("tool_choice") {
        let mapped_choice = if tc.is_string() {
            tc.as_str().map(|s| match s {
                "none" => json!({"type":"none"}),
                "required" => json!({"type":"any"}),
                _ => json!({"type":"auto"}),
            })
        } else {
            tc.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(|name| json!({"type":"tool","name":name}))
        };
        if let Some(choice) = mapped_choice {
            if let Some(obj) = out.as_object_mut() {
                obj.insert("tool_choice".to_string(), choice);
            }
        }
    }
    if streaming {
        if let Some(obj) = out.as_object_mut() {
            obj.insert("stream".to_string(), json!(true));
        }
    }
    Ok((serde_json::to_vec(&out)?, streaming))
}

pub fn anthropic_to_openai_chat(body: &[u8], model_hint: Option<&str>) -> anyhow::Result<Vec<u8>> {
    let v: serde_json::Value = serde_json::from_slice(body)?;
    let model = v
        .get("model")
        .and_then(|x| x.as_str())
        .or(model_hint)
        .unwrap_or("unknown-model");
    let text = v
        .get("content")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("text"))
        .and_then(|x| x.as_str())
        .unwrap_or_default();
    let out = json!({
        "id": "chatcmpl-panda-adapter",
        "object": "chat.completion",
        "created": (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)),
        "model": model,
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": text
            },
            "finish_reason": "stop"
        }]
    });
    Ok(serde_json::to_vec(&out)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_openai_to_anthropic_shape() {
        let body = br#"{"model":"gpt-4o-mini","max_tokens":50,"messages":[{"role":"user","content":"hi"}]}"#;
        let (out, streaming) = openai_chat_to_anthropic(body).unwrap();
        assert!(!streaming);
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["model"], "gpt-4o-mini");
        assert_eq!(v["messages"][0]["role"], "user");
    }

    #[test]
    fn maps_openai_streaming_to_anthropic_stream_request() {
        let body = br#"{"model":"gpt-4o-mini","stream":true,"messages":[{"role":"user","content":"hi"}]}"#;
        let (out, streaming) = openai_chat_to_anthropic(body).unwrap();
        assert!(streaming);
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["stream"], true);
    }

    #[test]
    fn maps_openai_tools_to_anthropic_tools() {
        let body = br#"{
            "model":"gpt-4o-mini",
            "messages":[{"role":"user","content":"weather?"}],
            "tools":[{"type":"function","function":{"name":"get_weather","parameters":{"type":"object","properties":{"city":{"type":"string"}}}}}],
            "tool_choice":"required"
        }"#;
        let (out, _) = openai_chat_to_anthropic(body).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["tools"][0]["name"], "get_weather");
        assert_eq!(v["tool_choice"]["type"], "any");
    }

    #[test]
    fn maps_anthropic_to_openai_shape() {
        let body = br#"{"model":"claude","content":[{"type":"text","text":"hello"}]}"#;
        let out = anthropic_to_openai_chat(body, None).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["object"], "chat.completion");
        assert_eq!(v["choices"][0]["message"]["content"], "hello");
    }
}
