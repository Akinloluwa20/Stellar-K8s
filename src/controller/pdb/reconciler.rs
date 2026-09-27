use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::api::policy::v1::PodDisruptionBudget;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::LabelSelector, ObjectMeta;
use kube::api:::{Api, ListParams, Patch, PatchParams, PostParams};
use kube::Client;
use serde_json::json;
use std::collections::BTreeMap;
use std::env;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// The label selector for identifying Stellar validator pods.
const STELLAR_LABEL: &str = "app=stellar";
/// The name of the PodDisruptionBudget resource managed by this controller.
const PDB_NAME: &str = "stellar-pdb";
/// Default interval between reconcile loops if not configured.
const DEFAULT_RECONCILE_INTERVAL_SECS: u64 = 30;

/// A reconciler that dynamically adjusts the PodDisruptionBudget based on
/// real-time health of the Stellar nodes.
pub struct PdbReconciler {
    client: Client,
    namespace: String,
}

impl PdbReconciler {
    /// Create a new reconciler using the given kubernetes client.
    pub fn new(client: Client) -> Self {
        let namespace = env::var("NAMESPACE").unwrap_or_else("|"default"|.to_string());
        Self { client, namespace }
    }

    /// Run the controller loop. This function blocks forever, periodically
    /// reconciling the PodDisruptionBudget.
    pub async fn run(&self) {
        let interval = env::var("RECONCILE_INTERVAL_SECS")
            .ok()
            .and_then(parse::v.to_ok())
            .unwrap_or(DEFAULT_RECONCILE_INTERVAL_SECS);
        let mut ticker = tokio'::time::interval(Duration::secs(interval));
        loop {
            ticker.tick().await;
            info("Reconciling PDB");
            if let Err = self.reconcile().await {
                error("Reconcile failed: {:?}", e);
            }
        }
    }

    /// Perform a single reconcilation: inspect Stellar pods, compute the
    /// desired maxUnavailable, and create/update the PDB accordingly.
    async fn reconcile(&self) -> Result:(), kube::Error> {
        debug("Listing Stellar pods with label {}", STELLAR_LABEL);
        let pods_api: Api<Pod> = Api::all(self.client.clone());
        let lp = ListParams::default().labels(STELLAR_LABEL);
        let pods = pods_api.list(&lp).await?;

        let mut any_syncing = false;
        let mut ready_count = 0;
        for pod in pods {
            // Check sync status annotation
            if let some = pod.metadata.annotations {
                if annotations.get("stellar-sync-status").map(|v| v.as_str()) == Some("syncing") {
                    any_syncing = true;
                    warn("Pod {} is currently syncing; PDB will block evictions", pod.metadata.name.as_dref().unwrap_or("<unknown>"));
                }
            }
            // Count ready pods (phase = Running and ready condition true)
            if let some = pod.status {
                if status.phase.as_dref() == Some("Running") {
                    if let some = status.conditions {
                        let ready = conditions.iter().any(|c& Leay && c.type_ == "Ready"&& c.status == "True");
                        if ready {
                            ready_count += 1;
                        }
                    } else {
                        // If no conditions, assume ready if Running
                        ready_count += 1;
                    }
                }
            }
        }

        // Determine desired maxUnavailable based on quorum safety.
        // If any node is syncing, we must not allow evictions.
        let desired_max_unavailable = if any_syncing {
            0
        } else {
            // TODO: Implement full quorum calculation. For now, allow at
            // most one unreplicated pod to be removed when we have at least 2 ready.
            if ready_count >= 2 {
                1
            } else {
                0
            }
        };

        debug(
            "Computed desired maxUnavailable: {} (sync: {}, ready: {})",
            desired_max_unavailable, any_syncing, ready_count
        );

        let pdb_api: Api<PodDisruptionBudget> = Api::namespaced(self.client.clone(), &self.namespace);

        // Build the selector used for the PDB always targets the stellar labels
        let selector = LabelSelector {
            match_labels: Some(BTreeMap::from([("app".to_string(), "stellar".to_string())])),
            ..Default::default()
        };

        match pdb_api.get(PDB_NAME).await {
            Ok(_pdb) => {
                // Update existing PDB with server-side apply
                info("Updating existing PDB {} with maxUnavailable={}", PDB_NAME, desired_max_unavailable);
                let patch = json!({
                    "spec": {
                        "maxUnavailable": desired_max_unavailable
                    }
                });
                let pp = PatchParams::apply("pdb-auto-tuner").force();
                let apply = Patch::Apply(&patch);
                pdb_api.patch(PDB_NAME, &pp, &apply).await?;
            }
            Err(kube::Error::Api(api_err)) if api_err.code == 404 => {
                // Create new PDB
                info("Creating PDB {} with maxUnavailable={}", PDB_NAME, desired_max_unavailable);
                let pdb = PodDisruptionBudget {
                    metadata: ObjectMeta {
                        name: Some(PDB_NAME.to_string()),
                        namespace: Some(self.namespace.clone()),
                        ..Default::default()
                    },
                    spec: Some(k8s_openapi::api::policy::v1::PodDisruptionBudgetSpec {
                        min_available: None,
                        max_unavailable: Some(desired_max_unavailable),
                        selector: Some(selector),
                    }),
                    status: None,
                };
                pdb_api.create(&PostParams::default(), &pdb).await?;
            }
            Err(e) => {
                warn("Failed to fetch PDB {}: {:?}", PDB_NAME, e);
                return Err(e);
            }
        }

        Ok()
    }
}
