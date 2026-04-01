# Compliance export (audit trail)

Panda can emit an **append-only, optionally signed** audit stream for governance regimes that require demonstrable traceability of AI traffic (for example EU AI Act–style documentation: who invoked what, when, and under which route).

**Current implementation:** a **local JSONL** sink under `observability.compliance_export` (see `panda.example.yaml`). Each line is a JSON object with a `schema` discriminator:

- **`panda.compliance.ingress.v1`** — correlation id, path, method; optional `request_body_sha256_hex` when the gateway buffered the request body (same bytes as received, before middleware mutation).
- **`panda.compliance.egress.v1`** — status code; optional `response_body_sha256_hex` when a full response snapshot exists (e.g. buffered JSON or semantic-cache hit); `response_streamed: true` when no snapshot was taken (SSE / streaming / pass-through).

**Object storage (S3 / GCS)** and richer fields (subject, tool ids) are specified below as the target architecture.

## Configuration (today)

| YAML field | Purpose |
|------------|---------|
| `enabled` | When true, the proxy opens the sink at startup. |
| `mode` | Must be `local_jsonl` when enabled (validation in `panda-config`). |
| `local_path` | Directory; records append to `panda-compliance.jsonl`. |
| `signing_secret_env` | Optional env var name; if set and non-empty in the environment, each line gets `hmac_sha256_hex` over the canonical JSON **without** the `hmac` field. |

Ops: `GET /compliance/status` (same optional admin auth as `/metrics`, `/tpm/status`, …) returns JSON describing config and whether signing is active.

Implementation sketch: `crates/panda-proxy/src/compliance_export.rs`.

## Target pipeline (design)

1. **Capture** — For each request/response (or stream chunk summary), record at minimum: correlation id, timestamps, authenticated subject/tenant (if any), ingress path, model/route id, **hashes** of prompt and completion payloads (never store raw secrets by default), token estimates, tool-call ids, and MCP server/tool names.
2. **Canonicalize** — Serialize a stable JSON object (sorted keys, fixed types) before signing.
3. **Sign** — HMAC-SHA256 with a **rotation-friendly** secret (KMS-wrapped material in production); consider moving to asymmetric keys (Ed25519) for verifier-only consumers.
4. **Ship** — A sidecar or batch worker tails the local file (or reads from a shared volume) and uploads to **S3** or **GCS** using **versioning + Object Lock / bucket retention (WORM)** so objects cannot be silently rewritten.
5. **Verify** — Offline job recomputes HMAC or signature over canonical bytes and checks object integrity; alert on mismatch.

## Operational notes

- **Immutability** is a property of the **bucket policy + Object Lock**, not of Panda alone.
- **PII:** default to hashes + redacted excerpts; expand only under explicit policy.
- **Rotation:** dual-sign during rollover, or version the `schema` field per record.

This doc is the contract for future crates or jobs that implement the object-store writers without changing the on-proxy JSONL format more than necessary.
