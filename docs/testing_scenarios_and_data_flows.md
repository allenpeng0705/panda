# Testing scenarios and data flows

**Purpose:** Map **what each major automated test exercises** and the **request path** (data flow) through Panda. Use this with the canonical dispatch description in [`panda_data_flow.md`](./panda_data_flow.md) (order: control plane → ingress → console → ops/health → JWT → `forward_to_upstream`).

**Legend — typical flow shorthand**

| Symbol | Meaning |
|--------|---------|
| `TCP` | Raw client to `listen` (often Hyper `dispatch` in tests) |
| `CP` | Control plane (`control_plane.enabled` + `path_prefix`) |
| `ING` | API gateway ingress (`IngressRouter` classification) |
| `MCP↑` | MCP HTTP ingress (`backend: mcp`, JSON-RPC on `/mcp`…) |
| `FWD` | `forward_to_upstream` (proxy / chat / MCP-on-chat path) |
| `EG` | Egress client to corporate HTTP |

Tests that only parse YAML or call pure helpers have **no network path** (noted explicitly).

---

## Config parsing — `crates/panda-config/tests/scenario_profiles.rs`

Aligned with [`panda_scenarios_summary.md`](./panda_scenarios_summary.md). **No proxy** — validates `PandaConfig::from_yaml_str` and `effective_*` helpers.

| Test | What is verified | “Flow” |
|------|------------------|--------|
| `scenario_a_ai_gateway_only_no_mcp` | AI route + `mcp.enabled: false` | Parse → `effective_backend_base` for `/v1/chat` |
| `scenario_b_mcp_and_egress_no_routes` | Ingress + egress + HTTP tool server | Parse + structural flags |
| `scenario_c_routes_mcp_advertise_global_false_route_true` | Route-level `mcp_advertise_tools` overrides global | Parse → `effective_mcp_advertise_tools_for_path` |
| `scenario_d_repo_root_panda_yaml_parses` | Repo root [`panda.yaml`](../panda.yaml) parses; observability, MCP servers, effective backends | Parse + assertions on representative fields |
| `scenario_e_longest_prefix_embeddings_uses_default_backend` | Longest-prefix routing for chat vs embeddings | Parse → `effective_backend_base` / `effective_adapter_provider` |
| `scenario_f_per_route_adapter_type_overrides_global` | Per-route `type: anthropic` | Parse → `effective_adapter_provider` |

---

## End-to-end workflows — `crates/panda-proxy/src/tests/gateway_workflow.rs`

Hyper serves `dispatch`; ingress + MCP runtime + egress wired like production.

| Test | What is verified | Data flow |
|------|------------------|-----------|
| `workflow_init_only_http_tools_config_reaches_200` | Minimal MCP config: `initialize` succeeds | `TCP → ING → MCP↑` → JSON-RPC 200 (no tool execution) |
| `workflow_full_stack_two_http_tools_two_mock_paths` | Two `http_tools`, mock upstream on dynamic port | `TCP → ING → MCP↑` (`initialize`); then `tools/call` ×2 → **`McpRuntime` → EG** → mock GET `/corp/...` → tool result in JSON-RPC body |
| `workflow_ingress_remote_mcp_tools_call_via_egress` | `remote_mcp_url` server | `TCP → ING → MCP↑` → **`tools/call`** → Panda calls remote MCP over HTTP **via egress** → mock JSON-RPC returns tool payload |
| `workflow_stdio_python_and_http_tool_ingress` | Stdio MCP (`mcp_mock_stdio.py`) + `http_tool` (skipped if no Python) | `TCP → ING → MCP↑` → stdio tool then HTTP tool → **EG** → REST mock |
| `workflow_ingress_off_post_mcp_not_handled_by_mcp_ingress` | Ingress disabled: `/mcp` is **not** MCP handler | `TCP → (no ING classify)` → **`FWD`** to `default_backend` → upstream missing → **502** |
| `workflow_mcp_runtime_off_ingress_mcp_returns_unavailable` | Ingress on but `state.mcp = None` | `TCP → ING → MCP↑` handler sees no runtime → **503** JSON-RPC |
| `workflow_http_tool_requires_egress_enabled` | Config validation | Parse error if `http_tool` without `egress.enabled` — **no TCP** |

---

## Control plane, tenant ingress, streamable SSE — `crates/panda-proxy/src/tests/control_plane_and_streamable_scenarios.rs`

Uses nested `tokio::spawn` per accept so long-lived SSE GET does not block. Matrix IDs are documented in the module header.

