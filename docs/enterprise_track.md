# Enterprise track (road to revenue)

This document describes a **commercial “Enterprise” tier** narrative for **2026**: capabilities that large buyers expect before they standardize on Panda as a cluster-wide AI gateway. It is a **product and architecture roadmap**, not a promise of shipping dates.

**Relationship to other docs:** Deployment evolution with Kong is in [`evolution_phases.md`](./evolution_phases.md). Engineering milestones remain in [`implementation_plan.md`](./implementation_plan.md). Today’s console is documented in [`developer_console.md`](./developer_console.md).

---

## Summary

| Pillar | Buyer outcome | Panda direction |
|--------|----------------|-----------------|
| **SSO for Developer Console** | Okta / Microsoft Entra–backed access to the live-trace UI; no shared static ops secret as the long-term model. | Add **OIDC (Authorization Code + PKCE)** for `/console` (and optionally `/console/ws`), with **session** or **token** validation on each request; keep **ops shared secret** as a **break-glass / automation** path. |
| **Hierarchical budgets** | Company-wide cap with **department (or cost-center) sub-caps**—e.g. $5k Marketing, $2k Sales—**enforced in one cluster**. | Generalize **budget identity** beyond flat TPM buckets: **tree-structured limits** (org → dept → team) with **shared Redis (or control-plane API)** for authoritative counters; surface **USD** (from **pricing tables** + token usage) alongside **tokens**. |
| **Automated failover (model parity)** | If **primary** model provider is down or **SLA-breached**, traffic **fails over** to an **equivalent** model on another vendor (e.g. OpenAI → Azure OpenAI → Anthropic) using a declared **parity map**. | First-class **`upstream_groups`** (or **model routes**) with **health probes**, **circuit breaking**, and **request-shape adapters** already partially aligned with universal adapter work; extend **`rate_limit_fallback`–style** paths into a **general failover policy** driven by config + metrics. |

---

## 1. SSO: Okta and Microsoft Entra for the Developer Console

**Today:** The console is protected by the same **optional shared secret** as other ops routes ([`developer_console.md`](./developer_console.md)). That is appropriate for **dev** and **break-glass**, not for **hundreds of engineers** with role-based access.

**Enterprise target**

- **Protocols:** OpenID Connect (OIDC) with **Authorization Code + PKCE** for browser login to `/console`.
- **IdPs:** Okta, Microsoft Entra ID (Azure AD), and any OIDC-compliant provider via **issuer URL** + **client id/secret** (or confidential client with **rotated** credentials).
- **Authorization:** Map IdP **groups / claims** to Panda **roles** (e.g. `console.viewer`, `console.admin`, `ops.readonly`). Deny by default for console routes when SSO is enabled and the user lacks a role.
- **WebSocket:** `/console/ws` must carry a **short-lived session** or **signed ticket** issued after OIDC login (query param or `Sec-WebSocket-Protocol` patterns are common; avoid long-lived secrets in URLs in production).

**Architecture notes**

- **Session store:** Encrypted cookie + server-side session (Redis) scales across replicas; alternatively **JWT session** with short TTL + refresh if you accept stateless tradeoffs.
- **Separation of concerns:** **Edge (Kong)** may still terminate OIDC for **API** clients; **console SSO** can be **Panda-native** so the UI does not depend on Kong plugin availability.

**Build sequencing (suggested)**

1. OIDC login/callback endpoints and session middleware scoped to `/console` only.  
2. Group → role mapping in config.  
3. Harden `/console/ws` with the same identity.  
4. Optional: SAML bridge via IdP (Okta/Entra) if customers require SAML at the IdP while Panda speaks OIDC.

---

## 2. Hierarchical budgets (org → department → …)

**Today:** TPM and related telemetry use **identity-derived buckets** (e.g. subject, tenant, subject+tenant) and optional **Redis** for shared counters ([`implementation_plan.md`](./implementation_plan.md), TPM sections). That is **flat**, not **hierarchical**, and **token-centric** rather than **dollar-centric**.

**Enterprise target**

- **Hierarchy:** Configurable **tree** of budget nodes (e.g. `org:acme` → `dept:marketing` → `dept:sales`). Each request resolves **one leaf** (or multiple ancestors) from **JWT claims**, **trusted gateway headers**, or **custom** Wasm-enriched metadata.
- **Enforcement:** A spend event (prompt + completion, optionally **cached** vs **uncached**) increments **every ancestor node** up to root until **any** node would exceed its limit—then **429** (or **policy deny**) with a clear **which limit fired** in headers/body for support.
- **USD:** Maintain **versioned price cards** (per model, per provider, per effective date). **Estimated USD** = f(tokens, model, direction). Store **integer micro-dollars** or **decimal** with explicit rounding rules for audit.
- **Storage:** **Redis** (or a **small control-plane service**) holds rolling windows per node; **GitOps** remains the source of **limits**, not live counters.

**Why buyers care**

- Finance and procurement see **one** cluster enforcing **delegated** caps without running **separate** gateways per department.

**Dependencies**

- Stable **tenant / cost-center** claims from IdP or edge (aligns with [`kong_handshake.md`](./kong_handshake.md) identity headers).  
- **Compliance export** and metrics should tag **budget node id** for disputes ([`compliance_export.md`](./compliance_export.md)).

---

## 3. Automated failover and the “model parity map”

**Today:** Panda has **multi-route upstreams**, **adapters** (OpenAI vs Anthropic shapes), and **rate-limit / fallback** style behavior in specific flows ([`implementation_plan.md`](./implementation_plan.md) Phase 4 notes). There is **no** single documented **parity map** object in config.

**Enterprise target**

- **Model parity map:** Declarative mapping: **logical model name** (customer-facing) → **ordered list** of **physical endpoints** (provider URL + credentials ref + **adapter**), each tagged with **equivalence class** (e.g. “chat JSON, function tools, 128k class”).
- **Triggers:** **HTTP errors** (5xx, 429), **timeout**, **circuit breaker** open, optional **synthetic probes** (lightweight health checks) to mark a **backend unhealthy**.
- **Behavior:** On failure of attempt *i*, **retry once** on *i+1* with the **same** normalized request (adapter applies provider-specific encoding). Cap **max hops** to avoid storms. **Stream** failover is harder than unary JSON—policy may be **fail only before first byte** or **restart stream** with explicit product semantics.
- **Observability:** Metrics **per logical model** and **per physical backend** (`failover_total`, `active_backend`) for SRE dashboards.

**Alignment with existing design**

- **Universal adapter** work reduces duplicate glue per provider.  
- **Semantic cache** keys should include **logical model** so a failover does not silently serve a **wrong** provider’s cached answer unless **explicitly** allowed.

---

## Packaging as “Enterprise tier”

A practical **SKU** split for 2026 conversations:

| Area | Core / open usage | Enterprise |
|------|-------------------|------------|
| Console | Shared-secret ops gate, single team | **SSO**, **RBAC**, audit of console access |
| Budgets | Per-user / per-tenant TPM | **Hierarchy + USD** + finance-friendly exports |
| Routing | Static `upstream` / `routes` | **Parity map**, **circuit breaking**, **SLO-driven** failover |

This framing matches how buyers compare you to **managed gateways** and **cloud provider** proxies: **data-plane performance** in core, **governance and resilience** in Enterprise.

---

## See also

- [`panda_vs_kong_positioning.md`](./panda_vs_kong_positioning.md) — where Panda sits relative to the edge.  
- [`high_level_design.md`](./high_level_design.md) — intent-aware gateway pillars.  
- [`integration_and_evolution.md`](./integration_and_evolution.md) — coexistence and domain split.
