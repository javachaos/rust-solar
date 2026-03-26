//! Terminal application and test harnesses for a solar MPPT controller.
//!
//! The crate is split into a few focused areas:
//!
//! - `app` owns terminal UI rendering, input handling, and thread orchestration.
//! - `serial_data_logger` defines the controller transport boundary and the real serial adapter.
//! - `controller_simulator` provides an in-process controller simulation and byte-stream wrapper.
//! - `pty_controller_harness` exposes the simulator through a real PTY on Unix for end-to-end tests.
//! - `database` persists parsed datapoints into SQLite with buffered writes.
//! - `datapoint` parses and formats the controller's colon-delimited telemetry frames.
//!
//! The normal binary entrypoint is [`run`]. Unix builds also export
//! [`PtyControllerHarness`] so tests and local tools can exercise the full serial path
//! without real hardware.

mod app;
mod controller_simulator;
mod database;
mod datapoint;
mod load_toggle_switch;
#[cfg(unix)]
mod pty_controller_harness;
mod serial_data_logger;

/// Shared application result type used by the binary entrypoint.
pub use app::{run, AppResult};
#[cfg(unix)]
/// Unix-only PTY-backed controller simulator for end-to-end testing and manual smoke runs.
pub use pty_controller_harness::{PtyControllerHarness, PTY_PORT_PREFIX};
