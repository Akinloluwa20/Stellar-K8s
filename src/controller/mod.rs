

pub mod autoscaler;
pub mod benchmark;
pub mod blue_green;

pub mod canary;
pub mod cross_cloud_failover;
pub mod event_taxonomy;
pub mod feature_flags;
pub mod gas_autoscaling;
pub mod gitops_upgrade;
pub mod horizon_cache;
pub mod horizon_metrics_collector;
pub mod horizon_scaler;
pub mod jurisdiction;
pub mod label_propagation;
pub mod leader;
pub mod maintenance;
pub mod migration;
pub mod ml_pipeline;
pub mod network_isolation;
pub mod observability_pipeline;
pub mod phases;
pub mod predictive_scaling;
pub mod pdb;
pub mod pss;
pub mod quota;
pub mod registry_controller;
pub mod resource_meta;


pub mod anomaly_detection;
pub(crate) mod archive_health;
pub mod archive_prune;
pub mod audit;
pub mod audit_log;
pub mod audit_recorder;
pub mod audit_sink;
pub mod audit_worker;
pub mod background_jobs;
pub mod captive;
pub mod captive_core;
pub mod chaos_engineering;
pub mod compliance_export;
pub mod conditions;
pub mod cost;
pub mod cross_cluster;
pub mod cross_region_sync;
pub mod cve;
pub(crate) mod cve_reconciler;
pub mod cve_scanner;
[cfg(test)]
pub(crate) mod cve_test;
pub mod db_pool;
pub mod diff;
pub mod disk_scaler;
[cfg(test)]
mod disk_scaler_test;
pub mod dr;
pub mod dr_drill;
[cfg(test)]
mod dr_test;
pub(crate) mod finalizers;
pub(crate) mod forensic_snapshot;
pub(crate) mod health;

mod health_test;
pub mod ingestion;
pub mod kms_secret;
[cfg(feature = "metrics")]
pub mod metrics;
pub mod mtls;
pub mod mtls_rotation;
pub mod oci_snapshot;
pub mod operator_config;
pub mod peer_discovery;
[cfg(test)]
mod peer_discovery_test;
pub mod pruning_reconciler;
pub mod pruning_worker;
pub mod quorum;
pub mod read_pool;
pub(crate) mod reconciler;
[cfg(test)]
mod reconciler_test;
pub(crate) mod remediation;
[cfg(test)]
mod remediation_test;
pub mod resource_optimization;
pub(crate) mod resources;
[cfg(test)]
mod resources_test;
pub mod rollout;
pub mod secret_watcher;
pub mod security;
pub mod service_mesh;
mod csi_snapshot;
pub mod snapshot;
pub mod snapshot_worker;
pub mod spot_drain;
pub mod storage_migration;
pub(crate) mod sync_scale;
pub(crate) mod sync_state_monitor;

pub mod topology;
pub mod traffic;
[cfg(test)]
mod traffic_test;
pub mod vpa;
pub(crate) mod vsl;
pub mod webhook_delivery;
pub mod zk_archive_verifier;

