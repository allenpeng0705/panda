# Panda API gateway + MCP gateway — design completion matrix

**Purpose:** Single place to see **which design artifacts match shipping code** and **what remains** (so [`implementation_plan_mcp_api_gateway.md`](./implementation_plan_mcp_api_gateway.md) and [`design_api_gateway_and_mcp_gateway.md`](./design_api_gateway_and_mcp_gateway.md) stay honest).

**Canonical flows:** [`panda_data_flow.md`](./panda_data_flow.md)

---

## 1. Phase map (implementation plan §4)

| Phase | Theme | In code today | Notes |
|-------|--------|----------------|--------|
| **A** | Config + no-op ingress when disabled | **Done** | `panda-config`: `ApiGatewayConfig` (`ingress`, `egress`); defaults off; validation. `api_gateway` module + `ProxyState.api_gateway`. |
| **B** | Egress client + allowlist + metrics | **Done** | `crates/panda-proxy/src/api_gateway/egress.rs`: pooled Hyper client, host/path allowlist, `corporate.default_base`, global + **named `profiles`** default headers, retries on **429 / 502 / 503 / 504**, `panda_egress_*` metrics. |
| **C** | Ingress path routing | **Done** | `api_gateway/ingress.rs`: longest-prefix routes, backends `ai` / `ops` / `deny` / `gone` / `not_found` / `mcp`, optional **HTTP methods** (405 + `Allow`), optional **`upstream` override** per route when `backend: ai`. Wired in `lib.rs` `dispatch`. |
| **D** | MCP ↔ egress + ingress MCP HTTP | **Done** | `http_tool` / `http_tools` → `EgressClient`. Ingress **`backend: mcp`:** JSON-RPC 2.0 over **POST** (`initialize`, `ping`, `tools/list`, `tools/call`, empty `resources/list` / `prompts/list`, notifications without `id`) in `inbound/mcp_http_ingress.rs`. **JSON-RPC arrays** = sequential **batch** (`handle_post_batch`). Tool names match **`mcp_{server}_{tool}`**. **`tools/call`** uses the same **`mcp.tool_routes`**, **`mcp.tool_cache`**, and **`mcp.hitl`** path as chat MCP follow-up (`lib.rs` `mcp_http_ingress_execute_tools_call` + context from `mcp_http_ingress_build_context`). **Streamable HTTP:** `Mcp-Session-Id`, GET SSE listener (keepalive), DELETE session, Origin check, SSE/JSON responses. **Still shallow vs full spec:** **`Last-Event-ID`** on GET listener is accepted but **not** used for replay; sessions are **in-memory** only (no cross-replica / durable resumption). |
| **E** | Control plane / dynamic routes | **E4 partial** | E3 plus optional **`control_plane.reload_pubsub`** (Redis **`PUBLISH`** / **`SUBSCRIBE`** to reload from store) and **`control_plane.additional_admin_secret_envs`** (extra secrets for `observability.admin_auth_header`, union with `admin_secret_env`). Postgres trigger for external SQL writers: [`runbooks/control_plane_postgres_external_writes.md`](./runbooks/control_plane_postgres_external_writes.md). Full **multi-tenant RBAC** still future. MySQL / **Azure SQL** → not implemented. |
| **F** | Hardening (mTLS egress, formal sec review) | **Partial (F1 done)** | SSRF allowlist shipped (Phase B). **Egress mTLS + corporate CA:** **`api_gateway.egress.tls`** (`client_cert_pem` + `client_key_pem`, optional **`extra_ca_pem`**) on the pooled egress client; covered by **`integration_https_mtls_presents_client_cert_to_upstream`** in `api_gateway/egress.rs`. **Still open:** Phase F2 (ingress SLO / load doc), F3 (formal security review). **TLS reload / min version** moved to **G4** (shipped). |
| **G** | Pools, ingress RL, TLS mgmt | **Partial** | **G1 — Egress upstream pools / RR:** **`api_gateway.egress.corporate.pool_bases`** + `panda_egress_requests_total{pool_slot=…}`. **G2 — Ingress + legacy RPS:** **`api_gateway.ingress.routes[].rate_limit`** and top-level **`routes[].rate_limit`** — 1s windows; optional **`api_gateway.ingress.rate_limit_redis`** (`url_env`, `key_prefix`) for **Redis `INCR` + short TTL** across replicas (`shared/route_rps.rs`, `dispatch` + `forward_to_upstream`). **G3 — Egress rate caps:** **`api_gateway.egress.rate_limit`** (`max_in_flight`, `max_rps`, process-local). **G4 — Egress TLS lifecycle:** **`min_protocol_version`** (`tls12` / `tls13`), Unix **`reload_on_sighup`**, optional **`watch_reload_ms`** (PEM mtime poll); client behind **`RwLock`** with **`reload_http_client`**. **Not shipped:** per-route Prometheus labels for ingress RL, custom **cipher** suites, cluster-wide **egress** RPS in Redis. |
| **H** | Developer portal (**one** surface) | **Partial (baseline shipped)** | **`/portal`** (HTML operator hub + embedded **`/portal/summary.json`**), **`/portal/summary.json`** (safe config/runtime snapshot), **`/portal/openapi.json`**, **`/portal/tools.json`**. **Goal:** help operators **manage** this Panda instance, not a full marketplace. **Optional later:** API keys (**H3**). |

