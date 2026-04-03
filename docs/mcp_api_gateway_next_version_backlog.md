# MCP gateway + API gateway — next version backlog

**Purpose:** Track **what is not finished** in the combined MCP gateway and built-in API gateway, for **release planning**. Canonical status vs `main` stays in [`gateway_design_completion.md`](./gateway_design_completion.md); this file is the **prioritized remainder** only.

**Scope:** In-repo product gaps (code + operator docs). Security sign-off and environment-specific SLOs may live partly outside the repo.

---

## 1. MCP transport and protocol

| ID | Item | Notes |
|----|------|--------|
| **D-open** | **MCP streamable HTTP (full semantics)** | Today: minimal SSE wrap + in-memory session IDs. **Next:** session lifecycle, resumption, multi-chunk streams — align with client expectations beyond one JSON-RPC envelope per POST. |
| **D-open** | **JSON-RPC batch** on ingress MCP | Not implemented on `backend: mcp` POST path. |

---

## 2. API gateway — ingress

| ID | Item | Notes |
|----|------|--------|
| **F2** | **Ingress load / SLO evidence** | Dedicated methodology + numbers (or pointers to repeatable scripts) for **API gateway ingress** overhead, not only general chat/SSE runbooks. |
| **Catalog** | **Per-route auth** on ingress rows | Design gap vs [`panda_api_gateway_features.md`](./panda_api_gateway_features.md) / completion matrix. |
| **G2-open** | **Ingress rate-limit observability** | RPS enforcement works (local + optional Redis); **Prometheus** still lacks safe per-prefix / bounded labels (or aggregates) for ingress RL. |
| **Catalog** | **Ingress TLS** | Split listen (admin vs public), ACME, advanced cipher policy — beyond top-level `tls` on `listen`. |

---

## 3. API gateway — egress

| ID | Item | Notes |
|----|------|--------|
| **G3-open** | **Cluster-wide egress rate limits** | `max_in_flight` / `max_rps` are **process-local**. Optional **Redis** (or equivalent) for shared egress caps if multi-replica fairness matters. |
| **G3-open** | **Per-target / per-route egress caps** | Finer than global egress `rate_limit`. |
| **G4-open** | **Explicit cipher suite policy** | `min_protocol_version` + rustls defaults today; no config allowlist/denylist. |

---

## 4. Control plane (dynamic config)

| ID | Item | Notes |
|----|------|--------|
| **E partial** | **Multi-tenant RBAC / row-level namespacing** | Control plane exists; **full** tenant isolation and role model still open per plan. |
| **E** | **MySQL / Azure SQL stores** | Postgres (and file/sqlite/memory) path exists; other SQL backends not implemented. |
| **E4-open** | **OIDC roles for control plane** | Called out as open in implementation plan. |
| **E4-open** | **Redis-backed issuance/revocation UI** (if product needs it) | Distinct from static admin secrets. |

---

## 5. Hardening and release hygiene

| ID | Item | Notes |
|----|------|--------|
| **F3** | **Security review sign-off** | Checklist in [`implementation_plan_mcp_api_gateway.md`](./implementation_plan_mcp_api_gateway.md) §3.6 — needs explicit **done** record when GA-ready. |

---

## 6. Developer portal

| ID | Item | Notes |
|----|------|--------|
| **H2** | **Portal accuracy** | Ongoing: keep `/portal`, OpenAPI slice, tools catalog aligned with shipped behavior. |
| **H3** | **API keys + ingress wiring** | **Optional** product slice: self-service issuance/revocation and **using** those keys on ingress — not required for “baseline portal” complete. |

---

## 7. Cross-cutting

| ID | Item | Notes |
|----|------|--------|
| **Correlation** | **`route_id` (and friends) everywhere** | Partial today; completion matrix calls full plumbing **partial**. |
| **Per-identity limits** | **RPS / concurrency by caller** | Design gap beyond prefix-based ingress RL. |

---

## Suggested ordering (non-binding)

1. **F2 + F3** when you are targeting a **named GA** for the gateway surface.  
2. **Ingress RL metrics (G2-open)** + **egress caps (G3-open)** when operators need dashboards and multi-replica fairness.  
3. **Streamable MCP (full)** when a concrete client or spec milestone requires it.  
4. **H3** only if product commits to API-key-based access on ingress.  
5. **E** depth (RBAC, extra stores) when control-plane becomes customer-facing.

---

## Related

- [`gateway_design_completion.md`](./gateway_design_completion.md) — phase table vs code  
- [`implementation_plan_mcp_api_gateway.md`](./implementation_plan_mcp_api_gateway.md) — phase IDs (D–H)  
- [`panda_api_gateway_features.md`](./panda_api_gateway_features.md) — feature catalog  
