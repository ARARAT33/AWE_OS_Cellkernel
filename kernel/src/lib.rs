#![no_std]
#![allow(dead_code)]
#![allow(unused_attributes)]
#![allow(clippy::missing_safety_doc)]
#![allow(clippy::len_without_is_empty)]
#![allow(clippy::module_inception)]

// CellKernel is intentionally hardware-driver free.
// All hardware discovery, driver lifecycle, compatibility adapters and
// VirtIO/Linux/Windows/Android driver execution live in services/driverd.
// The kernel owns only minimal process/IPC/capability primitives needed to
// isolate and communicate with those services.
pub mod ac_boot_gate;
pub mod ac_completion;
pub mod ac_runtime;
pub mod ai;
pub mod aosin;
pub mod arch;
pub mod boot_guard;
pub mod boot_phase;
pub mod boot_state;
pub mod cell;
pub mod compat;
pub mod continuum;
pub mod device;
pub mod drivers;
pub mod engineering_contracts;
pub mod entry;
pub mod execution_core;
pub mod formats;
pub mod interrupts;
pub mod ipc;
pub mod logging;
pub mod memory;
pub mod net;
pub mod platform;
pub mod process;
pub mod runtime;
pub mod scheduler;
pub mod security;
pub mod service;
pub mod service_registry;
pub mod storage;
pub mod syscall;
pub mod system_contract;
pub mod time;
