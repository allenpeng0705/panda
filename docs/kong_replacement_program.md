# Panda Kong-Replacement Program

This document is the execution tracker for the long-term goal: **Panda replaces Kong for AI traffic first, then for broader gateway needs**.

It is intentionally practical: every item has a status and an owner cadence so we always know where we are.

---

## 1) North Star

Panda becomes:

- **AI-first data plane** with better streaming, policy clarity, and MCP governance than Kong AI Gateway.
- **Simple and robust by default** for small teams.
- **Default gateway for small agents** (for example OpenClaw-class agents) with built-in token control, caching, and guardrails.
- **Enterprise and huge-company ready** through optional controls (SSO, budget hierarchy, policy, audit, reliability).
- **Control-plane capable** (multi-tenant config, RBAC, rollout, policy catalog, billing hooks) without bloating the hot path.

---

## 2) Program Principles

- **Security first:** fail-closed defaults for sensitive paths; explicit trust boundaries.
- **AI first:** optimize for model/tool/MCP flows before generic API feature breadth.
- **Simple first:** one binary + one YAML path for fast onboarding.
- **Reliability first:** documented failure semantics over "magic retries".
- **Compatibility first:** coexist with Kong during migration; no forced big-bang cutover.

---

## 3) Status Legend

- `DONE` - implemented, tested, documented.
- `IN_PROGRESS` - active implementation with partial coverage.
- `NEXT` - prioritized upcoming work.
- `PLANNED` - valid backlog, not yet scheduled.
- `BLOCKED` - waiting on decision/dependency.

---

## 4) Program Dashboard (Single Source of Truth)

## Track A - AI Data Plane Superiority (Beat Kong on AI path)

| Item | Status | Notes / Exit Criteria |
|------|--------|-----------------------|
| OpenAI-compatible ingress + SSE streaming hot path | `DONE` | Baseline in production path. |
| Semantic cache + semantic routing foundations | `DONE` | Present in data plane and docs. |
| MCP host and streaming tool-followup loop | `DONE` | `/mcp/status` plus intent-policy hints available. |
| Agent sessions/profile routing and limits | `DONE` | JWT/header-derived session/profile, effective tool-round reporting. |
| Model failover chain + circuit breaker (pre-response retries) | `DONE` | Includes provider protocol mapping and readiness visibility. |
| Streaming failover semantics + optional buffered mid-stream SSE | `DONE` | `/ready` + `panda_model_failover_midstream_retry_total`; `allow_failover_after_first_byte` buffers OpenAI chat SSE up to `midstream_sse_max_buffer_bytes` (excludes MCP streaming follow-up + Anthropic adapter streaming). |
| Mid-stream replay failover (seamless splice / multi-hop stream) | `PLANNED` | Current path is full-buffer replay, not chunk-level splice or cross-format merge. |
| Rich AI policy packs (prompt/tool/intent guardrails) | `IN_PROGRESS` | Prompt safety and policy gates exist; expand policy packs over time. |

## Track B - Governance, Security, and Compliance

| Item | Status | Notes / Exit Criteria |
|------|--------|-----------------------|
| JWT auth + trusted gateway handshake | `DONE` | Identity merge and anti-spoofing boundary implemented. |
| Ops endpoint auth + telemetry | `DONE` | Protected status endpoints and metrics counters available. |
| PII scrubber and deny-pattern prompt safety | `DONE` | Starter controls implemented. |
| Compliance JSONL exports (ingress/egress) | `DONE` | Includes optional hierarchy nodes. |
| Hierarchical prompt budgets (org/dept) | `DONE` | Redis-backed enforcement implemented. |
| USD hierarchy accounting (window estimates) | `DONE` | `budget_hierarchy.usd_per_million_prompt_tokens` + `/tpm/status` current-window estimates. |
| Enterprise SSO console roles (OIDC mode any/all) | `DONE` | Role mode validation and enforcement implemented. |
| Signed Wasm plugin supply-chain enforcement | `PLANNED` | Digest allowlists/signature workflow pending. |

## Track C - Reliability, SLO, and Observability

| Item | Status | Notes / Exit Criteria |
|------|--------|-----------------------|
| `/health` + `/ready` operational checks | `DONE` | Includes drain and active-connection visibility. |
| Structured request logs + OTLP traces | `DONE` | Includes OTel-friendly HTTP span attributes on request completion. |
| SLO runbook and readiness gate scripts | `DONE` | Runbooks and scripts committed. |
| OTLP metrics for AI dimensions (TTFT/token/error class) | `NEXT` | Trace fields improved; dedicated metrics pipeline still needed. |
| Long-soak and chaos evidence package | `PLANNED` | Formal recurring test artifacts pending. |

## Track D - Protocol & Platform Completeness

