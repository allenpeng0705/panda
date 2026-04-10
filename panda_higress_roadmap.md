# Panda Gateway: Enterprise Roadmap (Higress-Informed)

## Purpose

This document compares [Alibaba Higress](https://github.com/alibaba/higress) with **Panda** and outlines a phased path toward **enterprise-grade** operation: security, operability, ecosystem, and adoption—without assuming Panda should become a clone of Higress.

**Inputs reviewed:** local `../higress` tree (README and layout), Panda’s `README.md` and docs, and the upstream Higress positioning (AI gateway, Wasm plugins, MCP hosting, console, K8s ingress).

---

## Executive summary

**Higress** is a **cloud-native** stack: **Istio + Envoy**, Wasm plugins (Go/Rust/JS), a **first-class K8s ingress / Gateway API** story, **multi-registry** service discovery, a **separate console** repo, and a large **official plugin** catalog. It optimizes for **control-plane–driven** config, **millisecond** route updates, and **SSE/streaming** in AI workloads.

**Panda** is a **Rust unified gateway**: one binary, **unified YAML**, **API + MCP + AI** in one place, **stream-first** proxying, **low-cardinality metrics**, Wasm plugins, and features already aimed at **governance** (JWT/JWKS, audit JSONL, token budgets, semantic cache). Enterprise-oriented pieces are described in-repo (e.g. hierarchical budgets, model failover, console SSO in `README.md`).

**Strategic takeaway:** Learn from Higress’s *product surface* (what enterprises expect: console, plugin hub, MCP lifecycle, auth breadth, K8s packaging, observability contracts). Implement on Panda’s *architecture* (Rust core, YAML/control plane as you define it, existing MCP and AI paths) rather than re-deriving Envoy/Istio.

---

## Side-by-side positioning

| Dimension | Higress (reference) | Panda (current direction) |
|-----------|---------------------|---------------------------|
| Core runtime | Envoy data plane + Istio-style control plane | Rust `panda-server`, custom proxy stack |
| Config model | K8s CRDs / Ingress+annotations / Gateway API + console | Unified `panda.yaml`, optional control plane REST (`docs/testing_scenarios_and_data_flows.md`) |
| Plugins | Rich Wasm hub; Go SDK (`wasm-go`); C++/AssemblyScript | Wasm plugins + PDK (`crates/panda-pdk`); community plugins |
| AI gateway | `ai-proxy` providers, caching, LB, token limits | OpenAI-compatible routes, adapters, TPM, semantic cache, model failover (as configured) |
| MCP | Hosted MCP via plugins; **openapi-to-mcp** toolchain | MCP host (stdio/HTTP/remote), tool orchestration, registry examples (`panda-mcp-registry/`) |
| Ingress / K8s | First-class ingress controller; Gateway API | Deploy via Docker/K8s; not the same as replacing ingress-nginx |
| Service discovery | Nacos, ZK, Consul, Eureka, Dubbo ecosystem | Typically explicit upstreams; SD is a **gap** if you target microservice-gateway parity |
| Console | Separate **higress-console**; demo / all-in-one Docker | Control plane endpoints exist; full **product UI** is roadmap |
| Ops story | Prometheus ecosystem, enterprise SLAs on cloud | `/metrics`, `/ops/fleet/status`, `/ready`, compliance logging—strong **telemetry** direction |

---

## What to learn from Higress (concrete)

1. **Unified “AI + MCP” management** — One place to govern LLM routes and MCP tool exposure, with consistent auth, rate limits, and audit. Panda already unifies paths; the gap is **packaging** (UI, policy templates, docs) and **tooling** (OpenAPI → MCP parity with [openapi-to-mcpserver](https://github.com/higress-group/openapi-to-mcpserver)).
2. **Plugin as a product** — Versioned artifacts, a **hub** or catalog, examples, and CI for Wasm builds. Higress separates **plugin-server** and **wasm-go**; Panda can standardize on **PDK + registry story** without copying their layout exactly.
3. **Enterprise checklist** — WAF/CC/auth methods/ACME are **table stakes** for “security gateway” positioning; pair with **SLO-oriented** behavior (no reload jitter is Envoy’s story; Panda should document **config reload**, long-lived SSE, and failover semantics—already partially in `/ready` and docs).
4. **K8s-native distribution** — Helm charts, regional image mirrors, Gateway/Ingress **compatibility** only if you commit to that segment; otherwise position as **sidecar-friendly** or **standalone edge** with clear boundaries.
5. **Related repos to study** (not dependencies): [higress-console](https://github.com/higress-group/higress-console), [higress-standalone](https://github.com/higress-group/higress-standalone), [plugin-server](https://github.com/higress-group/plugin-server), [wasm-go](https://github.com/higress-group/wasm-go), [openapi-to-mcpserver](https://github.com/higress-group/openapi-to-mcpserver).

---

## Gap analysis: Panda today vs enterprise bar

**Strong or on-track (relative to README and tests):** Rust performance story, streaming/SSE, MCP + AI in one gateway, observability discipline (low-cardinality metrics), Wasm extension, JWT and compliance-oriented logging, control plane auth matrix (see test docs).

**Gaps to close for “enterprise gateway” narratives:**

| Area | Gap | Notes |
|------|-----|--------|
| Management plane | No full **web console** product comparable to Higress UI | API-first console may exist; productize UX, RBAC, audit of changes |
| Plugin ecosystem | **Catalog + versioning + docs** vs ad hoc community | Align with `community-plugins/` and PDK; optional registry |
| MCP | **OpenAPI → MCP** tooling and **hosted** lifecycle | Hosting may differ: Panda “hosts” tools via config vs Wasm plugin servers |
| Security | **WAF**, broader auth parity (HMAC, basic, OIDC depth), **ACME** | Many are incremental; order by customer demand |
| Multi-cluster / registry | **Service discovery** (Nacos, Consul, …) | Major effort; only if Panda targets service-mesh-adjacent use cases |
| K8s | **Ingress/Gateway API** as first-class | Strategic fork: either invest or document “not an ingress controller” |
| Reliability SLOs | Published **latency/availability** targets and test evidence | Benchmarks exist (`benchmarks/`); tie to enterprise doc set |
| Community | Higress-scale **contributor** funnel | Grow via clear CONTRIBUTING, plugin templates, public roadmap |

---

## Roadmap phases (refined)

Phases are **sequenced** so each builds on the last. Timelines are indicative; adjust per team size.

### Phase 1 — Foundation (0–3 months): ship clarity + fastest enterprise wins

| Priority | Initiative | Outcome / metric |
|----------|------------|------------------|
| P0 | **Admin & API completeness** — Document and harden control plane + ops endpoints; single “enterprise operations” doc (reload, HA assumptions, SSE/failover). | New admins succeed without reading source |
| P0 | **Security breadth** — HMAC, Basic, OIDC gaps vs JWT-only where needed; policy matrix in docs | Parity with common API gateway auth lists |
| P1 | **Plugin pipeline** — PDK + 3–5 **curated** plugins with semver, CI build, signing story (even if minimal) | Reproducible Wasm releases |
| P1 | **OpenAPI → MCP** — CLI or doc pipeline (evaluate reuse vs [openapi-to-mcpserver](https://github.com/higress-group/openapi-to-mcpserver)) | Covers typical OpenAPI 3.x services |
| P2 | **Distribution** — Multi-region image push + documented Helm/K8s values (align with existing deploy/) | Predictable installs in 3+ regions |

### Phase 2 — Platform (3–6 months): console + discovery (optional) + production semantics

| Priority | Initiative | Outcome / metric |
|----------|------------|------------------|
| P0 | **Web console** — Config, routes, MCP servers, key rotation UX; SSO alignment with README claims | Feature parity with “day-2 ops” tasks |
| P1 | **Config storage** — Durable source of truth for console (etcd/DB/K8s secrets—choose one model) | Auditable changes |
| P1 | **Hot / fast config apply** — Define targets (e.g. &lt;500 ms for route changes where applicable) and measure | Documented SLO for config |
| P2 | **Service discovery** — One registry first (e.g. **Nacos** or **Consul**) if product commits to microservice gateway | Opt-in; no impact on standalone users |
| P2 | **MCP hosting story** — Clear model: in-process tools vs external MCP vs Wasm; lifecycle & health | Matches Higress “hosting” narrative where applicable |

### Phase 3 — Enterprise hardening (6–12 months): security + scale + tenant model

| Priority | Initiative | Outcome / metric |
|----------|------------|------------------|
| P1 | **WAF / L7 rules** — Wasm or embedded ruleset; start with OWASP Top 10-style coverage | Measurable block rate + false-positive process |
| P1 | **ACME / cert lifecycle** — Let’s Encrypt + rotation | Hands-off TLS at the edge |
| P2 | **Multi-tenant isolation** — Namespaces, budgets, audit per tenant | Isolation guarantees documented |
| P2 | **Community & ecosystem** — Public roadmap updates, contributor onboarding, plugin hackability | Sustainable external contributions |

---

## Design sketches (unchanged ideas, tighter scope)

### 1. Plugin ecosystem

- **Registry**: GitHub/OCI-backed artifact list + metadata (not necessarily a full “store”).
- **PDK**: Rust/Go as today; document **ABI/version** compatibility.
- **Sandbox**: Wasm isolation; define **resource limits** for enterprise deals.

### 2. MCP management

- **Tooling**: OpenAPI → MCP server or tool definitions aligned with Panda’s MCP config.
- **Operations**: Health, metrics, and audit for tool calls (Higress emphasizes audit; Panda’s compliance JSONL is a differentiator—keep it).

### 3. Console

- **Backend**: Reuse control plane APIs; add **RBAC**, **change audit**, **dry-run** validation.
- **Frontend**: Stack-agnostic; prefer alignment with your existing `panda_site` or a dedicated app repo.

### 4. Deployment

- **Helm**: Values for secrets, ingress, resources, **pod disruption budgets**.
- **Standalone**: Docker Compose remains the quick path (like Higress all-in-one).

### 5. Service discovery (optional track)

- **Adapter interface** inside Panda for upstream resolution from Nacos/Consul/Eureka.
- **Cache + backoff** to avoid thundering herds on registry outages.

### 6. Security

- **Auth**: Unified **AuthenticationManager** concept in docs and config.
- **WAF**: Start with a **small** rule set; expand with Wasm modules.

---

## Success metrics (realistic)

| Category | Example target | Notes |
|----------|----------------|-------|
| Operations | Documented RTO/RPO for config and control plane | Enterprise sales enablement |
| Performance | P95 proxy overhead vs baseline | From `benchmarks/`; publish |
| Reliability | Measured uptime in reference K8s deployment | 99.9%+ achievable before claiming 99.99% |
| Ecosystem | N **curated** plugins with CI | Quality over raw count |
| Security | CVE response SLA | Publish in `SECURITY.md` |

Avoid fixing **contributor count** or **plugin count** as primary KPIs unless you actively invest in community growth.

---

## Risks and non-goals

- **Scope creep:** Implementing full **Ingress/Gateway API** parity is a multi-year product; declare in or out early.
- **Architecture mismatch:** Copying Envoy plugin APIs verbatim may not fit Panda’s host—define a **Panda plugin contract** and migrate gradually.
- **Duplicate OSS:** Prefer **contributing** or **wrapping** OpenAPI-to-MCP tools before rewriting.

---

## Suggested next step (for planning)

1. **Decide product wedge:** “AI+MCP gateway for teams already on K8s” vs “standalone edge AI proxy” vs both (different tracks).
2. **Produce a one-page gap list** from this doc with **owners** and **Q priorities**.
3. **Spike:** Console MVP on existing control plane + OpenAPI→MCP evaluation in parallel with one **security** item (e.g. OIDC depth or ACME).

---

## References

- Higress: https://github.com/alibaba/higress  
- Panda README: `README.md`  
- Higress README (local): `../higress/README.md`  
- Panda test/data-flow map: `docs/testing_scenarios_and_data_flows.md`

---

## Document history

- **Refined:** Compared Higress positioning and Panda’s actual capabilities; added positioning table, gaps, dependencies, risks, and realistic metrics; phased priorities reordered for enterprise value.
