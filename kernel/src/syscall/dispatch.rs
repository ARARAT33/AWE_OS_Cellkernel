#![no_std]

use super::abi::{ERR_INVALID_ARGUMENT, ERR_NOT_IMPLEMENTED, Syscall, SyscallResult};
use crate::process::ProcessDescriptor;

/// Kernel-side syscall context placeholder.
///
/// This type is retained as an internal integration seam, but it is not a
/// trusted syscall context: it contains neither an authoritative current-task
/// token nor a validated user address space. Consequently, dispatch accepts
/// no general syscall yet and must fail closed for every known operation.
pub struct SyscallContext<'a> {
    pub process: &'a mut ProcessDescriptor,
}

impl<'a> SyscallContext<'a> {
    /// Validate the syscall number, then reject known operations until their
    /// process, capability, scheduling and user-memory contracts are real.
    ///
    /// In particular, arguments are never interpreted as kernel pointers.
    pub fn dispatch(&mut self, number: u64, _args: [u64; 6]) -> SyscallResult {
        match number {
            value if value == Syscall::Yield as u64 => {}
            value if value == Syscall::Exit as u64 => {}
            value if value == Syscall::Spawn as u64 => {}
            value if value == Syscall::IpcSend as u64 => {}
            value if value == Syscall::IpcRecv as u64 => {}
            value if value == Syscall::Map as u64 => {}
            value if value == Syscall::Unmap as u64 => {}
            value if value == Syscall::Read as u64 => {}
            value if value == Syscall::Write as u64 => {}
            _ => return err(ERR_INVALID_ARGUMENT),
        }

        // Do not mutate the borrowed descriptor or consume quotas here.
        // The real dispatcher must operate through the authoritative process
        // manager and validated address-space/capability interfaces.
        err(ERR_NOT_IMPLEMENTED)
    }
}

const fn err(error: u64) -> SyscallResult {
    SyscallResult { value: 0, error }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{ProcessId, ProcessState, ResourceBudget};

    fn descriptor() -> ProcessDescriptor {
        ProcessDescriptor {
            id: ProcessId(1),
            state: ProcessState::Running,
            budget: ResourceBudget {
                cpu_ticks: 10,
                memory_bytes: 4096,
                ipc_messages: 2,
            },
        }
    }

    #[test]
    fn invalid_call_is_rejected() {
        let mut process = descriptor();
        let mut context = SyscallContext {
            process: &mut process,
        };

        let result = context.dispatch(99, [0; 6]);

        assert_eq!(result.value, 0);
        assert_eq!(result.error, ERR_INVALID_ARGUMENT);
    }

    #[test]
    fn every_known_syscall_fails_closed_without_side_effects() {
        let mut process = descriptor();
        let before = process;
        let mut context = SyscallContext {
            process: &mut process,
        };
        let calls = [
            Syscall::Yield,
            Syscall::Exit,
            Syscall::Spawn,
            Syscall::IpcSend,
            Syscall::IpcRecv,
            Syscall::Map,
            Syscall::Unmap,
            Syscall::Read,
            Syscall::Write,
        ];

        for call in calls {
            // These values would have been dangerous if interpreted as
            // unchecked pointers or lengths by the old Read/Write handlers.
            let args = match call {
                Syscall::Read | Syscall::Write => [1, u64::MAX, 0, 0, 0, 0],
                _ => [u64::MAX; 6],
            };
            let result = context.dispatch(call as u64, args);

            assert_eq!(result.value, 0, "{call:?} must not report fake success");
            assert_eq!(
                result.error, ERR_NOT_IMPLEMENTED,
                "{call:?} must remain unavailable until safely integrated"
            );
            assert_eq!(
                *context.process, before,
                "{call:?} must not mutate process state or resource budgets"
            );
        }
    }
}