pub use anomaly_detection::{run_anomaly_detection, AnomalyDetector, AnomalyEvent};
pub use archive_health::{
    calculate_backoff, check_archive_integrity, check_history_archive_health, ArchiveHealthResult,
    ArchiveIntegrityResult, ARCHIVE_LAG_THRESHOLD,
};
pub use audit_log::{AdminAction, AuditEntry, AuditLog};
pub use audit_recorder::AuditRecorder;
pub use background_jobs::{JobKind, JobRecord, JobRegistry, JobState, MAX_JOBS};
pub use captive::{CaptiveCoreProcess, CaptiveCoreSupervisor, SupervisorConfig, SupervisorState};
pub use benchmark::run_benchmark_controller;
pub use blue_green::{
    cleanup_blue_deployment, create_green_deployment, rollback_to_blue, run_smoke_tests,
    switch_traffic_to_green, wait_for_green_ready, BlueGreenConfig, BlueGreenStatus,
};
pub use blue_green_core::{
    evaluate_cutover_gate, may_switch_service_to_green, plan_cutover_advance,
    plan_rollback_advance, reconcile_validator_blue_green, should_take_over_validator_workload,
    storage_identities, CoreBlueGreenPhase, CutoverCommand, CutoverGateResult, CutoverStep,
    RollbackCommand, RollbackStep, COLOR_BLUE, COLOR_GREEN, COLOR_LABEL,
};
pub use cache_aware_queue::{
    calculate_cache_aware_backoff, priority_from_signals, CacheAwareBackoffInput,
    CacheAwarePriorityQueue, ReconcilePriority,
};
pub use cross_cloud_failover::reconcile_cross_cloud_failover;
pub use cross_cluster::{check_peer_latency, ensure_cross_cluster_services, PeerLatencyStatus};
pub use cve_reconciler::reconcile_cve_patches;
pub use cve_scanner::{
    list_vulnerable_pods, register_cve_metrics, spawn_background_scanner, CveScannerConfig,
    PodScanSummary,
};
pub use db_pool::{
    create_pool, DbPoolConfig, DEFAULT_CONNECTION_TIMEOUT_SECS, DEFAULT_MAX_CONNECTIONS,
};
pub use disk_scaler::{
    check_and_expand, get_disk_usage, supports_expansion, DiskScalerConfig, DiskUsage,
    ScalingResult, DEFAULT_EXPANSION_INCREMENT, DEFAULT_EXPANSION_THRESHOLD,
};
pub use event_taxonomy::{EventAction, EventCategory, EventDescriptor, EventReason};
pub use feature_flags::{
    watch_feature_flags, FeatureFlags, SharedFeatureFlags, FEATURE_FLAGS_CONFIGMAP,
};
pub use finalizers::STELLAR_NODE_FINALIZER;
pub use gitops_upgrade::{
    GitOpsEngine, GitOpsUpgradeController, GitOpsUpgradePlan, ProtocolUpgradeStep,
    ProtocolUpgradeTimeline,
};
pub use health::{check_node_health, HealthCheckResult};
pub use jurisdiction::{
    build_jurisdiction_node_affinity, compliance_report, merge_jurisdiction_tolerations,
    ComplianceReportEntry,
};
pub use migration::{
    HorizonToSorobanMigrationController, MigrationConfig, MigrationPhase, MigrationState,
    MIGRATE_TO_ANNOTATION,
};
pub use network_isolation::{
    check_network_safety, network_label_value, same_network_namespace_selector,
    NetworkSafetyViolation, NAMESPACE_NETWORK_LABEL, NODE_NETWORK_LABEL,
};
pub use operator_config::{hardcoded_defaults, OperatorConfig};
pub use peer_discovery::{
    get_peers_from_config_map, trigger_peer_config_reload, PeerDiscoveryConfig,
    PeerDiscoveryManager, PeerInfo,
};
pub use pruning_reconciler::{reconcile_pruning, update_pruning_status};
pub use pss::{
    ensure_namespace_pss_labels, restricted_container_security_context,
    restricted_pod_security_context, validate_pss_compliance, PssViolation,
};
[cfg(feature = "reconciler-fuzz")]
pub use reconciler::reconcile_for_fuzzz;
pub use reconciler::{run_controller, BatchSummaryReport, ControllerState};

pub use service_mesh::{
    delete_service_mesh_resources, ensure_destination_rule, ensure_peer_authentication,
    ensure_request_authentication, ensure_virtual_service,
};
pub use snapshot::{
    verify_file as snapshot_verify_file, ReconcileOutcome, SnapshotReconcilerConfig, SnapshotRef,
};
pub use snapshot_worker::run_snapshot_worker;
pub use webhook_delivery::{
    DeliveryRecord, WebhookDeliveryService, WebhookEndpoint, WebhookEvent, WebhookEventType,
};

// Topology enforcement (issue #115)
pub use topology::{
    build_statefulset_patch, discover_cluster_topology, enforce_namespace, enforce_on_statefulset,
    ClusterTopology, EnforcementResult, TopologyMode, TopologyRuleSet, TopologySpreadConstraint,
    WhenUnsatisfiable,
};
