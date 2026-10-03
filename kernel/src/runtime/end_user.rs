#![no_std]

use super::{CapabilitySet, RuntimeContext, RuntimeError};

pub const MAX_SERVICES: usize = 8;
pub const MAX_APPS: usize = 16;
pub const MAX_INPUT_EVENTS: usize = 128;

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServiceState {
    Declared,
    Starting,
    Running,
    Failed,
    Quarantined,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppState {
    Installed,
    Starting,
    Running,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputEvent {
    Key { code: u16, pressed: bool },
    Pointer { x: i32, y: i32, buttons: u8 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FramebufferInfo {
    pub address: u64,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub bytes_per_pixel: u8,
}

impl FramebufferInfo {
    pub const fn validate(&self) -> bool {
        if self.address == 0 || self.width == 0 || self.height == 0 || self.pitch == 0 {
            return false;
        }
        if self.bytes_per_pixel == 0 || self.bytes_per_pixel > 8 {
            return false;
        }
        let row_bytes = match self.width.checked_mul(self.bytes_per_pixel as u32) {
            Some(value) => value,
            None => return false,
        };
        if self.pitch < row_bytes {
            return false;
        }
        match (self.height as u64).checked_mul(self.pitch as u64) {
            Some(required) => required <= self.size,
            None => false,
        }
    }

    pub const fn required_bytes(&self) -> Option<u64> {
        (self.height as u64).checked_mul(self.pitch as u64)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceRecord {
    pub id: u16,
    pub state: ServiceState,
    pub capabilities: CapabilitySet,
    pub failures: u8,
    pub restart_limit: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppRecord {
    pub id: u16,
    pub state: AppState,
    pub required_capabilities: CapabilitySet,
    pub window_id: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeEvent {
    ServiceStarted(u16),
    ServiceFailed(u16),
    ServiceQuarantined(u16),
    AppStarted(u16),
    AppStopped(u16),
    Input(InputEvent),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndUserRuntimeError {
    Core(RuntimeError),
    Full,
    InvalidService,
    InvalidApp,
    InvalidTransition,
    CapabilityDenied,
    InvalidFramebuffer,
    InputQueueFull,
    InputQueueEmpty,
}

impl From<RuntimeError> for EndUserRuntimeError {
    fn from(value: RuntimeError) -> Self {
        match value {
            RuntimeError::CapabilityDenied => Self::CapabilityDenied,
            other => Self::Core(other),
        }
    }
}

pub struct EndUserRuntime {
    services: [Option<ServiceRecord>; MAX_SERVICES],
    apps: [Option<AppRecord>; MAX_APPS],
    input: [Option<InputEvent>; MAX_INPUT_EVENTS],
    input_head: usize,
    input_len: usize,
    next_window: u16,
    framebuffer: Option<FramebufferInfo>,
    desktop_ready: bool,
}

impl EndUserRuntime {
    pub const fn new() -> Self {
        Self {
            services: [None; MAX_SERVICES],
            apps: [None; MAX_APPS],
            input: [None; MAX_INPUT_EVENTS],
            input_head: 0,
            input_len: 0,
            next_window: 1,
            framebuffer: None,
            desktop_ready: false,
        }
    }

    pub const fn desktop_ready(&self) -> bool {
        self.desktop_ready
    }

    pub const fn framebuffer(&self) -> Option<FramebufferInfo> {
        self.framebuffer
    }

    pub const fn input_len(&self) -> usize {
        self.input_len
    }

    pub fn attach_framebuffer(&mut self, info: FramebufferInfo) -> Result<(), EndUserRuntimeError> {
        if !info.validate() {
            return Err(EndUserRuntimeError::InvalidFramebuffer);
        }
        self.framebuffer = Some(info);
        self.desktop_ready = true;
        Ok(())
    }

    pub fn declare_service(
        &mut self,
        id: u16,
        capabilities: CapabilitySet,
        restart_limit: u8,
    ) -> Result<(), EndUserRuntimeError> {
        if self.find_service(id).is_some() {
            return Err(EndUserRuntimeError::InvalidService);
        }
        let slot = self
            .services
            .iter()
            .position(Option::is_none)
            .ok_or(EndUserRuntimeError::Full)?;
        self.services[slot] = Some(ServiceRecord {
            id,
            state: ServiceState::Declared,
            capabilities,
            failures: 0,
            restart_limit,
        });
        Ok(())
    }

    pub fn start_service(
        &mut self,
        id: u16,
        caller: RuntimeContext,
    ) -> Result<RuntimeEvent, EndUserRuntimeError> {
        let index = self
            .find_service(id)
            .ok_or(EndUserRuntimeError::InvalidService)?;
        let record = self.services[index]
            .as_mut()
            .ok_or(EndUserRuntimeError::InvalidService)?;
        caller.require(record.capabilities)?;
        match record.state {
            ServiceState::Declared | ServiceState::Failed => {
                record.state = ServiceState::Starting;
                record.state = ServiceState::Running;
                record.failures = 0;
                Ok(RuntimeEvent::ServiceStarted(id))
            }
            _ => Err(EndUserRuntimeError::InvalidTransition),
        }
    }

    pub fn fail_service(&mut self, id: u16) -> Result<RuntimeEvent, EndUserRuntimeError> {
        let index = self
            .find_service(id)
            .ok_or(EndUserRuntimeError::InvalidService)?;
        let record = self.services[index]
            .as_mut()
            .ok_or(EndUserRuntimeError::InvalidService)?;
        if record.state != ServiceState::Starting
            && record.state != ServiceState::Running
            && record.state != ServiceState::Failed
        {
            return Err(EndUserRuntimeError::InvalidTransition);
        }
        record.failures = record.failures.saturating_add(1);
        record.state = if record.failures > record.restart_limit {
            ServiceState::Quarantined
        } else {
            ServiceState::Failed
        };
        if record.state == ServiceState::Quarantined {
            Ok(RuntimeEvent::ServiceQuarantined(id))
        } else {
            Ok(RuntimeEvent::ServiceFailed(id))
        }
    }

    pub fn install_app(
        &mut self,
        id: u16,
        required_capabilities: CapabilitySet,
    ) -> Result<(), EndUserRuntimeError> {
        if self.find_app(id).is_some() {
            return Err(EndUserRuntimeError::InvalidApp);
        }
        let slot = self
            .apps
            .iter()
            .position(Option::is_none)
            .ok_or(EndUserRuntimeError::Full)?;
        self.apps[slot] = Some(AppRecord {
            id,
            state: AppState::Installed,
            required_capabilities,
            window_id: 0,
        });
        Ok(())
    }

    pub fn start_app(
        &mut self,
        id: u16,
        caller: RuntimeContext,
    ) -> Result<RuntimeEvent, EndUserRuntimeError> {
        let index = self.find_app(id).ok_or(EndUserRuntimeError::InvalidApp)?;
        let record = self.apps[index]
            .as_mut()
            .ok_or(EndUserRuntimeError::InvalidApp)?;
        caller
            .require(record.required_capabilities)
            .map_err(EndUserRuntimeError::from)?;
        if record.state != AppState::Installed && record.state != AppState::Stopped {
            return Err(EndUserRuntimeError::InvalidTransition);
        }
        record.state = AppState::Starting;
        record.window_id = self.next_window;
        self.next_window = self.next_window.wrapping_add(1).max(1);
        record.state = AppState::Running;
        Ok(RuntimeEvent::AppStarted(id))
    }

    pub fn stop_app(&mut self, id: u16) -> Result<RuntimeEvent, EndUserRuntimeError> {
        let index = self.find_app(id).ok_or(EndUserRuntimeError::InvalidApp)?;
        let record = self.apps[index]
            .as_mut()
            .ok_or(EndUserRuntimeError::InvalidApp)?;
        if record.state != AppState::Running {
            return Err(EndUserRuntimeError::InvalidTransition);
        }
        record.state = AppState::Stopped;
        Ok(RuntimeEvent::AppStopped(id))
    }

    pub fn push_input(&mut self, event: InputEvent) -> Result<(), EndUserRuntimeError> {
        if self.input_len == MAX_INPUT_EVENTS {
            return Err(EndUserRuntimeError::InputQueueFull);
        }
        let index = (self.input_head + self.input_len) % MAX_INPUT_EVENTS;
        self.input[index] = Some(event);
        self.input_len += 1;
        Ok(())
    }

    pub fn pop_input(&mut self) -> Result<InputEvent, EndUserRuntimeError> {
        if self.input_len == 0 {
            return Err(EndUserRuntimeError::InputQueueEmpty);
        }
        let event = self.input[self.input_head]
            .take()
            .ok_or(EndUserRuntimeError::InputQueueEmpty)?;
        self.input_head = (self.input_head + 1) % MAX_INPUT_EVENTS;
        self.input_len -= 1;
        Ok(event)
    }

    pub fn service(&self, id: u16) -> Option<ServiceRecord> {
        self.find_service(id).and_then(|i| self.services[i])
    }

    pub fn app(&self, id: u16) -> Option<AppRecord> {
        self.find_app(id).and_then(|i| self.apps[i])
    }

    fn find_service(&self, id: u16) -> Option<usize> {
        self.services
            .iter()
            .position(|service| service.map(|r| r.id) == Some(id))
    }

    fn find_app(&self, id: u16) -> Option<usize> {
        self.apps
            .iter()
            .position(|app| app.map(|r| r.id) == Some(id))
    }
}

impl Default for EndUserRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_CAPS: CapabilitySet = CapabilitySet(
        CapabilitySet::PROCESS.0
            | CapabilitySet::MEMORY.0
            | CapabilitySet::IPC.0
            | CapabilitySet::DEVICE.0
            | CapabilitySet::STORAGE.0
            | CapabilitySet::NETWORK.0
            | CapabilitySet::UI.0,
    );

    #[test]
    fn framebuffer_validation_is_bounded() {
        let fb = FramebufferInfo {
            address: 0x1000,
            size: 800 * 600 * 4,
            width: 800,
            height: 600,
            pitch: 3200,
            bytes_per_pixel: 4,
        };
        assert!(fb.validate());
    }

    #[test]
    fn service_lifecycle_and_quarantine_work() {
        let mut rt = EndUserRuntime::new();
        rt.declare_service(1, CapabilitySet::IPC, 1).unwrap();
        let caller = RuntimeContext::new(CapabilitySet::IPC);
        assert_eq!(
            rt.start_service(1, caller),
            Ok(RuntimeEvent::ServiceStarted(1))
        );
        assert_eq!(rt.fail_service(1), Ok(RuntimeEvent::ServiceFailed(1)));
        assert_eq!(
            rt.start_service(1, caller),
            Ok(RuntimeEvent::ServiceStarted(1))
        );
        assert_eq!(rt.fail_service(1), Ok(RuntimeEvent::ServiceFailed(1)));
        assert_eq!(rt.fail_service(1), Ok(RuntimeEvent::ServiceQuarantined(1)));
        assert_eq!(rt.service(1).unwrap().state, ServiceState::Quarantined);
    }

    #[test]
    fn app_launch_requires_capability_and_allocates_window() {
        let mut rt = EndUserRuntime::new();
        rt.install_app(10, CapabilitySet::UI).unwrap();
        assert_eq!(
            rt.start_app(10, RuntimeContext::new(CapabilitySet::NONE)),
            Err(EndUserRuntimeError::CapabilityDenied)
        );
        assert_eq!(
            rt.start_app(10, RuntimeContext::new(CapabilitySet::UI)),
            Ok(RuntimeEvent::AppStarted(10))
        );
        assert_eq!(rt.app(10).unwrap().window_id, 1);
    }

    #[test]
    fn input_queue_is_fifo_and_bounded() {
        let mut rt = EndUserRuntime::new();
        rt.push_input(InputEvent::Key {
            code: 30,
            pressed: true,
        })
        .unwrap();
        rt.push_input(InputEvent::Pointer {
            x: 20,
            y: 30,
            buttons: 1,
        })
        .unwrap();
        assert_eq!(
            rt.pop_input().unwrap(),
            InputEvent::Key {
                code: 30,
                pressed: true
            }
        );
        assert_eq!(
            rt.pop_input().unwrap(),
            InputEvent::Pointer {
                x: 20,
                y: 30,
                buttons: 1
            }
        );
        assert_eq!(rt.pop_input(), Err(EndUserRuntimeError::InputQueueEmpty));
    }

    #[test]
    fn fullscreen_capability_set_is_constructible_without_std() {
        let ctx = RuntimeContext::new(FULL_CAPS);
        assert!(ctx.require(CapabilitySet::UI).is_ok());
        assert!(ctx.require(CapabilitySet::NETWORK).is_ok());
    }
}
