#![no_std]

use super::context::ProcessContext;
use super::context_switch::SwitchFrame;
use super::dispatch::{DispatchError, DispatchTarget, prepare_dispatch};
use super::scheduler::{Scheduler, SchedulerError};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuntimeDispatchError {
    Scheduler(SchedulerError),
    Dispatch(DispatchError),
}
pub fn prepare_runtime_switch<const N: usize>(
    scheduler: &mut Scheduler<N>,
    current: &SwitchFrame,
    contexts: &[Option<ProcessContext>; N],
) -> Result<DispatchTarget, RuntimeDispatchError> {
    let target = scheduler
        .prepare_next(contexts)
        .map_err(RuntimeDispatchError::Scheduler)?;
    prepare_dispatch(current, &target).map_err(RuntimeDispatchError::Dispatch)?;
    Ok(target)
}
