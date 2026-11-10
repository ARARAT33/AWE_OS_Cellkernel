#![no_std]
pub mod android;
pub mod bus;
pub mod compat;
pub mod contract;
pub mod core;
pub mod hal_registers;
pub mod installer;
pub mod learning;
pub mod linux;
pub mod linux_activation;
pub mod linux_activation_rollback;
pub mod linux_dependency;
pub mod linux_dependency_graph;
pub mod linux_dependency_multi_instance;
pub mod linux_dependency_order;
pub mod linux_driver_execution_guard;
pub mod linux_driver_health;
pub mod linux_driver_ops;
pub mod linux_driver_registry;
pub mod linux_driver_resource_binding;
pub mod linux_driver_supervisor;
pub mod linux_fault_impact;
pub mod linux_fault_recovery;
pub mod linux_install;
pub mod linux_multi_instance;
pub mod linux_package;
pub mod linux_recovery_pipeline;
pub mod linux_resolver;
pub mod linux_resource_manager;
pub mod linux_resource_transaction;
pub mod linux_runtime;
pub mod linux_transaction;
pub mod linux_transaction_bridge;
pub mod linux_transaction_graph;
pub mod linux_transaction_guard;
pub mod linux_transaction_orchestrator;
pub mod pci;
pub mod pci_virtio_probe;
pub mod product_gate;
pub mod ps2;
pub mod universal;
pub mod virtio;
pub mod virtio_block;
pub mod virtio_pci;
pub mod windows;
pub use android::AndroidLayer;
pub use bus::{DeviceId, DeviceKind, DriverBus};
pub use compat::{
    CompatibilityRegistry, DriverManifest, DriverSource, bind_compatible_driver, validate_contract,
};
pub use contract::{DeviceContract, DmaPolicy, InterruptMode, MmioRegion};
pub use core::{
    AdapterState, AndroidDriverAdapter, CoreError, DriverAdapter, DriverIdentity, DriverSlot,
    HardwareAbstraction, HardwareInfo, LinuxDriverAdapter, WindowsDriverAdapter,
};
pub use hal_registers::RegisterBank;
pub use installer::{InstallError, InstallPlan, InstallerPackage, PackageFormat, plan_install};
pub use learning::{DriverExperience, ExperienceDb, ProbeOutcome};
pub use linux::LinuxLayer;
pub use linux_activation::{
    ActivationError as LinuxActivationError, build_activation_order, validate_activation_order,
};
pub use linux_activation_rollback::{
    RollbackError as LinuxRollbackError, activation_failed, build_rollback_order,
};
pub use linux_dependency::{
    Dependency as LinuxDependency, DependencyError as LinuxDependencyError,
    validate as validate_linux_dependencies,
};
pub use linux_dependency_graph::{
    GraphError as LinuxGraphError, validate_graph as validate_linux_dependency_graph,
};
pub use linux_dependency_multi_instance::{DependencyMultiError, DependencyMultiInstanceManager};
pub use linux_dependency_order::{
    OrderError as LinuxDependencyOrderError, topological_order as linux_dependency_order,
};
pub use linux_driver_execution_guard::{ExecutionGuard, ExecutionGuardError};
pub use linux_driver_health::{DriverHealth, DriverHealthMonitor, HealthError, HealthState};
pub use linux_driver_ops::{DriverLifecycle, DriverOp, DriverOpError, DriverState};
pub use linux_driver_registry::{DriverRecord, DriverRegistry, RegistryError};
pub use linux_driver_resource_binding::{BindingError, DriverResourceBinding};
pub use linux_driver_supervisor::{DriverSupervisor, SupervisorError};
pub use linux_fault_impact::{FaultImpact, FaultImpactError};
pub use linux_fault_recovery::{FaultRecovery, RecoveryError};
pub use linux_install::{InstallError as LinuxInstallError, InstallPlan as LinuxInstallPlan, plan};
pub use linux_multi_instance::{DriverInstance, MultiInstanceError, MultiInstanceManager};
pub use linux_package::{
    LDRIVER_MAGIC, LinuxPackageError, LinuxPackageHeader, MAX_PAYLOAD, prepare_probe,
    validate_package,
};
pub use linux_recovery_pipeline::{RecoveryPipeline, RecoveryPipelineError, RecoveryReport};
pub use linux_resolver::{LinuxCandidate, ResolveError, resolve};
pub use linux_resource_manager::{Resource, ResourceError, ResourceKind, ResourceManager};
pub use linux_resource_transaction::{ResourceTransaction, ResourceTransactionError};
pub use linux_runtime::{LinuxDriverDescriptor, LinuxRuntime, LinuxRuntimeError};
pub use linux_transaction::{DriverTransaction, TransactionError, TransactionState};
pub use linux_transaction_bridge::{BridgeError as LinuxBridgeError, TransactionBridge};
pub use linux_transaction_graph::{
    GraphTransactionError as LinuxGraphTransactionError, prepare_graph_guarded,
};
pub use linux_transaction_guard::{
    GuardError as LinuxGuardError, install_plan_guarded, prepare_guarded,
};
pub use linux_transaction_orchestrator::{
    ActivationOrchestrator, OrchestratorError as LinuxOrchestratorError,
    OrchestratorState as LinuxOrchestratorState,
};
pub use pci::{
    ConfigSpace, Enumerator as PciEnumerator, MAX_PCI_FUNCTIONS, PciError as PciEnumerationError,
    PciFunction,
};
pub use pci_virtio_probe::{
    MAX_VIRTIO_PROBES, ProbeError as VirtioProbeError, VirtioDeviceKind, VirtioPciProbe,
};
pub use product_gate::{ProductGate, ProductGateError};
pub use universal::{
    DriverAbi, DriverAction, DriverError, DriverOs, DriverRequest, DriverResult, validate_request,
};
pub use virtio::{
    DESC_INDIRECT, DESC_NEXT, DESC_WRITE, VirtioDescriptor, VirtioDevice, VirtioError,
    VirtioFeatures, VirtioQueueConfig, VirtioSplitQueue, validate_chain,
};
pub use virtio_block::{
    BlockCompletion, BlockError, BlockOp, BlockRequest, MAX_REQUEST_SECTORS, SECTOR_SIZE,
    VirtioBlockConfig, VirtioBlockQueue,
};
pub use virtio_pci::{Bar, PciError, VIRTIO_VENDOR_ID, VirtioPciCapabilities, VirtioPciTransport};
pub use windows::WindowsLayer;
