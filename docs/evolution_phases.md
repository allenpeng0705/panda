# Panda and Kong: evolution phases (1–3)

This document summarizes the **three deployment evolution phases** discussed for enterprises that already use **Kong** (or a similar L7 edge) and adopt **Panda** for AI traffic.

These phases describe **how organizations move over time**. They are **not** the same as the numbered **implementation phases** in [`implementation_plan.md`](./implementation_plan.md) (Phase 1 “Heart”, Phase 2 “Wasm”, etc.).

---

## Summary

| Phase | Name (informal) | Idea | Typical Kong role | Typical Panda role |
|-------|-----------------|------|-------------------|---------------------|
| **1** | **Coexistence** (“best neighbors”) | Lowest-risk start: add Panda without replacing the edge. | Stays at the **public edge**: TLS, coarse auth, routing, WAF-style policies, legacy APIs. | Sits **behind** Kong; receives **AI-only** routes (e.g. chat/completions). Handles streaming, TPM, MCP, semantic cache, AI policy. |
| **2** | **AI-first edge** (“shift of power”) | AI traffic gets a dedicated, high-performance path. | Serves **non-AI** and **legacy** APIs; may receive **internal** calls from agents that need old systems. | **Client-facing** for **AI hostnames** (e.g. `ai.company.com`); optimized for long-lived streams and tool loops. |
| **3** | **Optional consolidation** | Reduce two gateways where it makes business sense. | Shrinks or exits **per scope** as features move to Panda. | Absorbs selected Kong-style behaviors via **built-ins + Wasm** where parity is real for *your* routes. |

**Standalone (no Kong):** Teams without Kong start in a **“phase 0”** posture: [deployment guide — standalone](./deployment.md#standalone-no-kong)—Panda as the single gateway for AI (and generic HTTP proxying where configured), with TLS at the LB or on Panda. Configuration can use a top-level `listen` (and `tls`) or a nested **`server`** block (`listen` / `address`+`port` / `tls`). **`routes`** in `panda.yaml` support **longest-prefix** matching to different upstreams, plus optional per-route **HTTP RPS**, **TPM budget**, **semantic cache**, **MCP server allowlists**, and **adapter type** (`openai` / `anthropic`)—see `panda.example.yaml` for commented patterns.

### Informal “Phase 6 — The Edge” (Panda as the bouncer)

Some product narratives describe **“Phase 6 — The Edge”**: Panda acts as its own **edge**—the **bouncer**—with **TLS** and **standard HTTP routing** (multiple backends via declarative `routes`), not only AI-specific transforms. **That combination is available today:** terminate TLS on Panda (`tls` / `server.tls`), bind with `listen` or `server`, and route traffic with **`routes`** + default `upstream` (see [deployment](./deployment.md)). This is **orthogonal** to the **Kong deployment** phases 1–3 in the table above (you can run “edge Panda” in phase 0 or phase 2). It is also **not** the same label as **“Bouncer”** in [`implementation_plan.md`](./implementation_plan.md), where Phase 3 names **identity and security** (JWT, policy), not TLS/routing.

---

## Phase 1 — Coexistence

**Goal:** Ship AI gateway value **without** a risky edge migration.

- **Kong** terminates TLS, performs initial identity/routing, and forwards **only** AI-related paths to Panda.
- **Panda** enforces **token-centric** limits (TPM), **SSE-friendly** forwarding, **MCP** tool orchestration, **semantic cache**, and AI-aware policy.

**Panda support today**

- **Trusted edge → Panda** identity and anti-spoofing: [`kong_handshake.md`](./kong_handshake.md) (`trusted_gateway`, `PANDA_TRUSTED_GATEWAY_SECRET`, correlation / trace continuity).
- **Ops and observability** independent of Kong: `/health`, `/ready`, `/metrics`, OTLP, structured logs.

**Documentation**

- [Kong handshake](./kong_handshake.md) — header contract and recipe outline.
- [Integration & evolution](./integration_and_evolution.md) — coexistence and domain-split patterns.

---

## Phase 2 — AI-first edge

**Goal:** Put **Panda** on the **AI** front door while **Kong** (or direct services) continues to serve the long tail of legacy APIs.

- Prefer **split hostnames** (e.g. `ai.*` → Panda, `api.*` → Kong) so you do not stack **two** full TLS+OIDC stories on the same hostname without care.
- When an **agent** needs a **legacy** API that only exists behind Kong, traffic flows **Panda → Kong (internal)** over a **private** path (mTLS / mesh / VPC), not over the public internet.

**Panda support today**

- **Data plane** is ready to be the **primary** listener for AI on a dedicated host/port.
- **Multi-backend routing** (e.g. Panda → Kong for legacy, Panda → model provider) can be expressed in **`panda.yaml`** as **`routes`** with per-route **`upstream`** values, alongside the default `upstream`; longest path prefix wins. Optional per-route **RPS** (process-local), **TPM budget**, **semantic cache**, **MCP server allowlists**, and **adapter** type sit on top of **global** middleware (identity, Wasm, etc.). Anything not modeled as an upstream URL still follows **network and DNS** design as before.

**Documentation**

- [Integration & evolution — migration path](./integration_and_evolution.md#3-migration-path-from-behind-kong-to-in-front-of-kong) — Step 2 narrative.
- [Deployment](./deployment.md) — standalone vs optional edge, integrations.

---

## Phase 3 — Optional consolidation

**Goal:** **Optionally** reduce dual-gateway **operational** cost **where** feature parity is real.

- Re-implement **narrow** Kong plugins as **Wasm** (Rust / TinyGo) or use Panda first-party middleware.
- Decommission Kong **per application** or **per line of business**, not necessarily company-wide on one date.

**Panda support today**

- **Wasm** plugin runtime and PDK ([`implementation_plan.md`](./implementation_plan.md) Phase 2 / Wasm notes; see repo `crates/panda-pdk`, `panda-wasm`).
- **Full** Kong feature parity is **not** a single switch—depends on which Kong plugins and routes you rely on.

---

## How this relates to “implementation phases”

| Document | What “phase” means |
|----------|---------------------|
| **This file** | **Organizational / deployment** evolution with Kong (1–3). |
| [`implementation_plan.md`](./implementation_plan.md) | **Engineering** roadmap (Heart, Wasm, Bouncer, Brain, Enterprise finish). |

You can be **implementation-complete** on core AI gateway features while still in **deployment Phase 1** (Panda behind Kong)—that is normal.

---

## See also

- [Integration & evolution](./integration_and_evolution.md)
- [Kong handshake](./kong_handshake.md)
- [Deployment](./deployment.md)
- [High-level design](./high_level_design.md)
- [Core vs Enterprise — self-serve defaults and optional enterprise features](./enterprise_track.md)
