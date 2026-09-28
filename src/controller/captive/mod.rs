//! Captive Core process supervision and IPC health checks.

pub mod ipc;
pub mod supervisor;

pub use supervisor::{CaptiveCoreSupervisor, SupervisorConfig};
