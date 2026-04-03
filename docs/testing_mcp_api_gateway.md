# Testing the MCP gateway with the API gateway

This doc describes **how to test** Panda when **`api_gateway` ingress/egress** and **`mcp`** are used together, and points at the **framework the repo already ships** (Rust integration tests + optional shell smoke).

---

## 1. What you are testing

| Layer | What to verify |
|--------|----------------|
| **Ingress** | Prefix → `backend: mcp` routes accept **POST JSON-RPC** (`initialize`, `tools/list`, `tools/call`); optional **RPS** / **Redis** counters; **portal** paths when ingress is on. |
| **MCP host** | Tool catalog, `tools/call` execution, **`http_tool`** and allowlist, **`mcp.tool_cache`** / **`mcp.tool_routes`** where configured. |
| **Egress** | Resolved URL is allowlisted; retries / mTLS / metrics on the egress client (see egress tests). |

End-to-end shape: **HTTP client → Panda (ingress) → MCP → egress → mock corporate HTTP**.

---

## 2. In-repo “framework”: Rust integration tests (primary)

The strongest coverage is **`cargo test -p panda-proxy`**. Tests spin up a **local `TcpListener`**, build **`ProxyState`** from **`PandaConfig::from_yaml_str`**, and drive **`dispatch`**—no separate binary required.

**Examples (full stack ingress + MCP + egress):**

- **`ingress_mcp_http_initialize_and_tools_list`** — `initialize` + `tools/list` on **`POST /mcp`** (streamable `Accept` + session id).
- **`ingress_mcp_http_tools_call_uses_tool_cache_second_hit`** — **`http_tool`** calls a **mock upstream** on an ephemeral port; **`tools/call`** over ingress; asserts **tool cache** prevents a second backend hit.
- **`ingress_mcp_initialize_accepts_streamable_sse`** / **`ingress_mcp_streamable_get_listener_and_delete_session`** — streamable HTTP surface.

**Ingress routing (unit-level):** `crates/panda-proxy/src/api_gateway/ingress.rs` (`#[test]` on classify / merge).

**Egress client:** `crates/panda-proxy/src/api_gateway/egress.rs` (`integration_hits_mock_upstream_when_allowed`, **`integration_https_mtls_presents_client_cert_to_upstream`**, pool / retry cases).

**How to run a focused subset:**

```bash
# All tests whose names mention ingress MCP
cargo test -p panda-proxy ingress_mcp -- --nocapture

# Ingress router unit tests
cargo test -p panda-proxy api_gateway::ingress:: -- --nocapture

# Egress integration-style tests (names vary)
cargo test -p panda-proxy api_gateway::egress:: -- --nocapture
```

**Convenience script (same filters):** [`../scripts/gateway_mcp_integration_tests.sh`](../scripts/gateway_mcp_integration_tests.sh)

**Pattern for new tests:** Copy **`ingress_mcp_http_tools_call_uses_tool_cache_second_hit`** in `crates/panda-proxy/src/lib.rs`: spawn a tiny TCP mock that speaks HTTP/1.1, point **`api_gateway.egress.corporate.default_base`** at it, set **`allowlist`**, add **`mcp.servers[]`** with **`http_tool`**, then issue raw HTTP to **`POST /mcp`** with JSON-RPC bodies.

---

## 3. Mock MCP over stdio (host process tests)

- **`crates/panda-proxy/tests/mcp_mock_stdio.py`** — minimal JSON-RPC stdio MCP ( **`ping`** tool ) for harnesses that spawn a subprocess.
- **`examples/mcp_stdio_minimal/`** — richer sample (`ping`, `echo_message`) for manual runs; see its README.

Wire with **`mcp.servers[].command` + `args`** pointing at the script path.

---

## 4. Live binary smoke (optional)

For a **real `panda-server` process** talking to a **mock corporate API**:

1. See **`examples/gateway_mcp_e2e/README.md`** — fixed or scripted ports, **`mock_corp_api.py`**, generated YAML.
2. Run **`scripts/gateway_mcp_e2e_smoke.sh`** — starts mock + Panda, runs **`curl`** JSON-RPC **`initialize` → `tools/call`**, tears down.

Use this when you care about **CLI startup**, **logging**, or **TLS on `listen`**, which the embedded `dispatch` tests do not exercise.

---

## 5. Staging / CI against a running deployment

[`scripts/staging_readiness_gate.sh`](../scripts/staging_readiness_gate.sh) runs **`cargo test -p panda-config`** and **`cargo test -p panda-proxy`**, then **`curl`** **`/health`**, **`/ready`**, **`/mcp/status`**. Point **`PANDA_BASE_URL`** at your instance; optional load with **`READINESS_LOAD_PAYLOAD`**.

---

## 6. Suggested workflow for a change

1. **`cargo test -p panda-config`** if you touched YAML structs / validation.  
2. **`./scripts/gateway_mcp_integration_tests.sh`** (or full **`cargo test -p panda-proxy`**).  
3. If you changed **process** behavior: **`./scripts/gateway_mcp_e2e_smoke.sh`**.  
4. Before release: **`./scripts/staging_readiness_gate.sh`** against a staging URL.

---

## Related

- [`panda_data_flow.md`](./panda_data_flow.md) — ingress / MCP / egress positions  
- [`mcp_gateway_phase1.md`](./mcp_gateway_phase1.md) — minimal MCP + **`http_tool`** config  
- [`gateway_design_completion.md`](./gateway_design_completion.md) — what is implemented vs backlog  
