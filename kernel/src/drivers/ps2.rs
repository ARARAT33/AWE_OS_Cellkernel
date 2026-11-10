#![no_std]

//! PS/2 (i8042) keyboard and mouse controller driver.
//!
//! The controller owns a bounded event queue and two stream decoders: a
//! set-2 scancode keyboard decoder and a three-byte PS/2 mouse decoder.
//! Port I/O is only compiled on x86_64; other architectures fail closed
//! with ['Ps2Error::Unsupported']. No interrupt handling is claimed: the
//! controller is polled explicitly by the caller.

#[cfg(target_arch = "x86_64")]
use crate::arch::x86_64::{io_in8, io_out8};

/// PS/2 data port (0x60).
pub const DATA_PORT: u16 = 0x60;
/// PS/2 status port (0x64, read).
pub const STATUS_PORT: u16 = 0x64;
/// PS/2 command port (0x64, write).
pub const COMMAND_PORT: u16 = 0x64;
/// Maximum number of buffered input events.
pub const MAX_EVENTS: usize = 128;

/// Errors surfaced by the PS/2 controller and its decoders.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ps2Error {
    /// The controller did not become ready within the bounded spin limit.
    Timeout,
    /// The controller was used before a successful initialization.
    Controller,
    /// The bounded event queue is full.
    QueueFull,
    /// A mouse packet was structurally invalid (overflow bits set).
    InvalidPacket,
    /// The current architecture has no PS/2 port I/O support.
    Unsupported,
}

/// Logical key codes decoded from set-2 scancodes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyCode {
    Escape,
    Enter,
    Backspace,
    Tab,
    Space,
    Left,
    Right,
    Up,
    Down,
    /// A printable-key scancode; the payload is the raw scancode.
    Character(u8),
    /// An unmapped scancode; the payload is the raw scancode.
    Unknown(u8),
}

/// A decoded input event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ps2Event {
    /// A key was pressed or released.
    Key { code: KeyCode, pressed: bool },
    /// Relative pointer motion and button state.
    Pointer { dx: i16, dy: i16, buttons: u8 },
}

/// Bounded FIFO of input events.
#[derive(Clone, Copy, Debug)]
pub struct EventQueue {
    slots: [Option<Ps2Event>; MAX_EVENTS],
    head: usize,
    len: usize,
}

impl EventQueue {
    pub const fn new() -> Self {
        Self {
            slots: [None; MAX_EVENTS],
            head: 0,
            len: 0,
        }
    }

    /// Number of buffered events.
    pub const fn len(&self) -> usize {
        self.len
    }

    /// True when no events are buffered.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Pushes an event; fails closed with 'Ps2Error::QueueFull' when full.
    pub fn push(&mut self, event: Ps2Event) -> Result<(), Ps2Error> {
        if self.len == MAX_EVENTS {
            return Err(Ps2Error::QueueFull);
        }
        let index = (self.head + self.len) % MAX_EVENTS;
        self.slots[index] = Some(event);
        self.len += 1;
        Ok(())
    }

    /// Pops the oldest event.
    pub fn pop(&mut self) -> Option<Ps2Event> {
        if self.len == 0 {
            return None;
        }
        let event = self.slots[self.head].take();
        self.head = (self.head + 1) % MAX_EVENTS;
        self.len -= 1;
        event
    }
}

impl Default for EventQueue {
    fn default() -> Self {
        Self::new()
    }
}

/// Set-2 scancode stream decoder for a single keyboard.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyboardDecoder {
    extended: bool,
    break_code: bool,
}

impl KeyboardDecoder {
    pub const fn new() -> Self {
        Self {
            extended: false,
            break_code: false,
        }
    }

    /// Feeds one scancode byte; returns a complete event or None while a
    /// multi-byte sequence (0xE0 prefix, 0xF0 break prefix) is in flight.
    pub fn feed(&mut self, byte: u8) -> Option<Ps2Event> {
        match byte {
            0xE0 => {
                self.extended = true;
                None
            }
            0xF0 => {
                self.break_code = true;
                None
            }
            code => {
                let pressed = !self.break_code;
                let mapped = if self.extended {
                    match code {
                        0x4B => KeyCode::Left,
                        0x4D => KeyCode::Right,
                        0x48 => KeyCode::Up,
                        0x50 => KeyCode::Down,
                        other => KeyCode::Unknown(other),
                    }
                } else {
                    match code {
                        0x01 => KeyCode::Escape,
                        0x0D => KeyCode::Tab,
                        0x1C => KeyCode::Enter,
                        0x0E => KeyCode::Backspace,
                        0x39 => KeyCode::Space,
                        0x10..=0x35 => KeyCode::Character(code),
                        other => KeyCode::Unknown(other),
                    }
                };
                self.extended = false;
                self.break_code = false;
                Some(Ps2Event::Key {
                    code: mapped,
                    pressed,
                })
            }
        }
    }
}

