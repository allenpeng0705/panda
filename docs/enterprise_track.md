# Core vs Enterprise (self-serve and revenue tracks)

Panda is **one binary and one config model**: **small teams and free users** stay fast and simple; **enterprise** features are **optional**—off by default—so they never block the quick path.

**Relationship to other docs:** Deployment patterns are in [`evolution_phases.md`](./evolution_phases.md) and [`deployment.md`](./deployment.md). Engineering milestones are in [`implementation_plan.md`](./implementation_plan.md). The Developer Console is in [`developer_console.md`](./developer_console.md).

---

## Core track: easy and quick (default)

**Who:** Solo developers, startups, small companies, hobbyists, internal PoCs.

**Goal:** Go from **zero → proxying traffic** in minutes without SSO, Redis, or multi-provider topology.

| Area | Core experience |
|------|------------------|
| **Install** | Copy [`panda.example.yaml`](../panda.example.yaml) → `panda.yaml`, set `listen` + `upstream`, run `panda` (see [QuickStart in README](../README.md#quickstart)). |
| **Identity** | Optional JWT/JWKS or trusted-gateway headers when you need them; **anonymous or simple bearer** is fine for early use. |
| **Budgets** | **Flat TPM** (per subject/tenant bucket) with optional Redis for replicas—**no** org tree, **no** USD ledgers required. |
| **Routing** | Single **`upstream`** or **`routes`** with one backend per path—**no** parity map or automatic failover required. |
| **Console** | Optional **shared-secret** ops gate (`observability.admin_secret_env`)—enough for a **small trusted team** ([`developer_console.md`](./developer_console.md)). |
| **Dependencies** | **None** beyond TLS certs if you terminate HTTPS; Redis and OTLP are **optional** quality-of-life. |

**Product principle:** Every **Core** workflow must stay **one YAML file** and **minimal env vars** unless the operator explicitly opts in.

---

## Enterprise track: optional governance and resilience

**Who:** Mid-market and large orgs with IdP standards, finance-owned spend caps, and SLOs on model availability.

**Goal:** Same binary; enable **Enterprise** capabilities through **config blocks / feature flags** (and, if you ship commercial tiers, **license gates**) without forking the open-source UX.

| Pillar | Enterprise outcome | Implementation direction |
|--------|--------------------|---------------------------|
| **SSO for Developer Console** | Okta / Microsoft Entra (and OIDC peers) for `/console` + `/console/ws`; **RBAC** from groups/claims. | **OIDC Authorization Code + PKCE**, session or short-lived tokens; keep **shared ops secret** as **break-glass** and for automation. |
| **Hierarchical budgets** | e.g. $5k **Marketing**, $2k **Sales**, under one **org** cap in **one** cluster. | **Budget tree** in config + **Redis** (or control-plane API) for counters; **USD** from versioned **price cards** × token usage; compliance/metrics tag **budget node**. |
| **Automated failover + model parity map** | Primary provider unhealthy → **next equivalent** backend (e.g. OpenAI → Azure OpenAI → Anthropic). | **Logical model** → ordered **physical backends**, health probes, **circuit breaking**, adapters on each hop; clear policy for **streaming** vs unary JSON. |

Details for each pillar (architecture notes and sequencing) follow the same headings as before; they apply **only when Enterprise features are enabled**.

---

## 1. SSO: Okta and Microsoft Entra for the Developer Console

**Core default:** Shared-secret protection for `/console` (optional). No OIDC required.

**Enterprise:** When `console_oidc.enabled=true` (default is `false`) and the console is enabled:

- **Protocols:** OpenID Connect Authorization Code flow for browser login to `/console` (Panda callback at `/console/oauth/callback`).
- **IdPs:** Okta, Microsoft Entra ID, and OIDC-compatible providers via **issuer** + **client** credentials.
- **Authorization:** Map IdP **groups / claims** to Panda **roles** (e.g. `console.viewer`, `console.admin`). Deny by default when SSO is on and the user lacks a role.
- **WebSocket:** `/console/ws` uses a **session** or **signed ticket** from the OIDC login (avoid long-lived secrets in URLs in production).

**Architecture notes**

- **Session store:** Encrypted cookie + Redis-backed session scales across replicas; or short-lived JWT + refresh if you accept stateless tradeoffs.
- **Edge vs Panda:** Kong may terminate OIDC for **API** traffic; **console SSO** can remain **Panda-native** so the UI does not depend on a specific edge plugin.

**Build sequencing (suggested)**

1. OIDC login/callback + session middleware scoped to `/console`.  
2. Group → role mapping in config.  
3. Align `/console/ws` with the same identity.  
4. Optional: SAML at the IdP while Panda speaks OIDC to the client.

---

## 2. Hierarchical budgets (org → department → …)

**Core default:** Flat TPM buckets (`subject` / `tenant` / combined) and optional Redis—documented in [`implementation_plan.md`](./implementation_plan.md).

**Enterprise:** When `budget_hierarchy.enabled=true` (default is `false`):

- **Hierarchy:** Configurable **org + department** limits. Resolve department from JWT claim (`budget_hierarchy.jwt_claim`, e.g. `department`).
- **Enforcement:** Each spend event increments **ancestors** toward the root; first **over-limit** node wins → **429** / policy response with **which node** tripped.
- **USD:** Versioned **price cards** (model, provider, effective date); store amounts with explicit rounding for audit.
- **Storage:** Redis counters (using `budget_hierarchy.redis_url`, else `tpm.redis_url` / `PANDA_REDIS_URL`); GitOps remains the source of limits.

**Dependencies:** Stable **cost-center / department** claims; extend **compliance export** with **budget node id** ([`compliance_export.md`](./compliance_export.md)).

---

## 3. Automated failover and the “model parity map”

**Core default:** One `upstream` or explicit **`routes`** — operators handle outages manually or at the load balancer.

**Enterprise:** When `model_failover.enabled=true` (default is `false`):

- **Parity map:** Request `model` → ordered backends from `model_failover.groups`. Paths are classified separately: **`path_prefix`** (chat), optional **`embeddings_path_prefix`**, **`responses_path_prefix`**, **`images_path_prefix`**, and **`audio_path_prefix`**.
- **Capabilities:** Each backend declares **`protocol`** (`openai_compatible` or `anthropic`) and optional **`supports`** (`chat_completions`, `embeddings`, `responses`, `images`, `audio`). Empty `supports` means defaults for that protocol (Anthropic: chat only; OpenAI-shaped: all operations). Anthropic hops map **OpenAI chat JSON** to **`/v1/messages`** for that hop only, including tool definitions/selection mapping.
- **Triggers:** 5xx, 429, timeouts on the current hop before the response is returned. **Streaming:** HTTP 200 + SSE is not retried on status alone by default; `allow_failover_after_first_byte` remains default `false`.
- **Circuit breaker:** Optional local breaker (`circuit_breaker_enabled`) opens per backend after `circuit_breaker_failure_threshold` retryable failures for `circuit_breaker_open_seconds`.
- **Global adapter:** If the main request is already transformed to Anthropic (`adapter.provider=anthropic` for that route), **model failover is skipped** to avoid double-mapping.
- **Provider coverage:** OpenAI-compatible providers (for example DeepSeek, Qwen-compatible endpoints, MiniMax OpenAI-compatible endpoints) can be added directly as `protocol=openai_compatible` backends. Non-compatible providers should use a protocol adapter hop (currently Anthropic is built in; others can be added as new protocol adapters).

**Alignment:** Universal **adapter** work; **semantic cache** keys must include **logical model** so failover does not return a **wrong-provider** cache entry unless explicitly allowed.

---

## Packaging summary

| Area | Core / self-serve | Enterprise (opt-in) |
|------|-------------------|------------------------|
| Onboarding | Example YAML + upstream URL | Same; then enable SSO / budget tree / parity blocks |
| Console | Optional shared secret | **SSO + RBAC** (+ break-glass secret) |
| Budgets | Flat TPM (+ optional Redis) | **Hierarchy + USD** + finance-friendly exports |
| Routing | Static `upstream` / `routes` | **Parity map**, health, **circuit breaking**, SLO-driven failover |

**Messaging:** Panda leads with **speed and simplicity** for the long tail; **Enterprise** is “turn on when you need IdP-backed console access, delegated spend caps, and provider failover”—not a separate product fork.

---

## See also

- [`panda_vs_kong_positioning.md`](./panda_vs_kong_positioning.md) — edge vs AI gateway.  
- [`high_level_design.md`](./high_level_design.md) — pillars.  
- [`integration_and_evolution.md`](./integration_and_evolution.md) — coexistence and domain split.
