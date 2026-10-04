#![no_std]

mod transport;

pub use transport::{
    AsyncRequests, CapabilityHandle, EventQueue, HandshakeState, PendingRequest, ServiceEndpoint,
    ServiceHandshake, ServiceHello, SharedRing, TransportError, opcode_allowed, service_channel,
};

/// A small, allocation-free IPC message used by the kernel boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcMessage {
    pub sender: u64,
    pub channel: u32,
    pub payload: [u64; 4],
}
impl IpcMessage {
    pub const fn new(sender: u64, channel: u32, payload: [u64; 4]) -> Self {
        Self {
            sender,
            channel,
            payload,
        }
    }
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceChannel {
    Driverd = 1,
    Appd = 2,
    Asappd = 3,
    Ayuid = 4,
    Aweterminald = 5,
    Awebusd = 6,
    Aweupdated = 7,
}

#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpcOpcode {
    Hello = 1,
    Ping = 2,
    Start = 3,
    Stop = 4,
    Reset = 5,
    Query = 6,
    Event = 7,
    Handoff = 8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IpcEnvelope {
    pub service: ServiceChannel,
    pub opcode: IpcOpcode,
    pub request_id: u64,
    pub message: IpcMessage,
}
impl IpcEnvelope {
    pub const fn new(
        service: ServiceChannel,
        opcode: IpcOpcode,
        request_id: u64,
        message: IpcMessage,
    ) -> Self {
        Self {
            service,
            opcode,
            request_id,
            message,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IpcError {
    Full,
    Empty,
    PermissionDenied,
    InvalidSender,
    InvalidReceiver,
}

/// Fixed-capacity, capability-aware IPC endpoint. Authentication is part of
/// the enqueue/dequeue operation, not a separate convention at call sites.
pub struct AuthorizedMailbox<const N: usize> {
    slots: [Option<IpcMessage>; N],
    head: usize,
    len: usize,
    receiver: u64,
    allowed_senders: u64,
}

impl<const N: usize> AuthorizedMailbox<N> {
    pub const fn new(receiver: u64, allowed_senders: u64) -> Self {
        Self {
            slots: [None; N],
            head: 0,
            len: 0,
            receiver,
            allowed_senders,
        }
    }

    pub const fn capacity(&self) -> usize {
        N
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub const fn is_full(&self) -> bool {
        self.len == N
    }

    pub const fn receiver(&self) -> u64 {
        self.receiver
    }

    pub const fn allows_sender(&self, sender: u64) -> bool {
        sender < 64 && (self.allowed_senders & (1u64 << sender)) != 0
    }

    pub fn send(&mut self, message: IpcMessage) -> Result<(), IpcError> {
        if message.sender == 0 || !self.allows_sender(message.sender) {
            return Err(IpcError::PermissionDenied);
        }
        if self.is_full() {
            return Err(IpcError::Full);
        }
        let index = if N == 0 {
            0
        } else {
            (self.head + self.len) % N
        };
        if N == 0 {
            return Err(IpcError::Full);
        }
        self.slots[index] = Some(message);
        self.len += 1;
        Ok(())
    }

    pub fn recv(&mut self, receiver: u64) -> Result<IpcMessage, IpcError> {
        if receiver != self.receiver {
            return Err(IpcError::InvalidReceiver);
        }
        if self.is_empty() {
            return Err(IpcError::Empty);
        }
        let message = self.slots[self.head].take().ok_or(IpcError::Empty)?;
        self.head = if N == 0 { 0 } else { (self.head + 1) % N };
        self.len -= 1;
        Ok(message)
    }

    pub fn teardown_sender(&mut self, sender: u64) {
        if sender == 0 {
            return;
        }
        for slot in &mut self.slots {
            if slot.map(|msg| msg.sender == sender).unwrap_or(false) {
                *slot = None;
            }
        }
        self.compact();
    }

    fn compact(&mut self) {
        if N == 0 {
            self.head = 0;
            self.len = 0;
            return;
        }
        let mut rebuilt = [None; N];
        let mut count = 0;
        for offset in 0..N {
            let index = (self.head + offset) % N;
            if let Some(message) = self.slots[index] {
                rebuilt[count] = Some(message);
                count += 1;
            }
        }
        self.slots = rebuilt;
        self.head = 0;
        self.len = count;
    }
}

/// Legacy unauthenticated FIFO retained for internal tests and fixed-function
/// channels that do not need caller identity at the transport layer.
pub struct Mailbox<const N: usize> {
    slots: [Option<IpcMessage>; N],
    head: usize,
    len: usize,
}
impl<const N: usize> Default for Mailbox<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> Mailbox<N> {
    pub const fn new() -> Self {
        Self {
            slots: [None; N],
            head: 0,
            len: 0,
        }
    }
    pub const fn capacity(&self) -> usize {
        N
    }
    pub const fn len(&self) -> usize {
        self.len
    }
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
    pub const fn is_full(&self) -> bool {
        self.len == N
    }
    pub fn send(&mut self, message: IpcMessage) -> Result<(), IpcError> {
        if self.is_full() {
            return Err(IpcError::Full);
        }
        if N == 0 {
            return Err(IpcError::Full);
        }
        let index = (self.head + self.len) % N;
        self.slots[index] = Some(message);
        self.len += 1;
        Ok(())
    }
    pub fn recv(&mut self) -> Result<IpcMessage, IpcError> {
        if self.is_empty() {
            return Err(IpcError::Empty);
        }
        let message = self.slots[self.head].take().ok_or(IpcError::Empty)?;
        self.head = if N == 0 { 0 } else { (self.head + 1) % N };
        self.len -= 1;
        Ok(message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_fifo_and_backpressure() {
        let mut q: Mailbox<2> = Mailbox::new();
        let a = IpcMessage::new(1, 10, [1, 0, 0, 0]);
        let b = IpcMessage::new(2, 10, [2, 0, 0, 0]);
        assert!(q.send(a).is_ok());
        assert!(q.send(b).is_ok());
        assert_eq!(q.send(a), Err(IpcError::Full));
        assert_eq!(q.recv(), Ok(a));
        assert_eq!(q.recv(), Ok(b));
        assert_eq!(q.recv(), Err(IpcError::Empty));
    }

    #[test]
    fn service_channels_and_opcodes_are_stable() {
        assert_eq!(ServiceChannel::Driverd as u16, 1);
        assert_eq!(ServiceChannel::Aweupdated as u16, 7);
        assert_eq!(IpcOpcode::Hello as u16, 1);
        assert_eq!(IpcOpcode::Handoff as u16, 8);
        let envelope = IpcEnvelope::new(
            ServiceChannel::Driverd,
            IpcOpcode::Hello,
            42,
            IpcMessage::new(7, 1, [1, 2, 3, 4]),
        );
        assert_eq!(envelope.request_id, 42);
        assert_eq!(envelope.message.payload[3], 4);
    }

    #[test]
    fn authorized_mailbox_enforces_sender_and_receiver() {
        let mut q: AuthorizedMailbox<2> = AuthorizedMailbox::new(2, (1u64 << 1) | (1u64 << 3));
        assert_eq!(
            q.send(IpcMessage::new(4, 10, [0; 4])),
            Err(IpcError::PermissionDenied)
        );
        assert!(q.send(IpcMessage::new(1, 10, [1, 0, 0, 0])).is_ok());
        assert_eq!(q.recv(3), Err(IpcError::InvalidReceiver));
        assert_eq!(q.recv(2).unwrap().sender, 1);
    }

    #[test]
    fn teardown_removes_only_the_terminated_sender() {
        let mut q: AuthorizedMailbox<4> = AuthorizedMailbox::new(9, (1u64 << 1) | (1u64 << 2));
        q.send(IpcMessage::new(1, 10, [1, 0, 0, 0])).unwrap();
        q.send(IpcMessage::new(2, 10, [2, 0, 0, 0])).unwrap();
        q.send(IpcMessage::new(1, 10, [3, 0, 0, 0])).unwrap();
        q.teardown_sender(1);
        assert_eq!(q.len(), 1);
        assert_eq!(q.recv(9).unwrap().sender, 2);
    }
}
