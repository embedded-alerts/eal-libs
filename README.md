# eal-libs

Shared, runtime-light libraries for **Embedded Alerts**.

- `crates/contracts` — stable event, actor, and request metadata contracts.
- `crates/routing` — deterministic partitioning and priority classification.
- `crates/semantic` — canonical URL normalization, SHA-256 identities, lexical and
  exact-space semantic scoring, cooldown decisions, and retry/idempotency helpers.
- `src/` — JavaScript reference implementation for Workers and web tooling.
- `schemas/` — JSON Schema documents for language-neutral validation.

The Rust crates use only the standard library. This keeps the core matching behavior
portable and auditable across `eal-api`, `eal-sync`, Mash/HTMX, Leptos, Dioxus, CLI,
and generated clients.

## Matching contract

A URL is accepted only with `http` or `https`, cannot contain user information, has
its host and default port normalized, removes the fragment, resolves literal dot
segments, sorts query pairs, and drops common tracking parameters. Network safety is
a separate ingestion concern: `eal-sync` must still resolve DNS, reject private and
reserved destinations, and repeat that check after every redirect.

Semantic vectors are comparable only when provider, model, model version,
dimensions, and normalization strategy match exactly. Cosine values below zero are
clamped to zero for alert scoring; they are never remapped into an apparently
positive match.

Lexical scoring uses normalized unique query-term coverage plus required and excluded
phrases. A rule combines lexical and semantic scores with weights that must be finite,
within `[0, 1]`, and sum to one.

The canonical logical match identity is SHA-256 over tenant, immutable rule revision,
immutable source revision, embedding-space ID (or `lexical-only`), and normalized
content hash. This mirrors the PostgreSQL contract in `eal-interfaces` and lets
concurrent workers collapse duplicate work before any notification is sent.

Cooldown suppression retains the match evidence while deferring delivery. Provider
attempts use deterministic idempotency keys and capped exponential backoff; reaching
the configured attempt limit produces a dead-letter decision rather than rewriting
prior receipts.

## Validation

```bash
./scripts/test.sh
```

The script runs JavaScript tests, validates JSON, then runs Rust formatting, clippy,
and all workspace tests when Cargo is available.