/// Three-byte PS/2 mouse packet decoder.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MouseDecoder {
    packet: [u8; 3],
    index: usize,
}

impl MouseDecoder {
    pub const fn new() -> Self {
        Self {
            packet: [0; 3],
            index: 0,
        }
    }

    /// Feeds one byte from the mouse stream. The first byte must carry the
    /// always-one bit 3; a byte without it is treated as desynchronization
    /// and dropped. Overflow bits 6/7 on the first byte are rejected with
    /// 'Ps2Error::InvalidPacket'. A complete packet is returned as a
    /// 'Ps2Event::Pointer'; movement bytes are nine-bit two's complement
    /// and are sign-extended through i8.
    pub fn feed(&mut self, byte: u8) -> Result<Option<Ps2Event>, Ps2Error> {
        if self.index == 0 {
            if byte & 0x08 == 0 {
                return Ok(None);
            }
            if byte & 0xC0 != 0 {
                return Err(Ps2Error::InvalidPacket);
            }
        }
        self.packet[self.index] = byte;
        self.index += 1;
        if self.index < 3 {
            return Ok(None);
        }
        self.index = 0;
        let flags = self.packet[0];
        let dx = self.packet[1] as i8 as i16;
        // PS/2 mice report positive Y as upward motion.
        let dy = -(self.packet[2] as i8 as i16);
        Ok(Some(Ps2Event::Pointer {
            dx,
            dy,
            buttons: flags & 0x07,
        }))
    }
}

/// A polled PS/2 controller with its decoders and bounded event queue.
pub struct Controller {
    /// Buffered decoded input events.
    pub events: EventQueue,
    /// Keyboard scancode decoder.
    pub keyboard: KeyboardDecoder,
    /// Mouse packet decoder.
    pub mouse: MouseDecoder,
    /// True after a successful 'Controller::init'.
    pub initialized: bool,
}

impl Controller {
    pub const fn new() -> Self {
        Self {
            events: EventQueue::new(),
            keyboard: KeyboardDecoder::new(),
            mouse: MouseDecoder::new(),
            initialized: false,
        }
    }

    /// Drains the controller output buffer and enables the first PS/2 port.
    ///
    /// # Safety
    /// Must only be called when port I/O to the PS/2 controller is valid
    /// (x86_64 with the i8042 present) and with interrupts disabled.
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn init(&mut self) -> Result<(), Ps2Error> {
        const LIMIT: usize = 100_000;
        let mut spins = 0;
        while unsafe { io_in8(STATUS_PORT) } & 0x01 != 0 {
            let _ = unsafe { io_in8(DATA_PORT) };
            spins += 1;
            if spins == LIMIT {
                return Err(Ps2Error::Timeout);
            }
        }
        unsafe { io_out8(COMMAND_PORT, 0xAE) };
        self.initialized = true;
        Ok(())
    }

    #[cfg(not(target_arch = "x86_64"))]
    pub unsafe fn init(&mut self) -> Result<(), Ps2Error> {
        Err(Ps2Error::Unsupported)
    }

    /// Polls at most 'max_bytes' bytes from the data port and decodes them
    /// into the event queue. Returns the number of bytes consumed. A
    /// 'Ps2Error::QueueFull' return means at least one decoded event was
    /// dropped because the bounded queue filled up.
    ///
    /// # Safety
    /// Must only be called after a successful 'Controller::init', with
    /// port I/O valid and interrupts disabled.
    #[cfg(target_arch = "x86_64")]
    pub unsafe fn poll(&mut self, max_bytes: usize) -> Result<usize, Ps2Error> {
        if !self.initialized {
            return Err(Ps2Error::Controller);
        }
        let mut read = 0;
        while read < max_bytes {
            let status = unsafe { io_in8(STATUS_PORT) };
            if status & 0x01 == 0 {
                break;
            }
            let byte = unsafe { io_in8(DATA_PORT) };
            if status & 0x20 != 0 {
                if let Some(event) = self.mouse.feed(byte)? {
                    self.events.push(event)?;
                }
            } else if let Some(event) = self.keyboard.feed(byte) {
                self.events.push(event)?;
            }
            read += 1;
        }
        Ok(read)
    }

    #[cfg(not(target_arch = "x86_64"))]
    pub unsafe fn poll(&mut self, _max_bytes: usize) -> Result<usize, Ps2Error> {
        Err(Ps2Error::Unsupported)
    }
}

