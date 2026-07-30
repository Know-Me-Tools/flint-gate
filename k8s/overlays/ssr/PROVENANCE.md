# SSR candidate provenance

- Source repository: `git@github.com:Know-Me-Tools/flint-gate.git`
- Baseline source SHA: `f1113c28323eec18bd2811f8236ac67e80c23f63`
- Candidate source SHA: `a5fd1fc40e3f07c63afa474c15710b41525da578`
- License: MIT
- Build input: repository `Dockerfile`, production defaults, `linux/amd64`;
  `rust:1.94-bookworm@sha256:6ae102bdbf528294bc79ad6e1fae682f6f7c2a6e6621506ba959f9685b308a55`
  and
  `debian:bookworm-slim@sha256:7b140f374b289a7c2befc338f42ebe6441b7ea838a042bbd5acbfca6ec875818`
- Candidate image: `ghcr.io/know-me-tools/flint-gate`
- Candidate digest:
  `sha256:e67abd80b024cd85dd1cbb2615ab940510aa3b48b48c86454b32c5a646398511`
  ([candidate workflow run 30538473672](https://github.com/Know-Me-Tools/flint-gate/actions/runs/30538473672)).
- PostgreSQL: `postgres:16.14-alpine3.24@sha256:57c72fd2a128e416c7fcc499958864df5301e940bca0a56f58fddf30ffc07777`
- AKS storage class: `managed-csi` (`disk.csi.azure.com`, expansion enabled),
  verified against `kubectl --context ssr get storageclass` on 2026-07-30.

The overlay references Secret objects but never renders their values:
`flint-gate-database` (`url`, `username`, `password`, `database`) and
`flint-gate-signing-key` (`private.pem`, `public.pem`).
Private GHCR pulls reference the pre-created `ghcr-pull` image pull Secret.
