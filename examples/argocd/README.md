# ArgoCD examples

This folder contains example, declarative ArgoCD `Application` manifests for the Stellar-K8s repository.

- `application-stellar-validator.yaml` — An `Application` that deploys the `charts/stellar-operator` Helm chart.
- `application-soroban-rpc.yaml` — An `Application` that deploys the `charts/soroban-rpc` Helm chart.

Usage

1. Add this repository URL to your ArgoCD (Settings → Repositories).
2. Apply one of these `Application` YAMLs into the namespace where ArgoCD watches (commonly `argocd`).
3. Sync the Application from the ArgoCD UI.

Notes

- The manifests are intentionally declarative and include `syncPolicy.automated.prune=true` and `PruneLast=true` to reduce orphaned resources during deletion. See the docs page for finalizer guidance.
