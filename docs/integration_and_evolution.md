# Panda: Integration & Evolution Design

**Audience:** Enterprises where **Kong (or similar) is already the standard**.  
**Intent:** Panda is positioned as **enterprise AI strategy**, not only a binary: by accepting the **Kong-heavy** reality, adoption reads as a **critical upgrade path** for AI traffic rather than a risky wholesale replacement.

The [high-level design](./high_level_design.md) describes Panda’s layers; this document describes **how Panda sits next to Kong** and how adoption evolves.

---

## 1. Coexistence architecture — the “best neighbor” strategy

Most enterprises will **not** remove Kong on day one. Panda’s **primary** design supports a **side-by-side** or **layered** approach.

### Deployment mode A: Panda as the “AI sidecar” (recommended)

Kong remains the **edge bouncer**: TLS termination, global DDoS protection, initial OIDC authentication, and routing of **legacy** traffic. Kong forwards **AI-specific** paths (e.g. `/v1/ai/*` or your chosen prefix) to Panda.

| Component | Role |
|-----------|------|
| **Kong** | Edge security, global rate limits, legacy REST routing. |
| **Panda** | Token accounting (TPM), streaming stability, semantic caching, MCP tool orchestration, intent-aware policy. |

**Value:** AI-specific reliability and cost control **without** breaking existing DevOps pipelines or re-homing thousands of routes on day one.

### Domain split pattern (preferred for “Panda at the AI edge”)

When Panda moves to the **client-facing edge** for AI (Step 2), **avoid terminating TLS and OIDC twice on the same hostname**. Prefer **split DNS / hostnames**:

| Host | Terminates TLS | Role |
|------|----------------|------|
| **`ai.company.com`** | **Panda** | AI clients; TPM, intent, MCP, streaming-native path. |
| **`api.company.com`** | **Kong** | Existing legacy REST and non-AI workflows (unchanged). |

