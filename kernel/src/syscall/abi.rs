#![no_std]

#[repr(u64)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Syscall {
    Yield = 0,
    Exit = 1,
    Spawn = 2,
    IpcSend = 3,
    IpcRecv = 4,
    Map = 5,
    Unmap = 6,
    Read = 7,
    Write = 8,
}

#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SyscallResult {
    pub value: u64,
    pub error: u64,
}

pub const ERR_OK: u64 = 0;
pub const ERR_INVALID_ARGUMENT: u64 = 1;
pub const ERR_PERMISSION: u64 = 2;
pub const ERR_NOT_FOUND: u64 = 3;
pub const ERR_BUSY: u64 = 4;
pub const ERR_NO_MEMORY: u64 = 5;
/// The syscall number is recognized, but its runtime contract is not ready.
pub const ERR_NOT_IMPLEMENTED: u64 = 6;