impl Default for Controller {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_decodes_press_release_and_extended() {
        let mut d = KeyboardDecoder::new();
        assert_eq!(
            d.feed(0x1C),
            Some(Ps2Event::Key {
                code: KeyCode::Enter,
                pressed: true
            })
        );
        assert_eq!(d.feed(0xF0), None);
        assert_eq!(
            d.feed(0x1C),
            Some(Ps2Event::Key {
                code: KeyCode::Enter,
                pressed: false
            })
        );
        assert_eq!(d.feed(0xE0), None);
        assert_eq!(
            d.feed(0x4B),
            Some(Ps2Event::Key {
                code: KeyCode::Left,
                pressed: true
            })
        );
    }

    #[test]
    fn keyboard_decodes_extended_break_sequence() {
        let mut d = KeyboardDecoder::new();
        assert_eq!(d.feed(0xE0), None);
        assert_eq!(d.feed(0xF0), None);
        assert_eq!(
            d.feed(0x4D),
            Some(Ps2Event::Key {
                code: KeyCode::Right,
                pressed: false
            })
        );
        // Prefix state must not leak into the next scancode.
        assert_eq!(
            d.feed(0x39),
            Some(Ps2Event::Key {
                code: KeyCode::Space,
                pressed: true
            })
        );
    }

    #[test]
    fn mouse_decodes_signed_motion() {
        let mut d = MouseDecoder::new();
        assert_eq!(d.feed(0x09).unwrap(), None);
        assert_eq!(d.feed(0xFE).unwrap(), None);
        assert_eq!(
            d.feed(0x02).unwrap(),
            Some(Ps2Event::Pointer {
                dx: -2,
                dy: -2,
                buttons: 1
            })
        );
    }

    #[test]
    fn mouse_motion_wraps_correctly_at_sign_boundary() {
        let mut d = MouseDecoder::new();
        // Right two units: sign bit clear, value 0x02.
        assert_eq!(d.feed(0x08).unwrap(), None);
        assert_eq!(d.feed(0x02).unwrap(), None);
        assert_eq!(
            d.feed(0x02).unwrap(),
            Some(Ps2Event::Pointer {
                dx: 2,
                dy: -2,
                buttons: 0
            })
        );
        // Left two units: 0xFE sign-extends to -2 through i8.
        assert_eq!(d.feed(0x08).unwrap(), None);
        assert_eq!(d.feed(0xFE).unwrap(), None);
        assert_eq!(
            d.feed(0xFE).unwrap(),
            Some(Ps2Event::Pointer {
                dx: -2,
                dy: 2,
                buttons: 0
            })
        );
    }

    #[test]
    fn malformed_mouse_packet_is_rejected() {
        let mut d = MouseDecoder::new();
        assert_eq!(d.feed(0xC8).unwrap_err(), Ps2Error::InvalidPacket);
        // The decoder must resynchronize cleanly after a rejected packet.
        assert_eq!(d.feed(0x08).unwrap(), None);
        assert_eq!(d.feed(0x00).unwrap(), None);
        assert_eq!(
            d.feed(0x00).unwrap(),
            Some(Ps2Event::Pointer {
                dx: 0,
                dy: 0,
                buttons: 0
            })
        );
    }

    #[test]
    fn mouse_resyncs_on_desynchronized_stream() {
        let mut d = MouseDecoder::new();
        // Bytes without the always-one bit 3 are dropped until a valid
        // first byte appears.
        assert_eq!(d.feed(0x00).unwrap(), None);
        assert_eq!(d.feed(0x07).unwrap(), None);
        assert_eq!(d.index, 0);
        assert_eq!(d.feed(0x08).unwrap(), None);
        assert_eq!(d.index, 1);
    }

    #[test]
    fn queue_is_fifo_and_bounded() {
        let mut q = EventQueue::new();
        let event = Ps2Event::Key {
            code: KeyCode::Escape,
            pressed: true,
        };
        q.push(event).unwrap();
        assert_eq!(q.len(), 1);
        assert_eq!(q.pop(), Some(event));
        assert!(q.is_empty());
        assert_eq!(q.pop(), None);
        for _ in 0..MAX_EVENTS {
            q.push(event).unwrap();
        }
        assert_eq!(q.push(event), Err(Ps2Error::QueueFull));
        // The full queue must not corrupt its contents.
        assert_eq!(q.pop(), Some(event));
        assert_eq!(q.len(), MAX_EVENTS - 1);
    }

    #[test]
    fn queue_wraps_around_the_ring() {
        let mut q = EventQueue::new();
        let first = Ps2Event::Key {
            code: KeyCode::Tab,
            pressed: true,
        };
        let second = Ps2Event::Key {
            code: KeyCode::Tab,
            pressed: false,
        };
        q.push(first).unwrap();
        q.push(second).unwrap();
        assert_eq!(q.pop(), Some(first));
        q.push(first).unwrap();
        assert_eq!(q.pop(), Some(second));
        assert_eq!(q.pop(), Some(first));
        assert_eq!(q.pop(), None);
    }
}
