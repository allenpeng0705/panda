//! Anthropic `text/event-stream` (Messages API) → OpenAI `chat.completion.chunk` SSE.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::{SystemTime, UNIX_EPOCH};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use hyper::body::{Body, Frame, Incoming};
use serde_json::{json, Value};
use uuid::Uuid;

pub struct AnthropicToOpenAiSseBody {
    inner: Incoming,
    buf: BytesMut,
    pending: VecDeque<Bytes>,
    openai_id: String,
    model: String,
    role_sent: bool,
    finished: bool,
    created: u64,
}

impl AnthropicToOpenAiSseBody {
    pub fn new(inner: Incoming, model_hint: Option<String>) -> Self {
        let created = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        Self {
            inner,
            buf: BytesMut::new(),
            pending: VecDeque::new(),
            openai_id: format!("chatcmpl-{}", Uuid::new_v4()),
            model: model_hint.unwrap_or_else(|| "claude".to_string()),
            role_sent: false,
            finished: false,
            created,
        }
    }

    fn enqueue_finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.pending.push_back(finish_chunk(
            &self.openai_id,
            &self.model,
            self.created,
        ));
        self.pending.push_back(Bytes::from("data: [DONE]\n\n"));
    }

    fn process_json_event(&mut self, v: &Value) {
        let Some(typ) = v.get("type").and_then(|x| x.as_str()) else {
            return;
        };
        match typ {
            "message_start" => {
                if let Some(m) = v.get("message") {
                    if let Some(mid) = m.get("model").and_then(|x| x.as_str()) {
                        self.model = mid.to_string();
                    }
                }
                if !self.role_sent {
                    self.role_sent = true;
                    self.pending.push_back(role_chunk(
                        &self.openai_id,
                        &self.model,
                        self.created,
                    ));
                }
            }
            "content_block_delta" => {
                if let Some(t) = anthropic_delta_text(v) {
                    if !self.role_sent {
                        self.role_sent = true;
                        self.pending.push_back(role_chunk(
                            &self.openai_id,
                            &self.model,
                            self.created,
                        ));
                    }
                    self.pending.push_back(content_chunk(
                        &self.openai_id,
                        &self.model,
                        self.created,
                        t,
                    ));
                }
            }
            "message_stop" => {
                self.enqueue_finish();
            }
            _ => {}
        }
    }

    fn process_line(&mut self, line: &[u8]) {
        if line.is_empty() {
            return;
        }
        let rest = if line.len() >= 5 && &line[..5] == b"data:" {
            trim_bytes(&line[5..])
        } else {
            return;
        };
        if rest == b"[DONE]" {
            self.enqueue_finish();
            return;
        }
        if let Ok(v) = serde_json::from_slice::<Value>(rest) {
            self.process_json_event(&v);
        }
    }

    fn process_buffer(&mut self) {
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let mut line = self.buf.split_to(pos + 1);
            if line.last() == Some(&b'\n') {
                line.truncate(line.len().saturating_sub(1));
            }
            if line.last() == Some(&b'\r') {
                line.truncate(line.len().saturating_sub(1));
            }
            self.process_line(&line[..]);
        }
    }
}

fn anthropic_delta_text(v: &Value) -> Option<&str> {
    let delta = v.get("delta")?;
    match delta.get("type").and_then(|t| t.as_str()) {
        Some("text_delta") => delta.get("text").and_then(|x| x.as_str()),
        _ => delta.get("text").and_then(|x| x.as_str()),
    }
}

fn trim_bytes(b: &[u8]) -> &[u8] {
    let mut s = b;
    while s.first().is_some_and(|x| x.is_ascii_whitespace()) {
        s = &s[1..];
    }
    while s.last().is_some_and(|x| x.is_ascii_whitespace()) {
        s = &s[..s.len() - 1];
    }
    s
}

fn role_chunk(id: &str, model: &str, created: u64) -> Bytes {
    let payload = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {"role": "assistant"},
            "finish_reason": null
        }]
    });
    let mut s = String::from("data: ");
    s.push_str(&payload.to_string());
    s.push_str("\n\n");
    Bytes::from(s)
}

fn content_chunk(id: &str, model: &str, created: u64, text: &str) -> Bytes {
    let payload = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {"content": text},
            "finish_reason": null
        }]
    });
    let mut s = String::from("data: ");
    s.push_str(&payload.to_string());
    s.push_str("\n\n");
    Bytes::from(s)
}

fn finish_chunk(id: &str, model: &str, created: u64) -> Bytes {
    let payload = json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": "stop"
        }]
    });
    let mut s = String::from("data: ");
    s.push_str(&payload.to_string());
    s.push_str("\n\n");
    Bytes::from(s)
}

impl Body for AnthropicToOpenAiSseBody {
    type Data = Bytes;
    type Error = hyper::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.as_mut().get_mut();

        if let Some(b) = this.pending.pop_front() {
            return Poll::Ready(Some(Ok(Frame::data(b))));
        }

        if this.finished {
            return Poll::Ready(None);
        }

        let polled = Pin::new(&mut this.inner).poll_frame(cx);
        match polled {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Some(Err(e))) => Poll::Ready(Some(Err(e))),
            Poll::Ready(None) => {
                if !this.buf.is_empty() {
                    let tail = std::mem::replace(&mut this.buf, BytesMut::new());
                    let mut bytes = tail.freeze();
                    if bytes.last() == Some(&b'\r') {
                        bytes = bytes.slice(..bytes.len().saturating_sub(1));
                    }
                    this.process_line(&bytes[..]);
                }
                this.process_buffer();
                if !this.finished {
                    this.enqueue_finish();
                }
                if let Some(b) = this.pending.pop_front() {
                    Poll::Ready(Some(Ok(Frame::data(b))))
                } else {
                    Poll::Ready(None)
                }
            }
            Poll::Ready(Some(Ok(frame))) => match frame.into_data() {
                Ok(mut data) => {
                    let data = data.copy_to_bytes(data.remaining());
                    this.buf.put_slice(&data);
                    this.process_buffer();
                    if let Some(b) = this.pending.pop_front() {
                        Poll::Ready(Some(Ok(Frame::data(b))))
                    } else {
                        Poll::Pending
                    }
                }
                Err(frame) => Poll::Ready(Some(Ok(frame))),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_anthropic_delta_to_openai_chunk_shape() {
        let v: Value = serde_json::from_str(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#,
        )
        .unwrap();
        let t = anthropic_delta_text(&v).unwrap();
        assert_eq!(t, "Hi");
    }
}
