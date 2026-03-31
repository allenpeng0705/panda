use serde_json::json;

pub fn is_anthropic_provider(cfg: &panda_config::PandaConfig) -> bool {
    cfg.adapter.provider == "anthropic"
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
    fn maps_anthropic_to_openai_shape() {
        let body = br#"{"model":"claude","content":[{"type":"text","text":"hello"}]}"#;
        let out = anthropic_to_openai_chat(body, None).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["object"], "chat.completion");
        assert_eq!(v["choices"][0]["message"]["content"], "hello");
    }
}