| ID / test | What is verified | Data flow |
|-----------|------------------|-----------|
| **CP-RO-1 / CP-RO-2 / CP-RW-1** — `control_plane_read_only_matrix_and_write_still_mutates` | Read-only env vs write secret for control plane REST | `TCP → CP` — GETs with RO secret **200**; mutating POST/DELETE with RO **403**; write secret **200** on POST |
| **TN-1 … TN-4** — `ingress_tenant_global_row_vs_scoped_row` | Dynamic ingress rows with/without `tenant_id` + `tenant_resolution_header` | `TCP → CP` (POST routes) then `TCP → ING` — scoped path/header → **410** vs **404** per matrix |
| **SSE-1** — `ingress_mcp_streamable_last_event_id_replays_only_newer_events` | Streamable MCP: `Last-Event-ID` replays only newer SSE events | `TCP → ING → MCP↑` (initialize, ping) → **`GET` listener** with `Last-Event-ID` → ring buffer replays id 2 only before `: mcp-listener` |
| `repo_root_panda_yaml_dispatch_health_smoke` | Repo [`panda.yaml`](../panda.yaml) works with **`IngressRouter::try_new` + `dispatch`** | Load same YAML as `scenario_d` → **`GET /health`** → **200** (not blocked by ingress misconfig) |

---

## Dispatch fall-through — `crates/panda-proxy/src/tests/dispatch_branches.rs`

| Test | What is verified | Data flow |
|------|------------------|-----------|
| `when_control_plane_disabled_ops_prefix_is_not_control_plane_json` | CP off: `/ops/...` is not CP JSON | `TCP →` skip CP → later handling (not CP 404 semantics for that prefix) |
| `when_control_plane_and_ingress_disabled_request_hits_default_backend` | No ingress: request proxied | `TCP →` skip ING → **`FWD`** → mock upstream |
| `when_control_plane_enabled_status_without_secret_is_401` | CP auth | `TCP → CP` → **401** without secret |

---

## Backend routing and proxy — `crates/panda-proxy/src/tests/backend_routing_and_proxy.rs`

| Test | What is verified | Data flow |
|------|------------------|-----------|
| `forward_get_uses_default_backend_and_preserves_full_path` | Default backend + path | `TCP → FWD` → mock upstream (path preserved) |
| `forward_longest_prefix_route_picks_backend_base` | Longest `path_prefix` wins | `TCP → FWD` → correct backend base |
| `forward_post_non_chat_json_not_rewritten_to_anthropic` | Non-chat POST not adapted as Anthropic | `TCP → FWD` → upstream body unchanged |
| `forward_post_chat_openai_preserves_path_and_openai_json` | OpenAI chat path | `TCP → FWD` → upstream |
| `forward_post_chat_anthropic_rewrites_to_messages_path` | Anthropic adapter rewrites URL | `TCP → FWD` → `/v1/messages` on upstream |
| `forward_get_chat_path_not_anthropic_even_when_provider_anthropic` | GET not treated as chat POST adapter | `TCP → FWD` |

---

## `lib.rs` integration tests (`crates/panda-proxy/src/lib.rs` — `#[cfg(test)]`)

Grouped by concern. All use **`test_proxy_state`** + **`dispatch`** unless noted.

### Control plane + dynamic ingress

| Test | What is verified | Data flow |
|------|------------------|-----------|
| `control_plane_dynamic_ingress_post_then_classify_merged` | POST dynamic route then traffic sees merged router | `TCP → CP` (mutate routes) → `TCP → ING` |

### Ingress MCP (HTTP / streamable / portal)

| Test | What is verified | Data flow |
|------|------------------|-----------|
| `ingress_mcp_http_initialize_and_tools_list` | JSON-RPC init + `tools/list` | `TCP → ING → MCP↑` |
| `ingress_mcp_initialize_accepts_streamable_sse` | Negotiates streamable accept | `TCP → ING → MCP↑` |
| `ingress_mcp_streamable_get_listener_and_delete_session` | SSE GET listener + session delete | `TCP → ING → MCP↑` + long-lived GET |
| `ingress_mcp_http_tools_call_uses_tool_cache_second_hit` | Tool cache hit on second call | `TCP → ING → MCP↑` → **EG** (or cache) |
| `portal_openapi_and_tools_json_with_ingress` | Portal OpenAPI + tools catalog | `TCP →` portal/ops paths under `dispatch` |

### JWT, ops auth, status JSON