**Inter-connectivity:** If a Panda-backed agent or tool needs a **legacy** resource exposed only through Kong, Panda calls Kong over an **internal** path—ideally **mTLS** (service mesh, private VPC, or Kong’s internal listener)—not over the public internet. Identity and correlation propagate via **trusted internal headers** and **W3C trace context**; see the **[Kong handshake contract](./kong_handshake.md)** (header names, attestation, correlation) and [implementation plan](./implementation_plan.md#kong-handshake-milestone-13) for milestone context.

This pattern keeps **one TLS + one primary auth story per hostname** and makes ownership obvious to platform teams.

---

## 2. Competitive design: where Panda addresses the “AI tax”

Generic gateways often pay an **AI tax** when LLM traffic is forced through generic plugins and buffering. Panda targets **native** handling on the hot path.

| Dimension | The “Kong tax” (typical plugin-style AI) | The Panda way (native AI data plane) |
|-----------|------------------------------------------|--------------------------------------|
| **Streaming** | Buffers or wraps SSE; risk of “hanging” or brittle timeouts. | Stream-first path, **minimal copy** on hot paths (see [implementation plan](./implementation_plan.md)), SSE-friendly heartbeats and timeouts. |
| **Throttling** | Requests per second (RPS). | **Tokens per minute (TPM)** and related token budgets to cap spend. |
| **Caching** | Exact URL / key match (e.g. `?q=hello`). | **Semantic** match (e.g. near-duplicate prompts). |
| **Intelligence** | Opaque bodies; limited intent context. | **Intent-aware** routing and tool-call alignment (e.g. proof-of-intent); see [AI routing strategy](./ai_routing_strategy.md) for semantic / tool / MCP / agent / model routing tiers. |
| **Extensibility** | Lua-centric history. | **Rust / Go via Wasm**; closer to mainstream service engineering. |

Exact behavior is implementation-dependent; this table states **design intent** and positioning.

---

## 3. Migration path: from “behind Kong” to “in front of Kong”

For a concise **Phase 1–3** summary (coexistence → AI-first edge → optional consolidation), see **[evolution phases](./evolution_phases.md)**.

Evolution is a **choice**, not a one-shot cutover. A practical three-step arc:

### Step 1: The AI upstream (coexistence)

- **Action:** Register Panda as a **new upstream service** in Kong.
- **Scope:** Only routes that touch **LLMs or agents** point to Panda.
- **Value:** Streaming and cost observability improve **immediately** with minimal blast radius.

### Step 2: Gateway-of-gateways (the shift)

- **Action:** For **AI-first** applications, place Panda **closer to the client** for **AI hostnames only**—prefer the **[domain split](#domain-split-pattern-preferred-for-panda-at-the-ai-edge)** (`ai.*` → Panda, `api.*` → Kong) so TLS and OIDC are not duplicated on one host.
- **Behavior:** Panda forwards **legacy** calls **to Kong** (or directly to services) over **internal mTLS** when an agent needs resources behind the existing gateway.
- **Value:** **Centralizes AI governance** while legacy stacks keep running behind Kong.

### Step 3: Full consolidation (replacement, optional)

- **Action:** Gradually re-home Kong capabilities (auth, CORS, logging, transforms) into **Panda built-ins + Wasm** where desired.
- **Goal:** Decommission Kong **only** when **feature parity** exists for **that** organization’s **actual** route and plugin surface.

Replacement is **per scope** (per app, per line of business), not necessarily company-wide on a single date.

---

## 4. Design patterns for “easy replacement” (familiar ops)

### 4.1 Familiar mental model (Kong-isms, improved implementation)

| Concept | Meaning in Panda |
|---------|------------------|
| **Service** | Logical model provider or backend (e.g. `openai-gpt4`, `anthropic-coding`). |
| **Route** | Path/method/host match → service + policy profile. |
| **Plugin** | **Wasm module** (and first-party middleware) composed on the chain. |

**What ships in `panda.yaml` today (Kong-like, not Kong-identical):** longest-prefix **`routes`** with per-route **`upstream`**, optional **`rate_limit.rps`** (HTTP), **`tpm_limit`**, **`semantic_cache`**, **`mcp_servers`**, and **`type`** (adapter). A nested **`server`** block can set **`listen`** / **`address`+`port`** / **`tls`** instead of (or layered with) top-level `listen` and `tls`. See **`panda.example.yaml`** for patterns. Full Kong parity (per-route plugins DB, Lua, control plane) is **not** the goal; the goal is **GitOps-friendly** static config for teams that want one binary and one file.

**Improvement:** Panda is **GitOps-native** by default—one (or a few) versioned YAML files per environment, without a mandatory heavy DB inside every data-plane replica. (Shared stores for cache/TPM remain **optional**, as in the [implementation plan](./implementation_plan.md).)

### 4.2 Shadow mode

Panda should support a **shadow / observe** mode when placed behind Kong:

- Log what **would** have happened under full enforcement: e.g. “would have redacted PII,” “would have been a semantic cache hit,” “would have blocked tool X given intent Y.”
- **Value:** Proves ROI and builds trust **before** turning on hard enforcement or moving traffic.

### 4.3 Unified observability (OpenTelemetry)

Panda emits **OTel** metrics and traces compatible with existing backends (Datadog, Grafana, etc.) so teams are **not** asked to rebuild dashboards for the AI tier.

For streaming MCP operations, use the [operator playbook in the high-level design](./high_level_design.md#operator-playbook-streaming-mcp-probe-tuning) to tune `mcp.stream_probe_bytes` and `mcp.probe_window_seconds` from real traffic signals, and use `/mcp/status` enrichment fields (`enrichment_enabled`, `enrichment_rules_count`, `enrichment_last_mtime_ms`) to validate context-enrichment cache behavior in production.

---

## 5. Summary: value proposition

Panda is not only a **faster proxy**; it is **specialized depth** for AI traffic.

**Message to enterprise users:** *Keep Kong for the thousand legacy REST APIs. Use Panda for the routes that matter for your AI future. When you are ready—and only where parity exists—Panda can absorb more of the edge.*

---

## 6. Market entry: layered vs. “big bang”

**Yes — the layered / neighbor strategy is both safer and usually more attractive** than big-bang replacement:

- **Lower risk:** One upstream and a handful of routes; no forced migration of unrelated APIs.
- **Faster proof:** Shadow mode and TPM/cache wins are visible without org-wide commitment.
- **Aligned procurement:** Platform teams approve an **AI gateway**; they are not asked to rip out a certified edge on day one.

Big-bang replacement remains a **valid end state for some apps**, but it should be **optional** and **late**, not the default sales or engineering story.

---

## 7. Agent protocols beyond REST (MCP, A2A, …)

Edge coexistence (above) is about **where** Panda sits. **Protocol evolution** is about **what** agents speak (MCP for tools today, agent-to-agent specs emerging). Panda’s direction is **pluggable protocol modules** and shared governance—not a bet on a single wire format. See **[protocol evolution](./protocol_evolution.md)**.

---

*This document should evolve with customer deployment patterns and product capabilities.*
