//! Deterministic FeatherMark build and evidence driver.

#[allow(dead_code)] // Task 1C wires the capability boundary to real application launchers.
mod app_launch;
#[allow(dead_code)] // Task 1C wires the capability boundary to real scenario owners.
mod candidate;
pub mod comparator;
pub mod fixtures;
pub mod gui;
pub mod metrics;
pub mod package;
pub mod runner;
mod tool_process;
