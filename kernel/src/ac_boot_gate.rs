#![no_std]

use crate::ac_runtime::AcRuntime;
use crate::process::{ProcessId, ResourceBudget, context::CpuContext};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BootStage {
    Reset,
    CpuValidated,
    MemoryValidated,
    KernelReady,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AcGateError {
    WrongStage,
    InvalidCpu,
    InvalidMemory,
}

/// Contract-level A-C bring-up gate. It validates supplied CPU/memory metadata
/// only; it is not a hardware discovery, page-table setup or boot initialization
/// sequence. Admission remains explicit and fail-closed.
pub struct AcBootGate {
    stage: BootStage,
}

impl Default for AcBootGate {
    fn default() -> Self {
        Self::new()
    }
}

impl AcBootGate {
    pub const fn new() -> Self {
        Self {
            stage: BootStage::Reset,
        }
    }

    pub const fn stage(&self) -> BootStage {
        self.stage
    }

    pub fn validate_cpu(
        &mut self,
        cpu_count: usize,
        stack_alignment: u64,
    ) -> Result<(), AcGateError> {
        if self.stage != BootStage::Reset || cpu_count == 0 || stack_alignment & 0xf != 0 {
            return Err(AcGateError::InvalidCpu);
        }
        self.stage = BootStage::CpuValidated;
        Ok(())
    }

    pub fn validate_memory(&mut self, page_size: usize, bytes: u64) -> Result<(), AcGateError> {
        if self.stage != BootStage::CpuValidated || page_size != 4096 || bytes < page_size as u64 {
            return Err(AcGateError::InvalidMemory);
        }
        self.stage = BootStage::MemoryValidated;
        Ok(())
    }

    pub fn activate(&mut self) -> Result<(), AcGateError> {
        if self.stage != BootStage::MemoryValidated {
            return Err(AcGateError::WrongStage);
        }
        self.stage = BootStage::KernelReady;
        Ok(())
    }

    pub fn admit_process<const N: usize>(
        &self,
        runtime: &mut AcRuntime<N>,
        budget: ResourceBudget,
        cpu: CpuContext,
    ) -> Result<ProcessId, AcGateError> {
        if self.stage != BootStage::KernelReady {
            return Err(AcGateError::WrongStage);
        }
        runtime
            .create_process(budget, cpu)
            .map_err(|_| AcGateError::WrongStage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bringup_is_ordered_and_fail_closed() {
        let mut gate = AcBootGate::new();
        assert_eq!(gate.activate(), Err(AcGateError::WrongStage));
        gate.validate_cpu(2, 0x1000).unwrap();
        gate.validate_memory(4096, 16 * 1024).unwrap();
        gate.activate().unwrap();
        assert_eq!(gate.stage(), BootStage::KernelReady);
    }

    #[test]
    fn invalid_cpu_and_memory_never_advance_the_gate() {
        let mut gate = AcBootGate::new();
        assert_eq!(gate.validate_cpu(0, 0x1000), Err(AcGateError::InvalidCpu));
        assert_eq!(gate.stage(), BootStage::Reset);
        gate.validate_cpu(1, 0x1000).unwrap();
        assert_eq!(
            gate.validate_memory(2048, 4096),
            Err(AcGateError::InvalidMemory)
        );
        assert_eq!(gate.stage(), BootStage::CpuValidated);
    }
}
