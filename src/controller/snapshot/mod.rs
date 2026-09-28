//! Multi-Cluster Snapshot Synchronization subsystem.
//!
//! Provides two submodules:
//!
//! - [`verifier`]: SHA-256 integrity checking for snapshot archives
//! - [`reconciler`]: automated download, verify, extract, and bootstrap loop
//!
//! # Overview
//!
//! Secondary cluster nodes can be bootstrapped from recent ledger snapshots
//! stored in S3-compatible cloud storage. The reconciler discovers the latest
//! archive, streams it to disk, verifies its integrity via SHA-256, extracts
//! it atomically, and marks the node as bootstrapped via a sentinel file.

pub mod reconciler;
pub mod verifier;

pub use reconciler::{ReconcileOutcome, SnapshotReconciler, SnapshotReconcilerConfig, SnapshotRef};
pub use verifier::{
    compute_sha256_sync, parse_sha256_sidecar, verify_file, VerificationResult,
};
