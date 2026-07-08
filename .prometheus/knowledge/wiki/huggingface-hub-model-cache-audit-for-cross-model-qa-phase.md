---
type: Reference
id: huggingface-hub-model-cache-audit-for-cross-model-qa-phase
title: HuggingFace hub model cache audit for cross-model QA phase
tags:
- huggingface-cache
- model-cache
- disk-cleanup
- cross-model-qa
- local-environment
links:
- pr-27-constant-time-bearer-auth-hardening
sources:
- stdin
timestamp: 2026-07-04T14:44:39.331261+00:00
created_at: 2026-07-04T14:44:39.331261+00:00
updated_at: 2026-07-04T14:44:39.331261+00:00
revision: 0
---

## Context

- **Position:** `phase-ci-cross-model-qa-and-hardening`
- **Status:** `reflect_complete`
- **HuggingFace cache root:** `/Users/gqadonis/.cache/huggingface/hub`
- **HF_HOME override:** none detected; standard HuggingFace Hub cache location is in use.
- **Total cached model data:** 25 GB across 4 models.
- **Cached datasets:** none.

This cache audit belongs to the same cross-model QA and hardening phase as [PR 27 constant-time bearer auth hardening](/pr-27-constant-time-bearer-auth-hardening.md).

## Cached models

| Model | Size | Notes |
|---|---:|---|
| `unsloth/gemma-4-E4B-it` | 15 GB | Largest reclaimable item. |
| `unsloth/gemma-4-12b-it-GGUF` | 7.0 GB | GGUF model cache. |
| `unsloth/Qwen3.5-4B-MTP-GGUF` | 2.8 GB | GGUF model cache. |
| `BAAI/bge-small-en-v1.5` | 128 MB | Embedding model. |

## Non-model cache findings

- A sibling `xet/` chunk-dedup cache exists but is negligible at 2.3 MB.
- `~/chat-ui/node_modules/@huggingface/hub` appeared during search, but it is the JavaScript SDK package, not a model cache. It should be ignored for model-cache cleanup.

## Cleanup options

- Preferred interactive cleanup: `huggingface-cli delete-cache`.
- Direct cleanup is also possible by removing the specific `models--*` directory for the target model.
- Highest-impact reclaim candidate: `unsloth/gemma-4-E4B-it` at 15 GB.

## Current next step

No cleanup was performed. Awaiting decision on whether any cached models should be pruned.

# Citations

1. stdin