---

## 2. Feature catalog vs code ([`panda_api_gateway_features.md`](./panda_api_gateway_features.md))

| Area | Shipped | Planned / partial |
|------|---------|-------------------|
| **Ingress TLS** | Reuse top-level **`tls`** on `listen` | Split listen, ACME, advanced cipher policy as in catalog |
| **Ingress routing** | Prefix → backend; method filters; AI upstream override; **`/portal/*`** on builtin ops prefix; **per-row `rate_limit.rps`** + optional **Redis** (`ingress.rate_limit_redis`) | Per-route auth; Prometheus labels per ingress RL row |
| **Ingress → MCP HTTP** | **POST** JSON-RPC on `backend: mcp` routes; **minimal streamable HTTP** (SSE `message` when `Accept: text/event-stream`) | Full MCP streamable session / resumption — future |
| **Egress HTTP** | Client, allowlist, base URL join, **`pool_bases` RR**, headers, profiles, retries, size cap, metrics (`pool_slot`), **`rate_limit`** (`max_in_flight`, `max_rps`, process-local), **TLS** (WebPKI + **`extra_ca_pem`**, optional **client cert mTLS**; **`min_protocol_version`**, **SIGHUP** + **`watch_reload_ms`** reload) | Custom **cipher** policy; cluster-wide egress throttling (**Redis**) if needed |
| **MCP gateway** | Stdio, OpenAI tools, rounds, policies, tool cache, `http_tool`, **remote `remote_mcp_url`** (JSON or **SSE** response), ingress MCP POST (+ **SSE wrap**), metrics, `/mcp/status` | Full MCP streamable session semantics |
| **Correlation** | `observability.correlation_header` through chat + egress tools | Full **route_id** label plumbing everywhere — partial |

---

## 3. Design doc maintenance

| Document | Role | When to update |
|----------|------|----------------|
| [`panda_data_flow.md`](./panda_data_flow.md) | Diagrams + checklist | New gateway role or listener split |
| [`design_api_gateway_and_mcp_gateway.md`](./design_api_gateway_and_mcp_gateway.md) | Internal pipelines, config *targets* | New module or breaking YAML |
| [`implementation_plan_mcp_api_gateway.md`](./implementation_plan_mcp_api_gateway.md) | Milestones | Phase completion or reprioritization |
| **This file** | Truth table vs `main` | After each gateway-related merge |

---

## 4. Suggested next engineering milestones (close remaining *design* gaps)

1. **MCP streamable HTTP (deeper):** session IDs, resumption, multi-chunk SSE — beyond one JSON-RPC envelope per POST.
2. **Ingress RL observability:** bounded **Prometheus** labels (or aggregates) for per-prefix limits; **per-identity** RPS/concurrency remains a design gap.
3. **Egress rate caps** — process-local **`max_in_flight` / `max_rps`** shipped; optional **cluster-wide** egress caps still a design item if required.
4. **Portal maintenance:** keep **`/portal`** + JSON exports truthful as the product grows; treat **API keys** / heavy doc portals as **optional** backlog, not a “full Phase H” gate.

Items **E** (control-plane product surface) and remaining **G** gaps (ciphers, cluster egress RL, ingress RL metrics) remain product-sized epics. **H** baseline is intentionally **minimal**—one portal for shipped features; **H3**-class work is separate.

---

## Related

- [`mcp_gateway_phase1.md`](./mcp_gateway_phase1.md) — onboarding YAML for MCP + `http_tool`  
- [`mcp_gateway_reference_designs.md`](./mcp_gateway_reference_designs.md) — external reference notes
