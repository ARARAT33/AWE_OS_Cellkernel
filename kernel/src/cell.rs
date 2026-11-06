#![no_std]

//! Bounded, local cell lifecycle and IPC runtime.
//!
//! A cell is a kernel-managed identity bound to an existing ProcessManager
//! process. This module does not create an address space or prove memory
//! isolation; it enforces lifecycle, admission, quota-accounting and local
//! message-boundary rules around the process/IPC primitives that already exist.

use crate::process::{
    ProcessId, ProcessManager, ProcessManagerError, ProcessState, ResourceBudget,
};
use crate::system_contract::{CapabilitySet, KernelCapability};

pub const CELL_ABI_MAJOR: u16 = 1;
pub const CELL_ABI_MINOR: u16 = 0;
pub const MAX_CELL_DEPENDENCIES: usize = 8;
pub const MAX_CELL_INTERFACES: usize = 8;

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellId(pub u64);

#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CellChannelId(pub u64);

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellKind {
    KernelCore = 0,
    Memory = 1,
    Process = 2,
    Scheduler = 3,
    Driver = 4,
    Storage = 5,
    Network = 6,
    Security = 7,
    Service = 8,
    UserInterface = 9,
    Application = 10,
    Virtualization = 11,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellState {
    Declared = 0,
    Admitted = 1,
    Starting = 2,
    Running = 3,
    Stopping = 4,
    Stopped = 5,
    Failed = 6,
    Quarantined = 7,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellHealth {
    Unknown = 0,
    Healthy = 1,
    Degraded = 2,
    Unresponsive = 3,
    Faulted = 4,
    Stopped = 5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellQuota {
    pub cpu_ticks: u64,
    pub memory_bytes: u64,
    pub ipc_messages: u64,
}

impl CellQuota {
    pub const fn new(cpu_ticks: u64, memory_bytes: u64, ipc_messages: u64) -> Self {
        Self {
            cpu_ticks,
            memory_bytes,
            ipc_messages,
        }
    }

    pub const fn unlimited() -> Self {
        Self::new(u64::MAX, u64::MAX, u64::MAX)
    }

    pub const fn fits_process_budget(self, budget: ResourceBudget) -> bool {
        budget.permits_cpu(self.cpu_ticks)
            && budget.permits_memory(self.memory_bytes)
            && budget.permits_ipc(self.ipc_messages)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellUsage {
    pub cpu_ticks: u64,
    pub memory_bytes: u64,
    pub ipc_messages: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellInterface {
    pub id: u32,
    pub abi_major: u16,
    pub abi_minor: u16,
}

impl CellInterface {
    pub const fn new(id: u32, abi_major: u16, abi_minor: u16) -> Self {
        Self {
            id,
            abi_major,
            abi_minor,
        }
    }

    pub const fn is_valid(self) -> bool {
        self.id != 0 && self.abi_major != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRecoveryPolicy {
    /// A cell may be rebound to a fresh process after at most this many
    /// failures. The next failure moves it to Quarantined.
    pub max_restarts: u8,
}

impl CellRecoveryPolicy {
    pub const fn new(max_restarts: u8) -> Self {
        Self { max_restarts }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSpec {
    kind: CellKind,
    process: ProcessId,
    capabilities: CapabilitySet,
    required_capabilities: CapabilitySet,
    quota: CellQuota,
    recovery: CellRecoveryPolicy,
    dependencies: [Option<CellId>; MAX_CELL_DEPENDENCIES],
    dependency_count: usize,
    interfaces: [Option<CellInterface>; MAX_CELL_INTERFACES],
    interface_count: usize,
}

impl CellSpec {
    pub const fn new(
        kind: CellKind,
        process: ProcessId,
        capabilities: CapabilitySet,
        required_capabilities: CapabilitySet,
        quota: CellQuota,
        recovery: CellRecoveryPolicy,
    ) -> Self {
        Self {
            kind,
            process,
            capabilities,
            required_capabilities,
            quota,
            recovery,
            dependencies: [None; MAX_CELL_DEPENDENCIES],
            dependency_count: 0,
            interfaces: [None; MAX_CELL_INTERFACES],
            interface_count: 0,
        }
    }

    pub fn add_dependency(&mut self, dependency: CellId) -> Result<(), CellError> {
        if dependency.0 == 0 {
            return Err(CellError::InvalidDependency);
        }
        if self.dependencies[..self.dependency_count].contains(&Some(dependency)) {
            return Err(CellError::DuplicateDependency);
        }
        if self.dependency_count == MAX_CELL_DEPENDENCIES {
            return Err(CellError::DependencyCapacity);
        }
        self.dependencies[self.dependency_count] = Some(dependency);
        self.dependency_count += 1;
        Ok(())
    }

    pub fn add_interface(&mut self, interface: CellInterface) -> Result<(), CellError> {
        if !interface.is_valid() {
            return Err(CellError::InvalidInterface);
        }
        if self.interfaces[..self.interface_count]
            .iter()
            .flatten()
            .any(|existing| existing.id == interface.id)
        {
            return Err(CellError::DuplicateInterface);
        }
        if self.interface_count == MAX_CELL_INTERFACES {
            return Err(CellError::InterfaceCapacity);
        }
        self.interfaces[self.interface_count] = Some(interface);
        self.interface_count += 1;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellDescriptor {
    id: CellId,
    kind: CellKind,
    process: ProcessId,
    state: CellState,
    health: CellHealth,
    capabilities: CapabilitySet,
    required_capabilities: CapabilitySet,
    quota: CellQuota,
    usage: CellUsage,
    recovery: CellRecoveryPolicy,
    failures: u16,
    dependencies: [Option<CellId>; MAX_CELL_DEPENDENCIES],
    dependency_count: usize,
    interfaces: [Option<CellInterface>; MAX_CELL_INTERFACES],
    interface_count: usize,
}

impl CellDescriptor {
    pub const fn id(self) -> CellId {
        self.id
    }

    pub const fn kind(self) -> CellKind {
        self.kind
    }

    pub const fn process(self) -> ProcessId {
        self.process
    }

    pub const fn state(self) -> CellState {
        self.state
    }

    pub const fn health(self) -> CellHealth {
        self.health
    }

    pub const fn capabilities(self) -> CapabilitySet {
        self.capabilities
    }

    pub const fn quota(self) -> CellQuota {
        self.quota
    }

    pub const fn usage(self) -> CellUsage {
        self.usage
    }

    pub const fn failures(self) -> u16 {
        self.failures
    }

    pub fn dependencies(&self) -> &[Option<CellId>] {
        &self.dependencies[..self.dependency_count]
    }

    pub fn interfaces(&self) -> &[Option<CellInterface>] {
        &self.interfaces[..self.interface_count]
    }

    fn has_interface(&self, interface_id: u32) -> Option<CellInterface> {
        self.interfaces[..self.interface_count]
            .iter()
            .flatten()
            .copied()
            .find(|interface| interface.id == interface_id)
    }

    fn has_capability(&self, capability: KernelCapability) -> bool {
        self.capabilities.contains(capability)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellMessage {
    pub channel: CellChannelId,
    pub sender: CellId,
    pub receiver: CellId,
    pub interface: CellInterface,
    pub request_id: u64,
    pub opcode: u32,
    pub payload: [u64; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSendRequest {
    pub channel: CellChannelId,
    pub sender: CellId,
    pub caller_process: ProcessId,
    pub request_id: u64,
    pub opcode: u32,
    pub payload: [u64; 4],
}

impl CellSendRequest {
    pub const fn new(
        channel: CellChannelId,
        sender: CellId,
        caller_process: ProcessId,
        request_id: u64,
        opcode: u32,
        payload: [u64; 4],
    ) -> Self {
        Self {
            channel,
            sender,
            caller_process,
            request_id,
            opcode,
            payload,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellError {
    Full,
    NotFound,
    InvalidId,
    IdExhausted,
    DuplicateCell,
    DuplicateProcess,
    ProcessNotFound,
    ProcessNotCreated,
    ProcessStateMismatch,
    InsufficientProcessBudget,
    MissingCapability,
    InvalidDependency,
    DuplicateDependency,
    DependencyCapacity,
    DependencyUnavailable,
    DependencyInUse,
    InvalidInterface,
    DuplicateInterface,
    InterfaceCapacity,
    InvalidTransition,
    QuotaExceeded,
    RestartLimitExceeded,
    ChannelTableFull,
    ChannelIdExhausted,
    ChannelNotFound,
    ChannelFull,
    ChannelEmpty,
    InvalidSenderSet,
    SenderCapacity,
    UnauthorizedSender,
    InvalidReceiver,
    InvalidRequest,
    CallerMismatch,
    Process(ProcessManagerError),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellChannel<const Q: usize, const S: usize> {
    id: CellChannelId,
    receiver: CellId,
    interface: CellInterface,
    senders: [Option<CellId>; S],
    sender_count: usize,
    messages: [Option<CellMessage>; Q],
    head: usize,
    len: usize,
}

impl<const Q: usize, const S: usize> CellChannel<Q, S> {
    const fn new(
        id: CellChannelId,
        receiver: CellId,
        interface: CellInterface,
        senders: [Option<CellId>; S],
        sender_count: usize,
    ) -> Self {
        Self {
            id,
            receiver,
            interface,
            senders,
            sender_count,
            messages: [None; Q],
            head: 0,
            len: 0,
        }
    }

    fn allows_sender(&self, sender: CellId) -> bool {
        self.senders[..self.sender_count].contains(&Some(sender))
    }

    fn is_full(&self) -> bool {
        Q == 0 || self.len == Q
    }

    fn push(&mut self, message: CellMessage) -> Result<(), CellError> {
        if self.is_full() {
            return Err(CellError::ChannelFull);
        }
        let index = (self.head + self.len) % Q;
        self.messages[index] = Some(message);
        self.len += 1;
        Ok(())
    }

    fn pop(&mut self) -> Result<CellMessage, CellError> {
        if self.len == 0 {
            return Err(CellError::ChannelEmpty);
        }
        let message = self.messages[self.head]
            .take()
            .ok_or(CellError::ChannelEmpty)?;
        self.head = (self.head + 1) % Q;
        self.len -= 1;
        if self.len == 0 {
            self.head = 0;
        }
        Ok(message)
    }

    fn remove_sender(&mut self, sender: CellId) {
        let mut senders = [None; S];
        let mut sender_count = 0;
        for allowed in self.senders[..self.sender_count].iter().flatten().copied() {
            if allowed != sender {
                senders[sender_count] = Some(allowed);
                sender_count += 1;
            }
        }
        self.senders = senders;
        self.sender_count = sender_count;

        if Q == 0 {
            self.head = 0;
            self.len = 0;
            return;
        }
        let mut messages = [None; Q];
        let mut message_count = 0;
        for offset in 0..self.len {
            let index = (self.head + offset) % Q;
            if let Some(message) = self.messages[index]
                && message.sender != sender
            {
                messages[message_count] = Some(message);
                message_count += 1;
            }
        }
        self.messages = messages;
        self.head = 0;
        self.len = message_count;
    }
}

/// Fixed-capacity local Cell Runtime.
///
/// The first const parameter bounds registered cells. The remaining parameters
/// bound channels, queued messages per channel, and authorized senders per
/// channel. All storage is inline and allocation-free.
pub struct CellManager<
    const N: usize,
    const CHANNELS: usize = 16,
    const QUEUE: usize = 8,
    const SENDERS: usize = 8,
> {
    cells: [Option<CellDescriptor>; N],
    channels: [Option<CellChannel<QUEUE, SENDERS>>; CHANNELS],
    count: usize,
    next_cell_id: u64,
    next_channel_id: u64,
}

impl<const N: usize, const C: usize, const Q: usize, const S: usize> Default
    for CellManager<N, C, Q, S>
{
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize, const C: usize, const Q: usize, const S: usize> CellManager<N, C, Q, S> {
    pub const fn new() -> Self {
        Self {
            cells: [None; N],
            channels: [None; C],
            count: 0,
            next_cell_id: 1,
            next_channel_id: 1,
        }
    }

    pub const fn len(&self) -> usize {
        self.count
    }

    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn cell(&self, id: CellId) -> Option<CellDescriptor> {
        self.find_index(id).and_then(|index| self.cells[index])
    }

    /// Register a cell only against a process that already exists in the
    /// ProcessManager in Created state. The process ID remains owned by that
    /// manager and is never synthesized or recycled by the CellManager.
    pub fn register<const P: usize>(
        &mut self,
        spec: CellSpec,
        processes: &ProcessManager<P>,
    ) -> Result<CellId, CellError> {
        if N == 0 {
            return Err(CellError::Full);
        }
        if spec.process.0 == 0 {
            return Err(CellError::InvalidId);
        }
        if self
            .cells
            .iter()
            .flatten()
            .any(|cell| cell.process == spec.process)
        {
            return Err(CellError::DuplicateProcess);
        }
        if !capabilities_contain(spec.capabilities, spec.required_capabilities) {
            return Err(CellError::MissingCapability);
        }
        let process = processes
            .descriptor(spec.process)
            .ok_or(CellError::ProcessNotFound)?;
        if process.state != ProcessState::Created {
            return Err(CellError::ProcessNotCreated);
        }
        if !spec.quota.fits_process_budget(process.budget) {
            return Err(CellError::InsufficientProcessBudget);
        }
        for dependency in spec.dependencies[..spec.dependency_count].iter().flatten() {
            let dependency_cell = self.cell(*dependency).ok_or(CellError::InvalidDependency)?;
            if matches!(
                dependency_cell.state,
                CellState::Failed | CellState::Stopped | CellState::Quarantined
            ) {
                return Err(CellError::DependencyUnavailable);
            }
        }
        let slot = self
            .cells
            .iter()
            .position(Option::is_none)
            .ok_or(CellError::Full)?;
        if self.next_cell_id == 0 {
            return Err(CellError::IdExhausted);
        }
        let raw_id = self.next_cell_id;
        self.next_cell_id = raw_id.checked_add(1).unwrap_or(0);
        let id = CellId(raw_id);
        self.cells[slot] = Some(CellDescriptor {
            id,
            kind: spec.kind,
            process: spec.process,
            state: CellState::Declared,
            health: CellHealth::Unknown,
            capabilities: spec.capabilities,
            required_capabilities: spec.required_capabilities,
            quota: spec.quota,
            usage: CellUsage::default(),
            recovery: spec.recovery,
            failures: 0,
            dependencies: spec.dependencies,
            dependency_count: spec.dependency_count,
            interfaces: spec.interfaces,
            interface_count: spec.interface_count,
        });
        self.count += 1;
        Ok(id)
    }

    pub fn admit(&mut self, id: CellId) -> Result<(), CellError> {
        let index = self.find_index(id).ok_or(CellError::NotFound)?;
        let mut cell = self.cells[index].ok_or(CellError::NotFound)?;
        if cell.state != CellState::Declared {
            return Err(CellError::InvalidTransition);
        }
        for dependency in cell.dependencies().iter().flatten() {
            let dependency = self.cell(*dependency).ok_or(CellError::InvalidDependency)?;
            if !matches!(
                dependency.state,
                CellState::Admitted | CellState::Starting | CellState::Running
            ) {
                return Err(CellError::DependencyUnavailable);
            }
        }
        cell.state = CellState::Admitted;
        self.cells[index] = Some(cell);
        Ok(())
    }

    /// Enqueue the bound process first; only publish Starting after scheduler
    /// admission succeeds. Dependencies must already be Running cells.
    pub fn begin_start<const P: usize>(
        &mut self,
        id: CellId,
        processes: &mut ProcessManager<P>,
    ) -> Result<(), CellError> {
        let index = self.find_index(id).ok_or(CellError::NotFound)?;
        let mut cell = self.cells[index].ok_or(CellError::NotFound)?;
        if cell.state != CellState::Admitted {
            return Err(CellError::InvalidTransition);
        }
        for dependency in cell.dependencies().iter().flatten() {
            if self.cell(*dependency).map(|item| item.state) != Some(CellState::Running) {
                return Err(CellError::DependencyUnavailable);
            }
        }
        let process = processes
            .descriptor(cell.process)
            .ok_or(CellError::ProcessNotFound)?;
        if process.state != ProcessState::Created {
            return Err(CellError::ProcessStateMismatch);
        }
        if !cell.quota.fits_process_budget(process.budget) {
            return Err(CellError::InsufficientProcessBudget);
        }
        processes
            .make_runnable(cell.process)
            .map_err(CellError::Process)?;
        cell.state = CellState::Starting;
        self.cells[index] = Some(cell);
        Ok(())
    }

    /// Called after the architecture/scheduler path has marked the bound
    /// process Running. Cell state is never advanced based on intent alone.
    pub fn confirm_running<const P: usize>(
        &mut self,
        id: CellId,
        processes: &ProcessManager<P>,
    ) -> Result<(), CellError> {
        let index = self.find_index(id).ok_or(CellError::NotFound)?;
        let mut cell = self.cells[index].ok_or(CellError::NotFound)?;
        if cell.state != CellState::Starting {
            return Err(CellError::InvalidTransition);
        }
        let process = processes
            .descriptor(cell.process)
            .ok_or(CellError::ProcessNotFound)?;
        if process.state != ProcessState::Running {
            return Err(CellError::ProcessStateMismatch);
        }
        cell.state = CellState::Running;
        cell.health = CellHealth::Healthy;
        self.cells[index] = Some(cell);
        Ok(())
    }

    /// Stop scheduling the process and close its cell IPC endpoints. Reaping is
    /// deliberately deferred until ProcessManager confirms it is not current.
    pub fn begin_stop<const P: usize>(
        &mut self,
        id: CellId,
        processes: &mut ProcessManager<P>,
    ) -> Result<(), CellError> {
        let index = self.find_index(id).ok_or(CellError::NotFound)?;
        let mut cell = self.cells[index].ok_or(CellError::NotFound)?;
        if cell.state != CellState::Running {
            return Err(CellError::InvalidTransition);
        }
        processes.exit(cell.process).map_err(CellError::Process)?;
        cell.state = CellState::Stopping;
        cell.health = CellHealth::Stopped;
        self.cells[index] = Some(cell);
        self.teardown_channels_for(id);
        Ok(())
    }

    /// Reap only after the process manager's current/queue safety checks pass.
    pub fn confirm_stopped<const P: usize>(
        &mut self,
        id: CellId,
        processes: &mut ProcessManager<P>,
    ) -> Result<(), CellError> {
        let index = self.find_index(id).ok_or(CellError::NotFound)?;
        let mut cell = self.cells[index].ok_or(CellError::NotFound)?;
        if cell.state != CellState::Stopping {
            return Err(CellError::InvalidTransition);
        }
        let process = processes
            .descriptor(cell.process)
            .ok_or(CellError::ProcessNotFound)?;
        if process.state != ProcessState::Exited {
            return Err(CellError::ProcessStateMismatch);
        }
        processes.reap(cell.process).map_err(CellError::Process)?;
        cell.state = CellState::Stopped;
        cell.health = CellHealth::Stopped;
        self.cells[index] = Some(cell);
        Ok(())
    }

    /// Record a process failure after forcing its ProcessManager entry to Exited.
    /// The failure threshold is explicit; exceeding it quarantines the cell.
    pub fn record_failure<const P: usize>(
        &mut self,
        id: CellId,
        processes: &mut ProcessManager<P>,
    ) -> Result<CellState, CellError> {
        let index = self.find_index(id).ok_or(CellError::NotFound)?;
        let mut cell = self.cells[index].ok_or(CellError::NotFound)?;
        if !matches!(
            cell.state,
            CellState::Starting | CellState::Running | CellState::Stopping
        ) {
            return Err(CellError::InvalidTransition);
        }
        let process = processes
            .descriptor(cell.process)
            .ok_or(CellError::ProcessNotFound)?;
        if process.state != ProcessState::Exited {
            processes.exit(cell.process).map_err(CellError::Process)?;
        }
        cell.failures = cell.failures.saturating_add(1);
        cell.health = CellHealth::Faulted;
        cell.state = if cell.failures > u16::from(cell.recovery.max_restarts) {
            CellState::Quarantined
        } else {
            CellState::Failed
        };
        self.cells[index] = Some(cell);
        self.teardown_channels_for(id);
        Ok(cell.state)
    }

    pub fn reap_failed<const P: usize>(
        &self,
        id: CellId,
        processes: &mut ProcessManager<P>,
    ) -> Result<(), CellError> {
        let cell = self.cell(id).ok_or(CellError::NotFound)?;
        if !matches!(cell.state, CellState::Failed | CellState::Quarantined) {
            return Err(CellError::InvalidTransition);
        }
        let process = processes
            .descriptor(cell.process)
            .ok_or(CellError::ProcessNotFound)?;
        if process.state != ProcessState::Exited {
            return Err(CellError::ProcessStateMismatch);
        }
        processes.reap(cell.process).map_err(CellError::Process)
    }

    /// Restart means binding a fresh Created process after the failed process
    /// has been safely reaped. A stale ProcessContext is never resurrected.
    pub fn rebind_process<const P: usize>(
        &mut self,
        id: CellId,
        new_process: ProcessId,
        processes: &ProcessManager<P>,
    ) -> Result<(), CellError> {
        let index = self.find_index(id).ok_or(CellError::NotFound)?;
        let mut cell = self.cells[index].ok_or(CellError::NotFound)?;
        if cell.state != CellState::Failed {
            return Err(if cell.state == CellState::Quarantined {
                CellError::RestartLimitExceeded
            } else {
                CellError::InvalidTransition
            });
        }
        if cell.failures > u16::from(cell.recovery.max_restarts) {
            return Err(CellError::RestartLimitExceeded);
        }
        if processes.descriptor(cell.process).is_some() {
            return Err(CellError::ProcessStateMismatch);
        }
        if self
            .cells
            .iter()
            .flatten()
            .any(|other| other.id != id && other.process == new_process)
        {
            return Err(CellError::DuplicateProcess);
        }
        if new_process.0 == 0 {
            return Err(CellError::InvalidId);
        }
        let process = processes
            .descriptor(new_process)
            .ok_or(CellError::ProcessNotFound)?;
        if process.state != ProcessState::Created {
            return Err(CellError::ProcessNotCreated);
        }
        if !cell.quota.fits_process_budget(process.budget) {
            return Err(CellError::InsufficientProcessBudget);
        }
        if !capabilities_contain(cell.capabilities, cell.required_capabilities) {
            return Err(CellError::MissingCapability);
        }
        cell.process = new_process;
        cell.state = CellState::Admitted;
        cell.health = CellHealth::Unknown;
        cell.usage = CellUsage::default();
        self.cells[index] = Some(cell);
        Ok(())
    }

    pub fn set_health(&mut self, id: CellId, health: CellHealth) -> Result<(), CellError> {
        if matches!(
            health,
            CellHealth::Unknown | CellHealth::Faulted | CellHealth::Stopped
        ) {
            return Err(CellError::InvalidTransition);
        }
        let index = self.find_index(id).ok_or(CellError::NotFound)?;
        let mut cell = self.cells[index].ok_or(CellError::NotFound)?;
        if cell.state != CellState::Running {
            return Err(CellError::InvalidTransition);
        }
        cell.health = health;
        self.cells[index] = Some(cell);
        Ok(())
    }

    /// Explicit scheduler accounting hook. It is not automatically charged
    /// until the timer/scheduler path calls it.
    pub fn charge_cpu(&mut self, id: CellId, ticks: u64) -> Result<(), CellError> {
        let index = self.find_index(id).ok_or(CellError::NotFound)?;
        let mut cell = self.cells[index].ok_or(CellError::NotFound)?;
        if cell.state != CellState::Running {
            return Err(CellError::InvalidTransition);
        }
        let next = cell
            .usage
            .cpu_ticks
            .checked_add(ticks)
            .ok_or(CellError::QuotaExceeded)?;
        if next > cell.quota.cpu_ticks {
            return Err(CellError::QuotaExceeded);
        }
        cell.usage.cpu_ticks = next;
        self.cells[index] = Some(cell);
        Ok(())
    }

    /// Explicit VM/accounting hook. It is not automatic until the allocator
    /// charges allocations and releases to this cell identity.
    pub fn charge_memory(&mut self, id: CellId, bytes: u64) -> Result<(), CellError> {
        let index = self.find_index(id).ok_or(CellError::NotFound)?;
        let mut cell = self.cells[index].ok_or(CellError::NotFound)?;
        if cell.state != CellState::Running {
            return Err(CellError::InvalidTransition);
        }
        let next = cell
            .usage
            .memory_bytes
            .checked_add(bytes)
            .ok_or(CellError::QuotaExceeded)?;
        if next > cell.quota.memory_bytes {
            return Err(CellError::QuotaExceeded);
        }
        cell.usage.memory_bytes = next;
        self.cells[index] = Some(cell);
        Ok(())
    }

    /// Only a running Security-capable broker may create a local endpoint.
    /// Receiver and senders must declare compatible, versioned interfaces and
    /// hold the IPC capability. Sender authorization is an explicit CellId
    /// list, not a truncated bit mask.
    pub fn create_channel<const P: usize>(
        &mut self,
        owner: CellId,
        caller_process: ProcessId,
        receiver: CellId,
        allowed_senders: &[CellId],
        interface_id: u32,
        processes: &ProcessManager<P>,
    ) -> Result<CellChannelId, CellError> {
        if C == 0 {
            return Err(CellError::ChannelTableFull);
        }
        let owner_cell = self.cell(owner).ok_or(CellError::NotFound)?;
        if owner_cell.process != caller_process {
            return Err(CellError::CallerMismatch);
        }
        let caller = processes
            .descriptor(caller_process)
            .ok_or(CellError::ProcessNotFound)?;
        if caller.state != ProcessState::Running {
            return Err(CellError::ProcessStateMismatch);
        }
        if owner_cell.state != CellState::Running
            || !owner_cell.has_capability(KernelCapability::Security)
            || !owner_cell.has_capability(KernelCapability::Ipc)
        {
            return Err(CellError::MissingCapability);
        }
        let receiver_cell = self.cell(receiver).ok_or(CellError::NotFound)?;
        if !cell_can_host_endpoint(receiver_cell.state)
            || !receiver_cell.has_capability(KernelCapability::Ipc)
        {
            return Err(CellError::InvalidReceiver);
        }
        let interface = receiver_cell
            .has_interface(interface_id)
            .ok_or(CellError::InvalidInterface)?;
        if allowed_senders.is_empty() {
            return Err(CellError::InvalidSenderSet);
        }
        if allowed_senders.len() > S {
            return Err(CellError::SenderCapacity);
        }
        let mut senders = [None; S];
        for (index, sender_id) in allowed_senders.iter().copied().enumerate() {
            if sender_id.0 == 0 || sender_id == receiver {
                return Err(CellError::InvalidSenderSet);
            }
            if allowed_senders[..index].contains(&sender_id) {
                return Err(CellError::InvalidSenderSet);
            }
            let sender = self.cell(sender_id).ok_or(CellError::NotFound)?;
            if !cell_can_host_endpoint(sender.state)
                || !sender.has_capability(KernelCapability::Ipc)
            {
                return Err(CellError::UnauthorizedSender);
            }
            let sender_interface = sender
                .has_interface(interface_id)
                .ok_or(CellError::InvalidInterface)?;
            if sender_interface.abi_major != interface.abi_major
                || sender_interface.abi_minor < interface.abi_minor
            {
                return Err(CellError::InvalidInterface);
            }
            senders[index] = Some(sender_id);
        }
        let slot = self
            .channels
            .iter()
            .position(Option::is_none)
            .ok_or(CellError::ChannelTableFull)?;
        if self.next_channel_id == 0 {
            return Err(CellError::ChannelIdExhausted);
        }
        let raw_id = self.next_channel_id;
        self.next_channel_id = raw_id.checked_add(1).unwrap_or(0);
        let id = CellChannelId(raw_id);
        self.channels[slot] = Some(CellChannel::new(
            id,
            receiver,
            interface,
            senders,
            allowed_senders.len(),
        ));
        Ok(id)
    }

    pub fn send<const P: usize>(
        &mut self,
        request: CellSendRequest,
        processes: &ProcessManager<P>,
    ) -> Result<(), CellError> {
        if request.request_id == 0 || request.opcode == 0 {
            return Err(CellError::InvalidRequest);
        }
        let channel_index = self
            .channels
            .iter()
            .position(|channel| channel.map(|item| item.id) == Some(request.channel))
            .ok_or(CellError::ChannelNotFound)?;
        let mut channel = self.channels[channel_index].ok_or(CellError::ChannelNotFound)?;
        if !channel.allows_sender(request.sender) {
            return Err(CellError::UnauthorizedSender);
        }
        let sender_index = self.find_index(request.sender).ok_or(CellError::NotFound)?;
        let receiver_index = self
            .find_index(channel.receiver)
            .ok_or(CellError::InvalidReceiver)?;
        let mut sender_cell = self.cells[sender_index].ok_or(CellError::NotFound)?;
        let receiver_cell = self.cells[receiver_index].ok_or(CellError::InvalidReceiver)?;
        if sender_cell.process != request.caller_process {
            return Err(CellError::CallerMismatch);
        }
        if sender_cell.state != CellState::Running
            || !sender_cell.has_capability(KernelCapability::Ipc)
        {
            return Err(CellError::UnauthorizedSender);
        }
        if !cell_can_host_endpoint(receiver_cell.state)
            || !receiver_cell.has_capability(KernelCapability::Ipc)
        {
            return Err(CellError::InvalidReceiver);
        }
        let sender_process = processes
            .descriptor(sender_cell.process)
            .ok_or(CellError::ProcessNotFound)?;
        if sender_process.state != ProcessState::Running {
            return Err(CellError::ProcessStateMismatch);
        }
        let receiver_process = processes
            .descriptor(receiver_cell.process)
            .ok_or(CellError::ProcessNotFound)?;
        if receiver_process.state == ProcessState::Exited {
            return Err(CellError::InvalidReceiver);
        }
        if channel.is_full() {
            return Err(CellError::ChannelFull);
        }
        let next_ipc_usage = sender_cell
            .usage
            .ipc_messages
            .checked_add(1)
            .ok_or(CellError::QuotaExceeded)?;
        if next_ipc_usage > sender_cell.quota.ipc_messages {
            return Err(CellError::QuotaExceeded);
        }
        let message = CellMessage {
            channel: request.channel,
            sender: request.sender,
            receiver: channel.receiver,
            interface: channel.interface,
            request_id: request.request_id,
            opcode: request.opcode,
            payload: request.payload,
        };
        channel.push(message)?;
        self.channels[channel_index] = Some(channel);
        sender_cell.usage.ipc_messages = next_ipc_usage;
        self.cells[sender_index] = Some(sender_cell);
        Ok(())
    }

    pub fn receive<const P: usize>(
        &mut self,
        channel_id: CellChannelId,
        receiver: CellId,
        caller_process: ProcessId,
        processes: &ProcessManager<P>,
    ) -> Result<CellMessage, CellError> {
        let channel_index = self
            .channels
            .iter()
            .position(|channel| channel.map(|item| item.id) == Some(channel_id))
            .ok_or(CellError::ChannelNotFound)?;
        let channel = self.channels[channel_index].ok_or(CellError::ChannelNotFound)?;
        if channel.receiver != receiver {
            return Err(CellError::InvalidReceiver);
        }
        let receiver_cell = self.cell(receiver).ok_or(CellError::NotFound)?;
        if receiver_cell.process != caller_process {
            return Err(CellError::CallerMismatch);
        }
        if receiver_cell.state != CellState::Running
            || !receiver_cell.has_capability(KernelCapability::Ipc)
        {
            return Err(CellError::InvalidReceiver);
        }
        let receiver_process = processes
            .descriptor(receiver_cell.process)
            .ok_or(CellError::ProcessNotFound)?;
        if receiver_process.state != ProcessState::Running {
            return Err(CellError::ProcessStateMismatch);
        }
        let mut channel = channel;
        let message = channel.pop()?;
        self.channels[channel_index] = Some(channel);
        Ok(message)
    }

    /// Remove all channels owned by the stopped receiver and revoke a failed
    /// sender from every other channel, including messages already queued.
    fn teardown_channels_for(&mut self, cell: CellId) {
        for channel in &mut self.channels {
            if let Some(endpoint) = channel.as_mut() {
                if endpoint.receiver == cell {
                    *channel = None;
                } else {
                    endpoint.remove_sender(cell);
                    if endpoint.sender_count == 0 {
                        *channel = None;
                    }
                }
            }
        }
    }

    pub fn remove<const P: usize>(
        &mut self,
        id: CellId,
        processes: &ProcessManager<P>,
    ) -> Result<(), CellError> {
        let index = self.find_index(id).ok_or(CellError::NotFound)?;
        let cell = self.cells[index].ok_or(CellError::NotFound)?;
        if !matches!(cell.state, CellState::Stopped | CellState::Quarantined) {
            return Err(CellError::InvalidTransition);
        }
        if processes.descriptor(cell.process).is_some() {
            return Err(CellError::ProcessStateMismatch);
        }
        if self
            .cells
            .iter()
            .flatten()
            .any(|other| other.id != id && other.dependencies().contains(&Some(id)))
        {
            return Err(CellError::DependencyInUse);
        }
        if self
            .channels
            .iter()
            .flatten()
            .any(|channel| channel.receiver == id || channel.allows_sender(id))
        {
            return Err(CellError::InvalidTransition);
        }
        self.cells[index] = None;
        self.count -= 1;
        Ok(())
    }

    fn find_index(&self, id: CellId) -> Option<usize> {
        self.cells
            .iter()
            .position(|cell| cell.map(|item| item.id) == Some(id))
    }
}

fn capabilities_contain(granted: CapabilitySet, required: CapabilitySet) -> bool {
    granted.bits() & required.bits() == required.bits()
}

fn cell_can_host_endpoint(state: CellState) -> bool {
    matches!(
        state,
        CellState::Admitted | CellState::Starting | CellState::Running
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::context::{CpuContext, ProcessContext};

    const IPC_CAPS: CapabilitySet = CapabilitySet::EMPTY
        .with(KernelCapability::Processes)
        .with(KernelCapability::Ipc);

    const BROKER_CAPS: CapabilitySet = IPC_CAPS.with(KernelCapability::Security);

    fn process_budget() -> ResourceBudget {
        ResourceBudget {
            cpu_ticks: 100,
            memory_bytes: 16 * 1024,
            ipc_messages: 16,
        }
    }

    fn quota() -> CellQuota {
        CellQuota::new(20, 4096, 4)
    }

    fn register_process<const N: usize>(
        processes: &mut ProcessManager<N>,
        id: u64,
        budget: ResourceBudget,
    ) {
        let process_id = ProcessId(id);
        let descriptor = crate::process::ProcessDescriptor {
            id: process_id,
            state: ProcessState::Created,
            budget,
        };
        let context = ProcessContext::new(
            process_id,
            CpuContext::kernel_entry(0x1000 + id * 0x10, 0x8000 + id * 0x100, 0),
        );
        processes.register(descriptor, context).unwrap();
    }

    fn spec(process: u64, caps: CapabilitySet, recovery: u8) -> CellSpec {
        CellSpec::new(
            CellKind::Service,
            ProcessId(process),
            caps,
            IPC_CAPS,
            quota(),
            CellRecoveryPolicy::new(recovery),
        )
    }

    fn start_cell<const N: usize, const C: usize, const Q: usize, const S: usize>(
        cells: &mut CellManager<N, C, Q, S>,
        processes: &mut ProcessManager<N>,
        id: CellId,
    ) {
        cells.admit(id).unwrap();
        cells.begin_start(id, processes).unwrap();
        let process = cells.cell(id).unwrap().process();
        processes.mark_running(process).unwrap();
        cells.confirm_running(id, processes).unwrap();
    }

    #[test]
    fn cell_ids_are_monotonic_and_exhaust_without_reuse() {
        let mut processes = ProcessManager::<3>::new();
        register_process(&mut processes, 1, process_budget());
        register_process(&mut processes, 2, process_budget());
        register_process(&mut processes, 3, process_budget());
        let mut cells = CellManager::<3>::new();
        assert_eq!(
            cells.register(spec(1, IPC_CAPS, 1), &processes),
            Ok(CellId(1))
        );
        cells.next_cell_id = u64::MAX;
        assert_eq!(
            cells.register(spec(2, IPC_CAPS, 1), &processes),
            Ok(CellId(u64::MAX))
        );
        assert_eq!(
            cells.register(spec(3, IPC_CAPS, 1), &processes),
            Err(CellError::IdExhausted)
        );
    }

    #[test]
    fn registration_rejects_missing_process_capability_and_budget() {
        let mut processes = ProcessManager::<3>::new();
        register_process(&mut processes, 1, process_budget());
        register_process(
            &mut processes,
            2,
            ResourceBudget {
                cpu_ticks: 1,
                memory_bytes: 128,
                ipc_messages: 1,
            },
        );
        let mut cells = CellManager::<3>::new();
        assert_eq!(
            cells.register(spec(9, IPC_CAPS, 1), &processes),
            Err(CellError::ProcessNotFound)
        );
        assert_eq!(
            cells.register(spec(1, CapabilitySet::EMPTY, 1), &processes),
            Err(CellError::MissingCapability)
        );
        assert_eq!(
            cells.register(spec(2, IPC_CAPS, 1), &processes),
            Err(CellError::InsufficientProcessBudget)
        );
    }

    #[test]
    fn lifecycle_is_confirmed_by_process_manager_and_reap_is_deferred() {
        let mut processes = ProcessManager::<2>::new();
        register_process(&mut processes, 1, process_budget());
        let mut cells = CellManager::<2>::new();
        let id = cells.register(spec(1, IPC_CAPS, 1), &processes).unwrap();

        cells.admit(id).unwrap();
        cells.begin_start(id, &mut processes).unwrap();
        assert_eq!(cells.cell(id).unwrap().state(), CellState::Starting);
        assert_eq!(
            cells.confirm_running(id, &processes),
            Err(CellError::ProcessStateMismatch)
        );

        processes.mark_running(ProcessId(1)).unwrap();
        cells.confirm_running(id, &processes).unwrap();
        assert_eq!(cells.cell(id).unwrap().state(), CellState::Running);

        cells.begin_stop(id, &mut processes).unwrap();
        assert_eq!(
            cells.confirm_stopped(id, &mut processes),
            Err(CellError::Process(ProcessManagerError::CurrentlyRunning))
        );
        processes.confirm_switched_to_idle();
        cells.confirm_stopped(id, &mut processes).unwrap();
        assert_eq!(cells.cell(id).unwrap().state(), CellState::Stopped);
        assert!(processes.descriptor(ProcessId(1)).is_none());
        cells.remove(id, &processes).unwrap();
        assert!(cells.is_empty());
    }

    #[test]
    fn dependencies_must_be_admitted_and_running_before_dependent_start() {
        let mut processes = ProcessManager::<3>::new();
        register_process(&mut processes, 1, process_budget());
        register_process(&mut processes, 2, process_budget());
        let mut cells = CellManager::<3>::new();
        let dependency = cells.register(spec(1, IPC_CAPS, 1), &processes).unwrap();
        let mut dependent_spec = spec(2, IPC_CAPS, 1);
        dependent_spec.add_dependency(dependency).unwrap();
        let dependent = cells.register(dependent_spec, &processes).unwrap();

        cells.admit(dependency).unwrap();
        cells.admit(dependent).unwrap();
        assert_eq!(
            cells.begin_start(dependent, &mut processes),
            Err(CellError::DependencyUnavailable)
        );

        cells.begin_start(dependency, &mut processes).unwrap();
        processes.mark_running(ProcessId(1)).unwrap();
        cells.confirm_running(dependency, &processes).unwrap();
        cells.begin_start(dependent, &mut processes).unwrap();
    }

    #[test]
    fn quota_accounting_is_bounded_and_failure_is_atomic() {
        let mut processes = ProcessManager::<1>::new();
        register_process(&mut processes, 1, process_budget());
        let mut cells = CellManager::<1>::new();
        let id = cells.register(spec(1, IPC_CAPS, 1), &processes).unwrap();
        start_cell(&mut cells, &mut processes, id);

        cells.charge_cpu(id, 10).unwrap();
        cells.charge_memory(id, 1024).unwrap();
        assert_eq!(cells.charge_cpu(id, 11), Err(CellError::QuotaExceeded));
        assert_eq!(cells.charge_memory(id, 4096), Err(CellError::QuotaExceeded));
        let usage = cells.cell(id).unwrap().usage();
        assert_eq!(usage.cpu_ticks, 10);
        assert_eq!(usage.memory_bytes, 1024);
    }

    #[test]
    fn channel_requires_security_broker_and_compatible_interfaces() {
        let mut processes = ProcessManager::<3>::new();
        register_process(&mut processes, 1, process_budget());
        register_process(&mut processes, 2, process_budget());
        register_process(&mut processes, 3, process_budget());
        let mut cells = CellManager::<3, 2, 2, 2>::new();
        let broker = cells
            .register(
                CellSpec::new(
                    CellKind::Security,
                    ProcessId(1),
                    BROKER_CAPS,
                    IPC_CAPS,
                    quota(),
                    CellRecoveryPolicy::new(1),
                ),
                &processes,
            )
            .unwrap();
        let mut receiver_spec = spec(2, IPC_CAPS, 1);
        receiver_spec
            .add_interface(CellInterface::new(7, 1, 2))
            .unwrap();
        let receiver = cells.register(receiver_spec, &processes).unwrap();
        let mut sender_spec = spec(3, IPC_CAPS, 1);
        sender_spec
            .add_interface(CellInterface::new(7, 1, 3))
            .unwrap();
        let sender = cells.register(sender_spec, &processes).unwrap();

        start_cell(&mut cells, &mut processes, broker);
        cells.admit(receiver).unwrap();
        cells.admit(sender).unwrap();
        assert_eq!(
            cells.create_channel(broker, ProcessId(99), receiver, &[sender], 7, &processes),
            Err(CellError::CallerMismatch)
        );
        let channel = cells
            .create_channel(broker, ProcessId(1), receiver, &[sender], 7, &processes)
            .unwrap();

        cells.begin_start(sender, &mut processes).unwrap();
        processes.mark_running(ProcessId(3)).unwrap();
        cells.confirm_running(sender, &processes).unwrap();
        cells
            .send(
                channel,
                sender,
                ProcessId(3),
                1,
                10,
                [11, 12, 13, 14],
                &processes,
            )
            .unwrap();
        assert_eq!(
            cells.send(CellSendRequest::new(channel, sender, ProcessId(2), 2, 10, [0; 4]), &processes),
            Err(CellError::CallerMismatch)
        );
        assert_eq!(
            cells.send(CellSendRequest::new(channel, broker, ProcessId(1), 2, 10, [0; 4]), &processes),
            Err(CellError::UnauthorizedSender)
        );

        cells.begin_start(receiver, &mut processes).unwrap();
        processes.mark_running(ProcessId(2)).unwrap();
        cells.confirm_running(receiver, &processes).unwrap();
        assert_eq!(
            cells.receive(channel, receiver, ProcessId(3), &processes),
            Err(CellError::CallerMismatch)
        );
        let message = cells
            .receive(channel, receiver, ProcessId(2), &processes)
            .unwrap();
        assert_eq!(message.sender, sender);
        assert_eq!(message.receiver, receiver);
        assert_eq!(message.interface, CellInterface::new(7, 1, 2));
        assert_eq!(message.payload, [11, 12, 13, 14]);
    }

    #[test]
    fn channel_queue_and_ipc_quota_fail_closed() {
        let mut processes = ProcessManager::<3>::new();
        register_process(&mut processes, 1, process_budget());
        register_process(&mut processes, 2, process_budget());
        register_process(&mut processes, 3, process_budget());
        let mut cells = CellManager::<3, 2, 2, 2>::new();
        let broker = cells
            .register(
                CellSpec::new(
                    CellKind::Security,
                    ProcessId(1),
                    BROKER_CAPS,
                    IPC_CAPS,
                    quota(),
                    CellRecoveryPolicy::new(1),
                ),
                &processes,
            )
            .unwrap();
        let mut receiver_spec = spec(2, IPC_CAPS, 1);
        receiver_spec
            .add_interface(CellInterface::new(11, 1, 0))
            .unwrap();
        let receiver = cells.register(receiver_spec, &processes).unwrap();
        let mut sender_spec = spec(3, IPC_CAPS, 1);
        sender_spec.quota = CellQuota::new(20, 4096, 1);
        sender_spec
            .add_interface(CellInterface::new(11, 1, 0))
            .unwrap();
        let sender = cells.register(sender_spec, &processes).unwrap();

        start_cell(&mut cells, &mut processes, broker);
        cells.admit(receiver).unwrap();
        cells.admit(sender).unwrap();
        let channel = cells
            .create_channel(broker, ProcessId(1), receiver, &[sender], 11, &processes)
            .unwrap();
        cells.begin_start(sender, &mut processes).unwrap();
        processes.mark_running(ProcessId(3)).unwrap();
        cells.confirm_running(sender, &processes).unwrap();

        cells
            .send(
                channel,
                sender,
                ProcessId(3),
                1,
                1,
                [1, 0, 0, 0],
                &processes,
            )
            .unwrap();
        assert_eq!(
            cells.send(CellSendRequest::new(channel, sender, ProcessId(3), 2, 1, [2, 0, 0, 0]), &processes),
            Err(CellError::QuotaExceeded)
        );
        assert_eq!(cells.cell(sender).unwrap().usage().ipc_messages, 1);
    }

    #[test]
    fn failure_quarantines_after_restart_limit_and_revokes_channels() {
        let mut processes = ProcessManager::<3>::new();
        register_process(&mut processes, 1, process_budget());
        register_process(&mut processes, 2, process_budget());
        register_process(&mut processes, 3, process_budget());
        let mut cells = CellManager::<3, 2, 2, 2>::new();
        let broker = cells
            .register(
                CellSpec::new(
                    CellKind::Security,
                    ProcessId(1),
                    BROKER_CAPS,
                    IPC_CAPS,
                    quota(),
                    CellRecoveryPolicy::new(0),
                ),
                &processes,
            )
            .unwrap();
        let mut receiver_spec = spec(2, IPC_CAPS, 1);
        receiver_spec
            .add_interface(CellInterface::new(9, 1, 0))
            .unwrap();
        let receiver = cells.register(receiver_spec, &processes).unwrap();
        let mut sender_spec = spec(3, IPC_CAPS, 0);
        sender_spec
            .add_interface(CellInterface::new(9, 1, 0))
            .unwrap();
        let sender = cells.register(sender_spec, &processes).unwrap();

        start_cell(&mut cells, &mut processes, broker);
        cells.admit(receiver).unwrap();
        cells.admit(sender).unwrap();
        let channel = cells
            .create_channel(broker, ProcessId(1), receiver, &[sender], 9, &processes)
            .unwrap();
        cells.begin_start(sender, &mut processes).unwrap();
        processes.mark_running(ProcessId(3)).unwrap();
        cells.confirm_running(sender, &processes).unwrap();
        cells
            .send(
                channel,
                sender,
                ProcessId(3),
                1,
                1,
                [9, 0, 0, 0],
                &processes,
            )
            .unwrap();

        processes.exit(ProcessId(3)).unwrap();
        assert_eq!(
            cells.record_failure(sender, &mut processes),
            Ok(CellState::Quarantined)
        );
        assert_eq!(
            cells.rebind_process(sender, ProcessId(2), &processes),
            Err(CellError::RestartLimitExceeded)
        );
        assert_eq!(cells.cell(sender).unwrap().health(), CellHealth::Faulted);

        cells.begin_start(receiver, &mut processes).unwrap();
        processes.mark_running(ProcessId(2)).unwrap();
        cells.confirm_running(receiver, &processes).unwrap();
        assert_eq!(
            cells.receive(channel, receiver, ProcessId(2), &processes),
            Err(CellError::ChannelNotFound)
        );
    }
}