| Test | What is verified | Data flow |
|------|------------------|-----------|
| `merge_jwt_identity_*` (3 tests) | JWT claims merged into identity context | `dispatch` path with JWT middleware |
| `jwt_validation_*` (4 tests) | require JWT, scope, route scope | `TCP →` JWT gate → **401** or continue |
| `token_exchange_mints_agent_token` | Agent token exchange | `TCP →` token endpoint |
| `ops_auth_guard_enforces_shared_secret` | Ops header guard | Unit / small request |
| `console_http_requires_ops_secret_when_configured` | Console requires ops secret | `TCP →` console |
| `compliance_status_requires_ops_secret_when_configured` | Compliance endpoint | `TCP →` |
| `control_plane_rest_path_respects_prefix` | CP path prefix | **Unit** (string / path helper) |
| `control_plane_status_requires_ops_secret_when_configured` | CP status auth | `TCP → CP` |
| `control_plane_accepts_additional_admin_secret_without_ops_secret` | Alternate admin secret | `TCP → CP` |

### TPM / budgets / fleet / MCP status

| Test | What is verified | Data flow |
|------|------------------|-----------|
| `tpm_status_json_reports_budget_fields` | TPM JSON shape | `GET` ops/metrics path |
| `tpm_status_json_includes_agent_context_when_agent_sessions_enabled` | Agent session in TPM status | `TCP →` |
| `tpm_status_json_bucket_reflects_jwt_sub` | Bucket key from JWT sub | `TCP →` with JWT |
| `mcp_status_json_reports_config` | MCP status | `GET` |
| `mcp_status_json_semantic_cache_effective_bucket_scoping_from_agent_sessions` | Semantic cache bucket in status | `GET` |
| `fleet_status_json_includes_core_sections` | Fleet status | `GET` |
| `readiness_status_*` (3 tests) | `/ready` ok / MCP required / draining | `TCP →` readiness |
| `ops_auth_metrics_render_prometheus_lines` | Metrics text | **String / unit** |

### Upstream / shutdown / MCP chat loops

| Test | What is verified | Data flow |
|------|------------------|-----------|
| `upstream_request_timeout_fails_when_backend_hangs` | Upstream timeout | `TCP → FWD` → hanging mock |
| `wait_for_active_connections_*` (2 tests) | Drain behavior | Async wait helpers + mock |
| `mcp_followup_stops_at_max_rounds` | Tool loop cap | `FWD` + MCP follow-up |
| `mcp_followup_converges_after_multiple_rounds` | Multi-round tool loop | `FWD` |
| `mcp_streaming_tool_loop_returns_sse_without_downgrade_header` | SSE streaming tool path | `FWD` + SSE |

### Pure helpers (no HTTP)

| Test | What is verified |
|------|------------------|
| `console_mcp_args_preview_redacts_sensitive_keys` | Redaction |
| `config_roundtrip_for_listener` | Config serialization |
| `tpm_bucket_key_*`, `tpm_bucket_class_formats`, `tpm_token_estimate_prefers_larger_of_hints` | TPM key/token helpers |
| `effective_mcp_max_tool_rounds_respects_session_and_profile_rules` | Round limits |
| `profile_upstream_longest_path_prefix_wins` | Profile routing helper |
| `deny_pattern_match_is_case_insensitive` | Policy match |
| `pii_scrubber_redacts_matches` | PII scrub |
| `inject_openai_tools_*` | Tool injection |
| `context_enrichment_injects_system_message_from_env_file` | Context enricher |
| `openai_chat_json_to_sse_contains_done` | SSE conversion |
| `semantic_cache_key_*` (2 tests) | Cache key material |
| `extract_tool_calls_from_streaming_sse_*` (2 tests) | Stream parsing |

---

## Other crates (spot index)

| Location | Role |
|----------|------|
| `crates/panda-proxy/src/api_gateway/ingress.rs` | Unit tests: ingress table matching, methods, defaults |
| `crates/panda-proxy/src/api_gateway/egress.rs` | Egress URL resolution, allowlist, retries (see tests in file) |
| `crates/panda-proxy/src/api_gateway/control_plane_store.rs` | Store async tests |

---

## Related docs

- [`testing_mcp_api_gateway.md`](./testing_mcp_api_gateway.md) — how to run tests, mock contracts, tool naming on ingress.
- [`panda_data_flow.md`](./panda_data_flow.md) — dispatch order and ingress vs `forward_to_upstream`.
- [`panda_scenarios_summary.md`](./panda_scenarios_summary.md) — scenario matrix (product view).

When you add a **new** integration test, add one row to the matching table here (or a short subsection) so the catalog stays truthful.
