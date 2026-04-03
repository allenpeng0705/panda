# OpenClaw Fleet Profile (Discussion Baseline)

This is the canonical persona for planning agent-first Panda work.

---

## Persona: "100-agent cloud team"

- 100 OpenClaw-class agent runtimes in Kubernetes.
- 10 tenants (teams), each with 5-20 active agents.
- Mixed workloads:
  - 50% retrieval/research/tool-heavy,
  - 30% structured automation tasks,
  - 20% complex planning tasks.
- Provider mix: one primary model provider + one failover provider.
- Shared Redis for budgets and cache.

---

## Main pain points

1. Token spend spikes from repeated tool and prompt context.
2. Unsafe tool exposure (too many tools advertised to every prompt).
3. Hard-to-debug agent loops and retries.
4. No single fleet view for budget, policy, and tool behavior.

---

## What Panda must provide first (ordered)

## 1) Intent-scoped tools + loop bounds + great status/metrics

Minimum deliverables:

- Intent-scoped tool advertising on by policy.
- Per-session/profile `max_tool_rounds` enforcement.
- Agent-focused status:
  - `/mcp/status`: effective rounds, intent policy summary.
  - `/tpm/status`: per-session/profile budget state.
- Metrics:
  - tool rounds used,
  - policy deny counts,
  - budget rejection counts.

Success criteria:

- 30%+ reduction in average tools advertised per request.
- 0 runaway loops beyond configured round caps.

## 2) Tool-result cache MVP (allowlisted, TTL, identity-scoped)

Minimum deliverables:

- Per-tool allowlist.
- TTL per tool with global defaults.
- Identity/session scoped keying.
- Bypass and audit events.

Success criteria:

- >=20% cache hit rate on designated deterministic tools.
- No cross-tenant cache leakage incidents.

## 3) Semantic cache defaults tuned for agents (safe-by-default)

Minimum deliverables:

- Conservative default threshold/profile for agent routes.
- Include model/tool/system fingerprint in cache key policy.
- Safe fallback behavior and visibility on bypass.

Success criteria:

- >=15% token reduction on selected agent flows.
- No correctness regressions in canary flows.

## 4) Richer per-agent fleet views (JSON first, Grafana second)

**Phased gap plan:** [`agent_fleet_gap_roadmap.md`](./agent_fleet_gap_roadmap.md) (what is still missing, priorities, Panda vs external ownership).

Operational runbook: [`runbooks/agent_fleet_oncall.md`](./runbooks/agent_fleet_oncall.md).

Minimum deliverables:

- JSON endpoints remain primary source.
- Grafana starter dashboard over exported metrics:
  - TPM rejects and MCP/tool-cache/semantic-cache aggregates (low-cardinality labels),
  - tool rounds and cache hit/miss,
  - deny/reject and retry/failover trends (plus `panda_model_failover_midstream_retry_total` when buffered failover is enabled).

Success criteria:

- On-call can identify the **top token-burning principal** in &lt;5 minutes using the **documented join** (low-cardinality metrics + logs/compliance + `/tpm/status` with that principal’s headers) — not by adding high-cardinality tenant labels to every Prometheus series.
- Weekly optimization loop runs from dashboard + status data.

---

## Suggested 6-week execution sequence

Use [`agent_fleet_gap_roadmap.md`](./agent_fleet_gap_roadmap.md) for the **cross-cutting** backlog (fleet joins, audit hardening, semantic-cache depth, resilience, Wasm). The **original** sequencing below remains a reasonable default **within** the OpenClaw fleet profile:

- Week 1-2: Item 1 hardening and agent metrics completeness.
- Week 2-3: Item 2 implementation (single low-risk tool family first).
- Week 3-4: Item 3 safe defaults + canary.
- Week 5-6: Item 4 dashboard pack + runbook.