| Item | Status | Notes / Exit Criteria |
|------|--------|-----------------------|
| HTTP primary gateway path | `DONE` | Core production path. |
| Developer portal (single `/portal` surface) | `DONE` | **Scope:** one portal so operators **manage** Panda easily — **`/portal`** (HTML hub + embedded **`/portal/summary.json`**), **`/portal/summary.json`** (no-secret snapshot: MCP, gateway, control plane, flags), **`/portal/openapi.json`**, **`/portal/tools.json`**. **Not a goal:** Kong-style marketplace; API keys stay **optional** backlog ([`implementation_plan_mcp_api_gateway.md`](./implementation_plan_mcp_api_gateway.md) Phase **H**). |
| gRPC health endpoint (feature-gated) | `DONE` | `panda-server --features grpc` + `PANDA_GRPC_HEALTH_LISTEN`. |
| gRPC AI ingress/egress APIs | `NEXT` | Roadmap documented; implementation not started. |
| Expanded provider protocol adapters (beyond Anthropic) | `PLANNED` | Prioritize by customer demand and parity ROI. |

## Track E - Coexistence -> Replacement -> Control Plane

| Item | Status | Notes / Exit Criteria |
|------|--------|-----------------------|
| Phase 1 coexistence with Kong | `DONE` | Integration path and docs established. |
| Phase 2 AI-first edge split-host migration | `IN_PROGRESS` | Technical base ready; needs more migration playbooks. |
| Phase 3 optional consolidation (Kong retirement by scope) | `NEXT` | Requires parity scorecard + migration tooling. |
| Control-plane MVP (tenants, RBAC, config versions, rollout) | `IN_PROGRESS` | **Gateway slice shipped:** dynamic API ingress store + multi-replica sync (poll / Postgres `NOTIFY` / optional Redis pub/sub) + extra admin secrets for automation; see [`implementation_plan_mcp_api_gateway.md`](./implementation_plan_mcp_api_gateway.md) Phase **E**, [`runbooks/control_plane_postgres_external_writes.md`](./runbooks/control_plane_postgres_external_writes.md). **Still NEXT for product MVP:** tenant isolation, versioned config, rollout orchestration, full RBAC/OIDC on control APIs. |
| Enterprise control-plane (policy catalog, metering/billing hooks, fleet ops) | `PLANNED` | Build after control-plane MVP stabilizes. |

## Track F - Agent-First Adoption (OpenClaw-Class Differentiator)

| Item | Status | Notes / Exit Criteria |
|------|--------|-----------------------|
| Agent quickstart profile (`panda` for small agents in <10 minutes) | `DONE` | Published guide: [`openclaw_agent_quickstart.md`](./openclaw_agent_quickstart.md). |
| Token control for agents (live status + per-session/profile budgets) | `IN_PROGRESS` | `/tpm/status` + agent session/profile controls exist; `/mcp/status` now includes `agent_governance` + Prometheus `panda_mcp_agent_*` counters (max rounds, intent filter/deny). |
| Intent-aware caching policy (safe semantic cache defaults) | `IN_PROGRESS` | Semantic cache exists; add intent-safe defaults to minimize wrong-context hits. |
| Tool-result caching for deterministic read tools | `IN_PROGRESS` | MVP memory-backend implementation landed (`mcp.tool_cache`) with allowlist + TTL + identity/session scoping; Redis backend remains follow-on. |
| Agent security baseline pack | `NEXT` | "secure-by-default" profile: JWT/trusted edge, intent policy, tool allowlists, compliance export enabled. |
| Agent SDK examples (OpenClaw and generic agent clients) | `NEXT` | End-to-end examples proving lower token use and policy enforcement with minimal setup. |
| Agent ROI benchmark (token reduction + latency impact) | `PLANNED` | Publish reproducible benchmark showing savings from intent/tool caching and routing controls. |

Execution baseline for this track:
- [`openclaw_fleet_profile.md`](./openclaw_fleet_profile.md) (100-agent persona, ordered priorities, 6-week sequence).

---

## 5) Gap-to-Kong Scorecard (Update Monthly)

Scoring: `0 = missing`, `1 = starter`, `2 = competitive`, `3 = better-than-Kong for target users`.

| Capability Area | Score | Current Assessment |
|-----------------|-------|--------------------|
| AI data-plane performance and streaming clarity | `2` | Strong base; need repeated benchmark evidence to claim superiority. |
| MCP/agent governance | `2` | Differentiated direction; continue hardening policy packs and observability. |
| Enterprise security/compliance controls | `2` | Good starter set; supply-chain and deeper audit controls remain. |
| SLO automation and AI metrics | `1` | Tracing in place; richer metrics/automation needed. |
| gRPC completeness | `1` | Health endpoint only; full gRPC API path pending. |
| Control plane maturity | `0` | Not started as product surface. |
| Small-agent usability and ROI | `1` | Building blocks exist; needs packaged defaults, docs, and benchmark proof. |

Rule: do not claim "Kong replacement complete" until all required customer-target rows are at least `2`, and selected wedge rows are `3`.

---

## 6) 12-Month Execution Order (Rolling)

## Wave 1 (Now -> 3 months): Win the AI data-plane wedge

