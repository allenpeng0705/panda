# Phase 1 — MCP gateway + API gateway (first step)

This document **narrows** what you need for the **first step**: a **focused MCP gateway** in Panda. The **product target** is **all-in-one**: **Panda’s own API gateway** (ingress in front of MCP and/or egress behind MCP toward corporate L7) — Phase 1 may not implement every gateway mode yet; see [`panda_data_flow.md`](./panda_data_flow.md) for the canonical flows.

---

## 1. Responsibility split

### 1.1 Panda API gateway (all-in-one) + optional external L7

**Target product:** Panda ships **its own API gateway** — **ingress** (in front of MCP: TLS, routing, auth into MCP/chat) and/or **egress** (behind MCP: outbound HTTP toward **corporate** API gateway + REST). Same **all-in-one** deployment; configure which roles are on. Canonical diagrams: [`panda_data_flow.md`](./panda_data_flow.md).

| Layer | Notes |
|--------|--------|
| **Panda API gateway — ingress** | Client-facing HTTP into MCP + chat. |
| **Panda MCP gateway** | Tools, rounds, policy (this doc’s Phase 1 focus). |
| **Panda API gateway — egress** | Tool and integration traffic toward corporate L7. |
| **External L7 (optional)** | Kong, NGINX, Envoy **outside** Panda — adds org-wide policy; use **`trusted_gateway`** when it wraps Panda ([`kong_handshake.md`](./kong_handshake.md), `trusted_gateway` in `panda.yaml`). |

| Concern | Who owns it |
|--------|-------------|
| **TLS / routing / auth into Panda** | **Panda API gateway (ingress)** and/or **external** L7 |
| **Trust hop to Panda** | When **external** L7 is **in front of** Panda, attested headers |
| **HTTP out to internal REST** | **Panda API gateway (egress)** → **corporate** API gateway |

Full design: [`design_mcp_control_plane_rust.md`](./design_mcp_control_plane_rust.md) §4.

### 1.2 Panda — MCP gateway (Phase 1 scope)

Panda **`mcp.*`** + OpenAI-shaped chat integration: connect tool servers, expose tools to the model, execute tool calls with guardrails.

**Phase 1 — include (use these first):**

| Capability | Config / behavior |
|------------|-------------------|
| **Enable MCP** | `mcp.enabled: true` with at least one `mcp.servers[]` entry (`enabled: true`) |
| **Transport** | **Stdio** MCP servers (`command` + `args`); optional **`http_tool`** / **`http_tools`** (declarative REST via **`api_gateway.egress`**) — [§3](#3-declarative-rest-tool-via-egress-http_tool); optional **`remote_mcp_url`** (full `https://…` to a remote MCP server speaking JSON-RPC POST — same wire shape as Panda ingress MCP; also via **`api_gateway.egress`** + allowlist); servers with none of these remain stubs (tests only) |
| **Tool discovery** | Aggregated tool list for the model (`mcp.advertise_tools: true` when you want tools in chat) |
| **Execution limits** | `tool_timeout_ms`, `max_tool_payload_bytes`, `max_tool_rounds` |
| **Failure behavior** | `fail_open` (model continues vs hard error) |
| **Observability** | `GET /mcp/status`; use defaults for stream probe fields unless you have a measured need |

**Phase 1 — defer for *onboarding* (features exist in code; turn on when ready):**

| Feature | Why defer in docs |
|---------|-------------------|
| **`mcp.tool_routes`** | Pattern-based allow/deny per tool name — operational rules |
| **`mcp.intent_tool_policies`** + **`proof_of_intent_mode`** | Intent gating — policy-heavy |
| **`mcp.tool_cache`** | Result caching — allowlists + compliance |
| **`mcp.hitl`** | Human-in-the-loop — runbooks |
| **`agent_sessions`** | Session/profile routing and MCP round caps — fleet concerns |
| **Per-route `mcp_servers` on `routes`** | Fine-grained MCP servers per HTTP route |

**Ingress MCP HTTP:** With **`api_gateway.ingress.enabled`**, routes whose **`backend` is `mcp`** accept **POST** JSON-RPC (`initialize`, `tools/list`, `tools/call`, …) on that prefix (e.g. **`/mcp`** in the default ingress table). Tool names are the same as in chat (`mcp_{server}_{tool}`). **`tools/call`** honors **`mcp.tool_routes`**, **`mcp.tool_cache`** (same scope/metrics/compliance as chat — see [`tool_cache_mvp.md`](./tool_cache_mvp.md)), and **`mcp.hitl`**. **`mcp.intent_tool_policies`** / proof-of-intent apply to the **chat** tool loop only, not to raw ingress `tools/call`. **Streamable HTTP** (SSE) is not implemented yet.

---

## 2. Minimal `panda.yaml` skeleton (Phase 1 MCP)

Use a real stdio server (see `examples/mcp_stdio_minimal/`). Adjust `command` / `args` for your environment.

```yaml
listen: "127.0.0.1:8080"
upstream: "https://api.openai.com"   # or your OpenAI-compatible LLM

mcp:
  enabled: true
  advertise_tools: true
  fail_open: true
  tool_timeout_ms: 30000
  max_tool_payload_bytes: 1048576
  max_tool_rounds: 4
  servers:
    - name: demo
      enabled: true
      command: "python3"
      args: ["examples/mcp_stdio_minimal/server.py"]

# Optional but common with an API gateway in front:
# trusted_gateway:
#   attestation_header: "X-Panda-Internal"
#   subject_header: "X-User-Id"
# Set PANDA_TRUSTED_GATEWAY_SECRET to match the gateway.
```

Keep **`stream_probe_bytes`** and **`probe_window_seconds`** at defaults unless debugging streaming + tools.

---

## 3. Declarative REST tool via egress (`http_tool`)

Some integrations are a **single corporate HTTP endpoint** (GET or JSON body). Instead of a stdio MCP server, you can declare one tool per `mcp.servers[]` entry using **`http_tool`**. Panda executes it through the **same [`api_gateway.egress`](./panda_data_flow.md) client** as other outbound corporate calls (allowlist, timeouts, metrics).

**Requirements (validated in config):**

- **`api_gateway.egress.enabled: true`** with a non-empty **`corporate.default_base`** (the `http_tool.path` is appended to this base URL).
- **`allowlist.allow_hosts`** and **`allowlist.allow_path_prefixes`** must permit the resolved URL (same rules as standalone egress).
- **`command` and `http_tool` are mutually exclusive** (do not set a non-empty `command` on the same server).

**Fields:**

| Field | Notes |
|--------|--------|
| **`path`** | Required. Must start with `/`. Resolved as `{default_base}{path}`. |
| **`method`** | Default `GET`. For **`POST`**, **`PUT`**, **`PATCH`**, the tool **`arguments`** object is sent as **JSON** with `Content-Type: application/json`. |
| **`tool_name`** | Default `call`. Model-facing OpenAI function name: `mcp_{server}_{tool_name}` (sanitized like other MCP servers). |
| **`description`** | Optional; shown in tool metadata. |
| **`egress_profile`** | Optional. Must match a **`api_gateway.egress.profiles[].name`**; merges that profile’s `default_headers` after global egress headers (requires **`egress.enabled: true`**). |

**Observability:** Panda sets the configured **`observability.correlation_header`** on the egress request from the active tool-call correlation id when present.

**Metrics:** Egress requests from this path are labeled with a fixed low-cardinality route label **`mcp_http_tool`** (see `panda_egress_requests_total`).

Use **`http_tools`** (a list) on one server for **multiple** REST tools; each entry has the same shape as `http_tool`, with **unique `tool_name`** values. You cannot set both `http_tool` and `http_tools` on the same server.

---

## 3b. Remote MCP server over HTTP (`remote_mcp_url`)

Use this when another service already exposes **MCP JSON-RPC over HTTP POST** (same methods as Panda’s ingress **`backend: mcp`** path: `initialize`, `tools/list`, `tools/call`, …).

**Requirements:**

- **`api_gateway.egress.enabled: true`** with **`allowlist.allow_hosts`** (and path prefixes) that cover the remote URL’s host and path.
- **`remote_mcp_url`:** absolute `http://` or `https://` URL (e.g. `https://mcp.internal.example/sse` or `/mcp` path — whatever that server expects for POST bodies).
- **Mutually exclusive** with `command`, `http_tool`, and `http_tools` on the same server.
- Optional **`remote_mcp_egress_profile`:** same as `http_tool.egress_profile` (named headers on egress).

Aggregated tool names for the model remain **`mcp_{server}_{tool}`** where `{tool}` is the **remote** server’s tool name.

---

**Example (excerpt):** enable egress, then add a server with `http_tool` or `http_tools` (see also `panda.example.yaml`).

```yaml
api_gateway:
  egress:
    enabled: true
    timeout_ms: 30000
    corporate:
      default_base: "https://api.internal.example.com"
    allowlist:
      allow_hosts: ["api.internal.example.com"]
      allow_path_prefixes: ["/v1/internal/"]

mcp:
  enabled: true
  advertise_tools: true
  servers:
    - name: corp_lookup
      enabled: true
      http_tool:
        path: /v1/internal/lookup
        method: POST
        tool_name: run_query
        description: "Corporate lookup API"
```

**Remote MCP (excerpt):** allowlist must include the remote host (and path prefix `/` if needed).

```yaml
api_gateway:
  egress:
    enabled: true
    corporate:
      default_base: "https://mcp.internal.example"
    allowlist:
      allow_hosts: ["mcp.internal.example"]
      allow_path_prefixes: ["/"]
mcp:
  enabled: true
  servers:
    - name: upstream_mcp
      enabled: true
      remote_mcp_url: "https://mcp.internal.example/mcp"
```

---

## 4. What Phase 1 does *not* cover

- **Outbound AI gateway** at full strength (TPM, semantic cache, model failover, semantic routing) is a **parallel track**. You can run Phase 1 MCP with a single `upstream` and add budgets/cache later — see [`architecture_two_pillars.md`](./architecture_two_pillars.md).
- **Wasm plugins, budget hierarchy, console OIDc** — enterprise or later hardening, not Phase 1 MCP prerequisites.

---

## 5. Advanced MCP (after Phase 1)

When basics are stable, enable in order that matches your risk:

1. **`tool_routes`** — explicit tool allow/deny by pattern  
2. **`intent_tool_policies`** / **proof_of_intent** — tighter tool exposure  
3. **`tool_cache`** — latency/cost (see [`tool_cache_mvp.md`](./tool_cache_mvp.md))  
4. **`hitl`** — high-risk tools  
5. **`agent_sessions`** — fleet routing and caps ([`ai_routing_strategy.md`](./ai_routing_strategy.md))

---

## 6. Related docs

- [`gateway_design_completion.md`](./gateway_design_completion.md) — MCP + API gateway phases vs shipping code  
- [`panda_data_flow.md`](./panda_data_flow.md) — canonical Agent → gateway(s) → Panda → REST flows  
- [`design_api_gateway_and_mcp_gateway.md`](./design_api_gateway_and_mcp_gateway.md) — detailed design: MCP + API gateway pipelines and config  
- [`implementation_plan_mcp_api_gateway.md`](./implementation_plan_mcp_api_gateway.md) — build plan for Panda API gateway + MCP  
- [`design_mcp_control_plane_rust.md`](./design_mcp_control_plane_rust.md) — Panda target: control plane + data plane in Rust, easy deploy  
- [`mcp_gateway_reference_designs.md`](./mcp_gateway_reference_designs.md) — design notes from Docker & Microsoft MCP gateway projects  
- [`architecture_two_pillars.md`](./architecture_two_pillars.md) — full two-pillar map  
- [`protocol_evolution.md`](./protocol_evolution.md) — MCP vs future agent protocols  
- [`kong_handshake.md`](./kong_handshake.md) — API gateway → Panda trust contract  