- Harden AI path where we already lead in clarity:
  - Ship **Agent Quickstart profile** aimed at OpenClaw-class clients (simple config, secure defaults, clear token controls).
  - Add **tool-result caching MVP** for deterministic read tools (opt-in, scoped, auditable).
  - Complete OTel AI metrics baseline (TTFT, stream duration, error classes, token counters).
  - Publish repeatable benchmark pack (hardware baseline + scripts + result template).
  - Finalize migration runbook for Kong coexistence -> AI-first split hostnames.
- Exit criteria:
  - Agent quickstart demo runs end-to-end with token usage visibility in one setup flow.
  - First published benchmark shows measurable token savings from caching and intent policies.
  - At least two production-like reference deployments with documented SLO outcomes.
  - Gap score for "AI data-plane performance and streaming clarity" reaches `3`.

## Wave 2 (3 -> 6 months): Enterprise trust package

- Prioritize enterprise blocker set:
  - Signed Wasm plugin verification / digest policy.
  - Expanded compliance/audit controls (field-level redaction policy + SIEM export guidance).
  - gRPC AI ingress/egress design finalized and phase-1 implementation started.
- Exit criteria:
  - Security/compliance gap score reaches stable `2+`.
  - First "Kong retirement by scope" migration case study.

## Wave 3 (6 -> 12 months): Control-plane MVP

- Build minimal but real control plane:
  - Tenant/project model.
  - RBAC.
  - Versioned config + staged rollout + rollback.
  - Data-plane registration/health and fleet view.
- Exit criteria:
  - One managed environment can run Panda without direct per-node YAML edits.
  - Control-plane maturity score reaches `1` and roadmap to `2` is approved.

---

## 7) Weekly Operating Rhythm

- **Weekly update (required):**
  - Move statuses in Section 4.
  - Add one bullet to "Progress Log".
  - Record blockers and decisions.
- **Monthly review:**
  - Re-score Section 5.
  - Re-prioritize `NEXT` items for the next month.

### Progress Log

- 2026-04-02: Added hierarchy USD estimates in `/tpm/status`, clarified streaming failover semantics in `/ready`, added feature-gated gRPC health server, enriched HTTP OTel span attributes, and aligned docs/runbooks.
- 2026-04-02: Elevated "small-agent adoption" to a first-class program track with prioritized quickstart, tool caching, and measurable token-ROI milestones.
- 2026-04-02: Added OpenClaw-class integration docs (`openclaw_agent_quickstart.md`) and Tool Cache MVP spec (`tool_cache_mvp.md`) to accelerate agent-first adoption.
- 2026-04-02: Added `openclaw_fleet_profile.md` with the four ordered priorities: (1) intent+loop bounds+metrics, (2) tool cache MVP, (3) safe semantic-cache defaults, (4) fleet views.
- 2026-04-02: Priority (1) partial: MCP agent metrics (`panda_mcp_agent_*`) + `/mcp/status` `agent_governance` block (intent policy summary + counter snapshot).
- 2026-04-02: Priority (2) partial: tool-result cache MVP implemented with `mcp.tool_cache` (memory backend), allowlist+TTL+identity/session scope, and `panda_mcp_tool_cache_*` counters.
- 2026-04-02: Implemented **buffered** mid-stream SSE failover (`model_failover.allow_failover_after_first_byte`, `midstream_sse_max_buffer_bytes`) and **memory** semantic-cache **embedding** near-miss (`semantic_cache.embedding_*`); updated `panda.example.yaml`, enterprise/runbook docs, and `/ready` text.
- 2026-04-02: Clarified **developer portal** program intent: **one** `/portal` surface for **existing** Panda features; no “full marketplace portal” milestone—API keys / heavy doc UX remain optional backlog (Phase **H** docs + `kong_replacement_program` Track D).
- 2026-04-02: Shipped **operator portal** upgrade: **`/portal/summary.json`** (read-only manageability snapshot), richer **`/portal`** HTML (live snapshot + quick links), OpenAPI + **`/ops/fleet/status`** endpoint list updated.

---

## 8) Decision Register (Short Form)

Use this table for major program decisions.

| Date | Decision | Why | Impact |
|------|----------|-----|--------|
| 2026-04-02 | Ship **buffered** mid-stream SSE failover (opt-in) instead of silent chunk splice | OpenAI SSE cannot be safely concatenated across providers; full-buffer replay + explicit cap + exclusions (MCP probe path, Anthropic adapter stream) | Operators get real retries with documented TTFT/limit tradeoffs; metric `panda_model_failover_midstream_retry_total` |
| 2026-04-02 | Ship gRPC as feature-gated health first | Deliver incremental protocol progress without destabilizing HTTP hot path | Lower risk path to full gRPC |

---

## 9) How to Mark Completion

When finishing a work item:

1. Update status in Section 4 (`IN_PROGRESS` -> `DONE`).
2. Add date-stamped note in "Progress Log".
3. If it changes competitive position, update Section 5 score.
4. Link the PR/commit in the note when available.

This keeps the document live and operational, not aspirational.
