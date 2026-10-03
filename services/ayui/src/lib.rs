//! Bounded AYUI desktop compositor, software rendering engine, widget system,
//! desktop shell, terminal emulator backend, multi-monitor, clipboard, accessibility,
//! notifications, themes, GPU path, and process execution stack.

#![no_std]

pub const MAX_WINDOWS: usize = 64;
pub const MAX_EVENTS: usize = 128;
pub const MAX_WIDTH: u32 = 16_384;
pub const MAX_HEIGHT: u32 = 16_384;
pub const MAX_NOTIFICATIONS: usize = 16;
pub const MAX_MONITORS: usize = 4;
pub const MAX_CLIPBOARD_LEN: usize = 1024;
pub const TERM_ROWS: usize = 25;
pub const TERM_COLS: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn valid(self) -> bool {
        self.width > 0 && self.height > 0 && self.width <= MAX_WIDTH && self.height <= MAX_HEIGHT
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x
            && px < self.x + (self.width as i32)
            && py >= self.y
            && py < self.y + (self.height as i32)
    }

    pub fn snap_to_grid(&mut self, grid_size: u32) {
        if grid_size > 0 {
            let gs = grid_size as i32;
            self.x = ((self.x + gs / 2) / gs) * gs;
            self.y = ((self.y + gs / 2) / gs) * gs;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const BLACK: Self = Self {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const WHITE: Self = Self {
        r: 255,
        g: 255,
        b: 255,
        a: 255,
    };
    pub const RED: Self = Self {
        r: 255,
        g: 0,
        b: 0,
        a: 255,
    };
    pub const GREEN: Self = Self {
        r: 0,
        g: 255,
        b: 0,
        a: 255,
    };
    pub const BLUE: Self = Self {
        r: 0,
        g: 120,
        b: 215,
        a: 255,
    };
    pub const DARK_GRAY: Self = Self {
        r: 32,
        g: 32,
        b: 32,
        a: 255,
    };
    pub const LIGHT_GRAY: Self = Self {
        r: 192,
        g: 192,
        b: 192,
        a: 255,
    };
    pub const YELLOW: Self = Self {
        r: 255,
        g: 255,
        b: 0,
        a: 255,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AppType {
    Launcher,
    Shell,
    Terminal,
    FileManager,
    Settings,
    SystemMonitor,
    TextEditor,
    PackageCenter,
    UpdateCenter,
    Recovery,
    Generic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    pub bounds: Rect,
    pub visible: bool,
    pub focused: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub bg_color: Color,
    pub app_type: AppType,
    pub title_len: usize,
    pub title_bytes: [u8; 32],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputEvent {
    Pointer { x: i32, y: i32, buttons: u8 },
    Key { code: u16, pressed: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiError {
    Full,
    InvalidRect,
    InvalidWindow,
    InvalidEvent,
    BufferTooSmall,
}

/// Theme settings for AYUI
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub bg_color: Color,
    pub window_border: Color,
    pub titlebar_active: Color,
    pub titlebar_inactive: Color,
    pub text_color: Color,
    pub accent_color: Color,
    pub dark_mode: bool,
}

impl Theme {
    pub const fn default_dark() -> Self {
        Self {
            bg_color: Color {
                r: 24,
                g: 28,
                b: 36,
                a: 255,
            },
            window_border: Color {
                r: 60,
                g: 64,
                b: 72,
                a: 255,
            },
            titlebar_active: Color {
                r: 0,
                g: 120,
                b: 215,
                a: 255,
            },
            titlebar_inactive: Color {
                r: 45,
                g: 50,
                b: 60,
                a: 255,
            },
            text_color: Color::WHITE,
            accent_color: Color {
                r: 0,
                g: 153,
                b: 255,
                a: 255,
            },
            dark_mode: true,
        }
    }

    pub const fn default_light() -> Self {
        Self {
            bg_color: Color {
                r: 240,
                g: 242,
                b: 245,
                a: 255,
            },
            window_border: Color {
                r: 200,
                g: 205,
                b: 212,
                a: 255,
            },
            titlebar_active: Color {
                r: 0,
                g: 120,
                b: 215,
                a: 255,
            },
            titlebar_inactive: Color {
                r: 210,
                g: 215,
                b: 220,
                a: 255,
            },
            text_color: Color::BLACK,
            accent_color: Color {
                r: 0,
                g: 120,
                b: 215,
                a: 255,
            },
            dark_mode: false,
        }
    }
}

/// Accessibility Settings
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Accessibility {
    pub high_contrast: bool,
    pub large_text: bool,
    pub screen_reader_active: bool,
    pub sticky_keys: bool,
}

impl Accessibility {
    pub const fn new() -> Self {
        Self {
            high_contrast: false,
            large_text: false,
            screen_reader_active: false,
            sticky_keys: false,
        }
    }
}

impl Default for Accessibility {
    fn default() -> Self {
        Self::new()
    }
}

/// Clipboard Manager
pub struct Clipboard {
    data: [u8; MAX_CLIPBOARD_LEN],
    len: usize,
}

impl Clipboard {
    pub const fn new() -> Self {
        Self {
            data: [0; MAX_CLIPBOARD_LEN],
            len: 0,
        }
    }

    pub fn copy(&mut self, src: &[u8]) -> Result<(), UiError> {
        if src.len() > MAX_CLIPBOARD_LEN {
            return Err(UiError::BufferTooSmall);
        }
        self.data[..src.len()].copy_from_slice(src);
        self.len = src.len();
        Ok(())
    }

    pub fn paste<'a>(&'a self, dst: &'a mut [u8]) -> usize {
        let copy_len = self.len.min(dst.len());
        dst[..copy_len].copy_from_slice(&self.data[..copy_len]);
        copy_len
    }

    pub fn text(&self) -> &[u8] {
        &self.data[..self.len]
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

/// Display Monitor Info for Multi-Monitor Architecture
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Monitor {
    pub id: u8,
    pub bounds: Rect,
    pub primary: bool,
    pub gpu_accelerated: bool,
}

/// 8x8 Bitmap Font table covering ASCII characters 0x20 (' ') through 0x7E ('~').
pub static FONT_8X8: [[u8; 8]; 95] = [
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // ' ' (32)
    [0x18, 0x18, 0x18, 0x18, 0x18, 0x00, 0x18, 0x00], // '!' (33)
    [0x66, 0x66, 0x66, 0x00, 0x00, 0x00, 0x00, 0x00], // '"' (34)
    [0x66, 0x66, 0xFF, 0x66, 0xFF, 0x66, 0x66, 0x00], // '#' (35)
    [0x18, 0x7E, 0x18, 0x3C, 0x18, 0x7E, 0x18, 0x00], // '$' (36)
    [0x00, 0x63, 0x6C, 0x18, 0x30, 0x66, 0x43, 0x00], // '%' (37)
    [0x38, 0x6C, 0x38, 0x76, 0xDC, 0xCC, 0x76, 0x00], // '&' (38)
    [0x18, 0x18, 0x30, 0x00, 0x00, 0x00, 0x00, 0x00], // '\'' (39)
    [0x0C, 0x18, 0x30, 0x30, 0x30, 0x18, 0x0C, 0x00], // '(' (40)
    [0x30, 0x18, 0x0C, 0x0C, 0x0C, 0x18, 0x30, 0x00], // ')' (41)
    [0x00, 0x66, 0x3C, 0xFF, 0x3C, 0x66, 0x00, 0x00], // '*' (42)
    [0x00, 0x18, 0x18, 0x7E, 0x18, 0x18, 0x00, 0x00], // '+' (43)
    [0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x30, 0x00], // ',' (44)
    [0x00, 0x00, 0x00, 0x7E, 0x00, 0x00, 0x00, 0x00], // '-' (45)
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x18, 0x18, 0x00], // '.' (46)
    [0x06, 0x0C, 0x18, 0x30, 0x60, 0xC0, 0x80, 0x00], // '/' (47)
    [0x3C, 0x66, 0x6E, 0x76, 0x66, 0x66, 0x3C, 0x00], // '0' (48)
    [0x18, 0x38, 0x18, 0x18, 0x18, 0x18, 0x7E, 0x00], // '1' (49)
    [0x3C, 0x66, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0x00], // '2' (50)
    [0x3C, 0x66, 0x06, 0x1C, 0x06, 0x66, 0x3C, 0x00], // '3' (51)
    [0x0C, 0x1C, 0x3C, 0x6C, 0xFE, 0x0C, 0x0C, 0x00], // '4' (52)
    [0x7E, 0x60, 0x7C, 0x06, 0x06, 0x66, 0x3C, 0x00], // '5' (53)
    [0x3C, 0x66, 0x60, 0x7C, 0x66, 0x66, 0x3C, 0x00], // '6' (54)
    [0x7E, 0x66, 0x0C, 0x18, 0x18, 0x18, 0x18, 0x00], // '7' (55)
    [0x3C, 0x66, 0x66, 0x3C, 0x66, 0x66, 0x3C, 0x00], // '8' (56)
    [0x3C, 0x66, 0x66, 0x3E, 0x06, 0x66, 0x3C, 0x00], // '9' (57)
    [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x00, 0x00], // ':' (58)
    [0x00, 0x18, 0x18, 0x00, 0x18, 0x18, 0x30, 0x00], // ';' (59)
    [0x0C, 0x18, 0x30, 0x60, 0x30, 0x18, 0x0C, 0x00], // '<' (60)
    [0x00, 0x00, 0x7E, 0x00, 0x7E, 0x00, 0x00, 0x00], // '=' (61)
    [0x30, 0x18, 0x0C, 0x06, 0x0C, 0x18, 0x30, 0x00], // '>' (62)
    [0x3C, 0x66, 0x0C, 0x18, 0x18, 0x00, 0x18, 0x00], // '?' (63)
    [0x3C, 0x66, 0x6E, 0x6E, 0x60, 0x62, 0x3C, 0x00], // '@' (64)
    [0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x00], // 'A' (65)
    [0x7C, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x7C, 0x00], // 'B' (66)
    [0x3C, 0x66, 0x60, 0x60, 0x60, 0x66, 0x3C, 0x00], // 'C' (67)
    [0x78, 0x6C, 0x66, 0x66, 0x66, 0x6C, 0x78, 0x00], // 'D' (68)
    [0x7E, 0x60, 0x60, 0x78, 0x60, 0x60, 0x7E, 0x00], // 'E' (69)
    [0x7E, 0x60, 0x60, 0x78, 0x60, 0x60, 0x60, 0x00], // 'F' (70)
    [0x3C, 0x66, 0x60, 0x6E, 0x66, 0x66, 0x3E, 0x00], // 'G' (71)
    [0x66, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x66, 0x00], // 'H' (72)
    [0x3C, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0x00], // 'I' (73)
    [0x1E, 0x0C, 0x0C, 0x0C, 0x0C, 0x6C, 0x38, 0x00], // 'J' (74)
    [0x66, 0x6C, 0x78, 0x70, 0x78, 0x6C, 0x66, 0x00], // 'K' (75)
    [0x60, 0x60, 0x60, 0x60, 0x60, 0x60, 0x7E, 0x00], // 'L' (76)
    [0x63, 0x77, 0x7F, 0x6B, 0x63, 0x63, 0x63, 0x00], // 'M' (77)
    [0x66, 0x76, 0x7E, 0x7E, 0x6E, 0x66, 0x66, 0x00], // 'N' (78)
    [0x3C, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00], // 'O' (79)
    [0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60, 0x60, 0x00], // 'P' (80)
    [0x3C, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x0E, 0x00], // 'Q' (81)
    [0x7C, 0x66, 0x66, 0x7C, 0x70, 0x68, 0x66, 0x00], // 'R' (82)
    [0x3C, 0x66, 0x60, 0x3C, 0x06, 0x66, 0x3C, 0x00], // 'S' (83)
    [0x7E, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00], // 'T' (84)
    [0x66, 0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x00], // 'U' (85)
    [0x66, 0x66, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00], // 'V' (86)
    [0x63, 0x63, 0x63, 0x6B, 0x7F, 0x77, 0x63, 0x00], // 'W' (87)
    [0x66, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x66, 0x00], // 'X' (88)
    [0x66, 0x66, 0x66, 0x3C, 0x18, 0x18, 0x18, 0x00], // 'Y' (89)
    [0x7E, 0x06, 0x0C, 0x18, 0x30, 0x60, 0x7E, 0x00], // 'Z' (90)
    [0x3C, 0x30, 0x30, 0x30, 0x30, 0x30, 0x3C, 0x00], // '[' (91)
    [0x80, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x00], // '\' (92)
    [0x3C, 0x0C, 0x0C, 0x0C, 0x0C, 0x0C, 0x3C, 0x00], // ']' (93)
    [0x18, 0x3C, 0x66, 0x00, 0x00, 0x00, 0x00, 0x00], // '^' (94)
    [0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x00], // '_' (95)
    [0x30, 0x18, 0x0C, 0x00, 0x00, 0x00, 0x00, 0x00], // '`' (96)
    [0x00, 0x00, 0x3C, 0x06, 0x3E, 0x66, 0x3E, 0x00], // 'a' (97)
    [0x60, 0x60, 0x7C, 0x66, 0x66, 0x66, 0x7C, 0x00], // 'b' (98)
    [0x00, 0x00, 0x3C, 0x66, 0x60, 0x66, 0x3C, 0x00], // 'c' (99)
    [0x06, 0x06, 0x3E, 0x66, 0x66, 0x66, 0x3E, 0x00], // 'd' (100)
    [0x00, 0x00, 0x3C, 0x66, 0x7E, 0x60, 0x3C, 0x00], // 'e' (101)
    [0x1C, 0x30, 0x7C, 0x30, 0x30, 0x30, 0x30, 0x00], // 'f' (102)
    [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x7C], // 'g' (103)
    [0x60, 0x60, 0x6C, 0x76, 0x66, 0x66, 0x66, 0x00], // 'h' (104)
    [0x18, 0x00, 0x38, 0x18, 0x18, 0x18, 0x3C, 0x00], // 'i' (105)
    [0x06, 0x00, 0x0E, 0x06, 0x06, 0x06, 0x66, 0x3C], // 'j' (106)
    [0x60, 0x60, 0x66, 0x6C, 0x78, 0x6C, 0x66, 0x00], // 'k' (107)
    [0x38, 0x18, 0x18, 0x18, 0x18, 0x18, 0x3C, 0x00], // 'l' (108)
    [0x00, 0x00, 0x66, 0x7F, 0x7F, 0x6B, 0x63, 0x00], // 'm' (109)
    [0x00, 0x00, 0x7C, 0x66, 0x66, 0x66, 0x66, 0x00], // 'n' (110)
    [0x00, 0x00, 0x3C, 0x66, 0x66, 0x66, 0x3C, 0x00], // 'o' (111)
    [0x00, 0x00, 0x7C, 0x66, 0x66, 0x7C, 0x60, 0x60], // 'p' (112)
    [0x00, 0x00, 0x3E, 0x66, 0x66, 0x3E, 0x06, 0x06], // 'q' (113)
    [0x00, 0x00, 0x6C, 0x76, 0x60, 0x60, 0x60, 0x00], // 'r' (114)
    [0x00, 0x00, 0x3E, 0x60, 0x3C, 0x06, 0x7C, 0x00], // 's' (115)
    [0x18, 0x18, 0x7E, 0x18, 0x18, 0x18, 0x0E, 0x00], // 't' (116)
    [0x00, 0x00, 0x66, 0x66, 0x66, 0x66, 0x3E, 0x00], // 'u' (117)
    [0x00, 0x00, 0x66, 0x66, 0x66, 0x3C, 0x18, 0x00], // 'v' (118)
    [0x00, 0x00, 0x63, 0x6B, 0x7F, 0x3E, 0x36, 0x00], // 'w' (119)
    [0x00, 0x00, 0x66, 0x3C, 0x18, 0x3C, 0x66, 0x00], // 'x' (120)
    [0x00, 0x00, 0x66, 0x66, 0x66, 0x3E, 0x06, 0x7C], // 'y' (121)
    [0x00, 0x00, 0x7E, 0x0C, 0x18, 0x30, 0x7E, 0x00], // 'z' (122)
    [0x0E, 0x18, 0x18, 0x70, 0x18, 0x18, 0x0E, 0x00], // '{' (123)
    [0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x18, 0x00], // '|' (124)
    [0x70, 0x18, 0x18, 0x0E, 0x18, 0x18, 0x70, 0x00], // '}' (125)
    [0x32, 0x4C, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], // '~' (126)
];

/// Returns the 8x8 font glyph row bitmasks for ASCII character `ch`.
pub fn get_glyph(ch: u8) -> &'static [u8; 8] {
    if (32..=126).contains(&ch) {
        &FONT_8X8[(ch - 32) as usize]
    } else {
        &FONT_8X8[63 - 32] // Fallback '?'
    }
}

/// Software / GPU Framebuffer for display composition.
#[derive(Debug)]
pub struct Framebuffer<'a> {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub buffer: &'a mut [u8],
    pub gpu_accel: bool,
}

impl<'a> Framebuffer<'a> {
    pub fn clear(&mut self, color: Color) {
        self.fill_rect(
            Rect {
                x: 0,
                y: 0,
                width: self.width,
                height: self.height,
            },
            color,
        );
    }

    pub fn draw_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || (x as u32) >= self.width || (y as u32) >= self.height {
            return;
        }
        let offset = (((y as u32) * self.stride + (x as u32)) * 4) as usize;
        if offset + 3 < self.buffer.len() {
            self.buffer[offset] = color.b;
            self.buffer[offset + 1] = color.g;
            self.buffer[offset + 2] = color.r;
            self.buffer[offset + 3] = color.a;
        }
    }

    pub fn draw_hline(&mut self, x: i32, y: i32, length: u32, color: Color) {
        if y < 0 || (y as u32) >= self.height || length == 0 {
            return;
        }
        let start_x = x.max(0) as u32;
        let end_x = ((x + length as i32).max(0) as u32).min(self.width);
        for px in start_x..end_x {
            self.draw_pixel(px as i32, y, color);
        }
    }

    pub fn draw_vline(&mut self, x: i32, y: i32, length: u32, color: Color) {
        if x < 0 || (x as u32) >= self.width || length == 0 {
            return;
        }
        let start_y = y.max(0) as u32;
        let end_y = ((y + length as i32).max(0) as u32).min(self.height);
        for py in start_y..end_y {
            self.draw_pixel(x, py as i32, color);
        }
    }

    pub fn draw_line(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, color: Color) {
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut cx = x0;
        let mut cy = y0;

        loop {
            self.draw_pixel(cx, cy, color);
            if cx == x1 && cy == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                cx += sx;
            }
            if e2 <= dx {
                err += dx;
                cy += sy;
            }
        }
    }

    pub fn draw_rect(&mut self, rect: Rect, color: Color) {
        if !rect.valid() {
            return;
        }
        self.draw_hline(rect.x, rect.y, rect.width, color);
        self.draw_hline(rect.x, rect.y + (rect.height as i32) - 1, rect.width, color);
        self.draw_vline(rect.x, rect.y, rect.height, color);
        self.draw_vline(rect.x + (rect.width as i32) - 1, rect.y, rect.height, color);
    }

    pub fn fill_rect(&mut self, rect: Rect, color: Color) {
        if rect.x + (rect.width as i32) <= 0
            || rect.y + (rect.height as i32) <= 0
            || rect.x >= (self.width as i32)
            || rect.y >= (self.height as i32)
        {
            return;
        }
        let start_x = rect.x.max(0) as u32;
        let start_y = rect.y.max(0) as u32;
        let end_x = (rect.x + rect.width as i32).max(0) as u32;
        let end_x = end_x.min(self.width);
        let end_y = (rect.y + rect.height as i32).max(0) as u32;
        let end_y = end_y.min(self.height);

        for y in start_y..end_y {
            for x in start_x..end_x {
                let offset = ((y * self.stride + x) * 4) as usize;
                if offset + 3 < self.buffer.len() {
                    self.buffer[offset] = color.b;
                    self.buffer[offset + 1] = color.g;
                    self.buffer[offset + 2] = color.r;
                    self.buffer[offset + 3] = color.a;
                }
            }
        }
    }

    pub fn draw_circle(&mut self, cx: i32, cy: i32, radius: i32, color: Color) {
        let mut x = radius;
        let mut y = 0;
        let mut err = 0;

        while x >= y {
            self.draw_pixel(cx + x, cy + y, color);
            self.draw_pixel(cx + y, cy + x, color);
            self.draw_pixel(cx - y, cy + x, color);
            self.draw_pixel(cx - x, cy + y, color);
            self.draw_pixel(cx - x, cy - y, color);
            self.draw_pixel(cx - y, cy - x, color);
            self.draw_pixel(cx + y, cy - x, color);
            self.draw_pixel(cx + x, cy - y, color);

            if err <= 0 {
                y += 1;
                err += 2 * y + 1;
            }
            if err > 0 {
                x -= 1;
                err -= 2 * x + 1;
            }
        }
    }

    pub fn blit_buffer(
        &mut self,
        dest_x: i32,
        dest_y: i32,
        src_w: u32,
        src_h: u32,
        src_buf: &[u8],
    ) {
        for y in 0..src_h {
            let py = dest_y + (y as i32);
            if py < 0 || (py as u32) >= self.height {
                continue;
            }
            for x in 0..src_w {
                let px = dest_x + (x as i32);
                if px < 0 || (px as u32) >= self.width {
                    continue;
                }
                let src_off = (((y * src_w) + x) * 4) as usize;
                if src_off + 3 < src_buf.len() {
                    let color = Color {
                        r: src_buf[src_off + 2],
                        g: src_buf[src_off + 1],
                        b: src_buf[src_off],
                        a: src_buf[src_off + 3],
                    };
                    self.draw_pixel(px, py, color);
                }
            }
        }
    }

    /// Renders a single character glyph using the 8x8 bitmap font table into the framebuffer.
    pub fn draw_char(&mut self, x: i32, y: i32, ch: u8, color: Color) {
        let glyph = get_glyph(ch);
        for row in 0..8i32 {
            let row_mask = glyph[row as usize];
            if row_mask == 0 {
                continue;
            }
            let py = y + row;
            if py < 0 || (py as u32) >= self.height {
                continue;
            }
            for col in 0..8i32 {
                if (row_mask & (1 << (7 - col))) != 0 {
                    let px = x + col;
                    if px >= 0 && (px as u32) < self.width {
                        let offset = (((py as u32) * self.stride + (px as u32)) * 4) as usize;
                        if offset + 3 < self.buffer.len() {
                            self.buffer[offset] = color.b;
                            self.buffer[offset + 1] = color.g;
                            self.buffer[offset + 2] = color.r;
                            self.buffer[offset + 3] = color.a;
                        }
                    }
                }
            }
        }
    }

    /// Renders text with basic 8x8 font representation into the framebuffer.
    pub fn draw_text(&mut self, x: i32, y: i32, text: &str, color: Color) {
        let mut cur_x = x;
        for byte in text.bytes() {
            if byte != b' ' {
                self.draw_char(cur_x, y, byte, color);
            }
            cur_x += 9;
        }
    }

    /// Draws a mouse cursor
    pub fn draw_cursor(&mut self, x: i32, y: i32, color: Color) {
        let cursor_rect = Rect {
            x,
            y,
            width: 10,
            height: 10,
        };
        self.fill_rect(cursor_rect, color);
    }
}

/// Widget system primitives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidgetType {
    Button { label_id: u16 },
    Label { text_len: u8 },
    Panel,
    TextField,
}

#[derive(Clone, Copy, Debug)]
pub struct Widget {
    pub id: u16,
    pub bounds: Rect,
    pub widget_type: WidgetType,
    pub active: bool,
}

/// Terminal Emulator Backend (80x25 Grid).
#[derive(Debug)]
pub struct TerminalBackend {
    pub grid: [[u8; TERM_COLS]; TERM_ROWS],
    pub cursor_row: usize,
    pub cursor_col: usize,
}

impl TerminalBackend {
    pub const fn new() -> Self {
        Self {
            grid: [[b' '; TERM_COLS]; TERM_ROWS],
            cursor_row: 0,
            cursor_col: 0,
        }
    }

    pub fn write_char(&mut self, ch: u8) {
        match ch {
            b'\n' => {
                self.cursor_col = 0;
                self.cursor_row += 1;
            }
            b'\r' => {
                self.cursor_col = 0;
            }
            c => {
                if self.cursor_row < TERM_ROWS && self.cursor_col < TERM_COLS {
                    self.grid[self.cursor_row][self.cursor_col] = c;
                    self.cursor_col += 1;
                }
            }
        }
        if self.cursor_col >= TERM_COLS {
            self.cursor_col = 0;
            self.cursor_row += 1;
        }
        if self.cursor_row >= TERM_ROWS {
            self.scroll_up();
        }
    }

    fn scroll_up(&mut self) {
        for r in 1..TERM_ROWS {
            self.grid[r - 1] = self.grid[r];
        }
        self.grid[TERM_ROWS - 1] = [b' '; TERM_COLS];
        self.cursor_row = TERM_ROWS - 1;
    }
}

impl Default for TerminalBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// Notification Item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Notification {
    pub id: u32,
    pub title_hash: u64,
    pub timestamp: u64,
    pub active: bool,
}

/// Notification Daemon Instance.
#[derive(Debug)]
pub struct NotificationDaemon {
    notifications: [Option<Notification>; MAX_NOTIFICATIONS],
    counter: u32,
}

impl NotificationDaemon {
    pub const fn new() -> Self {
        Self {
            notifications: [None; MAX_NOTIFICATIONS],
            counter: 1,
        }
    }

    pub fn push_notification(&mut self, title_hash: u64, timestamp: u64) -> Result<u32, UiError> {
        let nid = self.counter;
        for slot in self.notifications.iter_mut() {
            if slot.is_none() {
                *slot = Some(Notification {
                    id: nid,
                    title_hash,
                    timestamp,
                    active: true,
                });
                self.counter += 1;
                return Ok(nid);
            }
        }
        Err(UiError::Full)
    }
}

impl Default for NotificationDaemon {
    fn default() -> Self {
        Self::new()
    }
}

/// AYUI Compositor and Window Manager
pub struct Compositor {
    windows: [Option<Window>; MAX_WINDOWS],
    events: [Option<InputEvent>; MAX_EVENTS],
    event_head: usize,
    event_len: usize,
    next_id: u16,
    monitors: [Option<Monitor>; MAX_MONITORS],
    monitor_count: usize,
    pub theme: Theme,
    pub accessibility: Accessibility,
    pub clipboard: Clipboard,
    pub pointer_x: i32,
    pub pointer_y: i32,
    pub pointer_buttons: u8,
    pub start_menu_open: bool,
    pub installed_apps_mask: u32,
}

impl Compositor {
    pub const fn new() -> Self {
        Self {
            windows: [None; MAX_WINDOWS],
            events: [None; MAX_EVENTS],
            event_head: 0,
            event_len: 0,
            next_id: 1,
            monitors: [None; MAX_MONITORS],
            monitor_count: 0,
            theme: Theme::default_dark(),
            accessibility: Accessibility::new(),
            clipboard: Clipboard::new(),
            pointer_x: 400,
            pointer_y: 300,
            pointer_buttons: 0,
            start_menu_open: false,
            installed_apps_mask: 0b1111_1111, // Standard AWE apps pre-installed
        }
    }

    pub fn apply_theme(&mut self, theme: Theme) {
        self.theme = theme;
        for win in self.windows.iter_mut().flatten() {
            win.bg_color = theme.bg_color;
        }
    }

    pub fn center_window(
        &mut self,
        id: WindowId,
        screen_width: u32,
        screen_height: u32,
    ) -> Result<(), UiError> {
        for win in self.windows.iter_mut().flatten() {
            if win.id == id {
                win.bounds.x = ((screen_width as i32) - (win.bounds.width as i32)) / 2;
                win.bounds.y = ((screen_height as i32) - (win.bounds.height as i32)) / 2;
                return Ok(());
            }
        }
        Err(UiError::InvalidWindow)
    }

    pub fn toggle_start_menu(&mut self) {
        self.start_menu_open = !self.start_menu_open;
    }

    pub fn install_app_package(&mut self, app_id: u8) {
        if app_id < 32 {
            self.installed_apps_mask |= 1 << app_id;
        }
    }

    pub fn remove_app_package(&mut self, app_id: u8) {
        if app_id < 32 {
            self.installed_apps_mask &= !(1 << app_id);
        }
    }

    pub fn is_app_installed(&self, app_id: u8) -> bool {
        if app_id < 32 {
            (self.installed_apps_mask & (1 << app_id)) != 0
        } else {
            false
        }
    }

    pub fn register_monitor(
        &mut self,
        bounds: Rect,
        primary: bool,
        gpu_accel: bool,
    ) -> Result<u8, UiError> {
        if self.monitor_count >= MAX_MONITORS {
            return Err(UiError::Full);
        }
        let id = self.monitor_count as u8;
        self.monitors[self.monitor_count] = Some(Monitor {
            id,
            bounds,
            primary,
            gpu_accelerated: gpu_accel,
        });
        self.monitor_count += 1;
        Ok(id)
    }

    pub fn create_window(&mut self, bounds: Rect) -> Result<WindowId, UiError> {
        self.create_app_window(bounds, AppType::Generic, b"Window")
    }

    pub fn create_app_window(
        &mut self,
        bounds: Rect,
        app_type: AppType,
        title: &[u8],
    ) -> Result<WindowId, UiError> {
        if !bounds.valid() {
            return Err(UiError::InvalidRect);
        }
        let slot = self
            .windows
            .iter()
            .position(Option::is_none)
            .ok_or(UiError::Full)?;
        let id = WindowId(self.next_id);
        self.next_id = self.next_id.checked_add(1).ok_or(UiError::Full)?;

        let mut title_bytes = [0u8; 32];
        let title_len = title.len().min(32);
        title_bytes[..title_len].copy_from_slice(&title[..title_len]);

        self.windows[slot] = Some(Window {
            id,
            bounds,
            visible: true,
            focused: false,
            minimized: false,
            maximized: false,
            bg_color: self.theme.bg_color,
            app_type,
            title_len,
            title_bytes,
        });
        self.focus(id)?;
        Ok(id)
    }

    pub fn destroy_window(&mut self, id: WindowId) -> Result<(), UiError> {
        for win in self.windows.iter_mut() {
            if let Some(w) = win
                && w.id == id
            {
                *win = None;
                return Ok(());
            }
        }
        Err(UiError::InvalidWindow)
    }

    pub fn focus(&mut self, id: WindowId) -> Result<(), UiError> {
        let mut found = false;
        for window in self.windows.iter_mut().flatten() {
            if window.id == id {
                window.focused = true;
                found = true;
            } else {
                window.focused = false;
            }
        }
        if found {
            Ok(())
        } else {
            Err(UiError::InvalidWindow)
        }
    }

    pub fn push_input(&mut self, event: InputEvent) -> Result<(), UiError> {
        match event {
            InputEvent::Pointer { x, y, buttons } => {
                if x < -1_048_576 || y < -1_048_576 || x > 1_048_576 || y > 1_048_576 {
                    return Err(UiError::InvalidEvent);
                }
                let dx = x - self.pointer_x;
                let dy = y - self.pointer_y;
                let was_down = (self.pointer_buttons & 1) != 0;
                let is_down = (buttons & 1) != 0;

                self.pointer_x = x;
                self.pointer_y = y;
                self.pointer_buttons = buttons;

                // Handle window drag if button held down while moving
                if is_down && (dx != 0 || dy != 0) {
                    for win in self.windows.iter_mut().flatten() {
                        if win.focused && win.visible && !win.minimized {
                            win.bounds.x += dx;
                            win.bounds.y += dy;
                            break;
                        }
                    }
                }

                // Handle Start Menu button click in taskbar (x: 5..120, y >= 560)
                if !was_down && is_down && y >= 560 && x <= 120 {
                    self.toggle_start_menu();
                }

                // Handle window focus & titlebar buttons (Close/Minimize/Maximize) on click
                if !was_down && is_down {
                    let mut win_to_destroy = None;
                    let mut win_to_focus = None;

                    for win in self.windows.iter_mut().flatten() {
                        if win.visible && win.bounds.contains(x, y) {
                            let wid = win.id;
                            win_to_focus = Some(wid);
                            let close_x = win.bounds.x + (win.bounds.width as i32) - 20;
                            let min_x = win.bounds.x + (win.bounds.width as i32) - 40;
                            if y >= win.bounds.y && y <= win.bounds.y + 24 {
                                if x >= close_x {
                                    win_to_destroy = Some(wid);
                                    break;
                                } else if x >= min_x {
                                    win.minimized = !win.minimized;
                                    break;
                                }
                            }
                        }
                    }
                    if let Some(wid) = win_to_destroy {
                        let _ = self.destroy_window(wid);
                    } else if let Some(wid) = win_to_focus {
                        let _ = self.focus(wid);
                    }
                }
            }
            InputEvent::Key { .. } => {}
        }

        if self.event_len == MAX_EVENTS {
            return Err(UiError::Full);
        }
        let index = (self.event_head + self.event_len) % MAX_EVENTS;
        self.events[index] = Some(event);
        self.event_len += 1;
        Ok(())
    }

    pub fn pop_input(&mut self) -> Option<InputEvent> {
        if self.event_len == 0 {
            return None;
        }
        let event = self.events[self.event_head].take();
        self.event_head = (self.event_head + 1) % MAX_EVENTS;
        self.event_len -= 1;
        event
    }

    pub fn render_to_framebuffer(&self, fb: &mut Framebuffer) {
        // Render desktop wallpaper background
        fb.fill_rect(
            Rect {
                x: 0,
                y: 0,
                width: fb.width,
                height: fb.height,
            },
            self.theme.bg_color,
        );

        // Render Desktop Taskbar / Panel
        let taskbar_rect = Rect {
            x: 0,
            y: (fb.height as i32) - 40,
            width: fb.width,
            height: 40,
        };
        fb.fill_rect(
            taskbar_rect,
            Color {
                r: 18,
                g: 20,
                b: 26,
                a: 255,
            },
        );
        fb.draw_text(
            10,
            (fb.height as i32) - 30,
            "AWE_OS AYUI Desktop",
            self.theme.text_color,
        );

        // Render visible windows (non-minimized)
        for win in self.windows.iter().flatten() {
            if win.visible && !win.minimized {
                // Window Title Bar
                let titlebar_color = if win.focused {
                    self.theme.titlebar_active
                } else {
                    self.theme.titlebar_inactive
                };
                let title_rect = Rect {
                    x: win.bounds.x,
                    y: win.bounds.y,
                    width: win.bounds.width,
                    height: 24,
                };
                fb.fill_rect(title_rect, titlebar_color);

                // Window Content Body
                let content_rect = Rect {
                    x: win.bounds.x,
                    y: win.bounds.y + 24,
                    width: win.bounds.width,
                    height: win.bounds.height.saturating_sub(24),
                };
                fb.fill_rect(content_rect, win.bg_color);

                // Render App Title & Control Buttons [-] [X]
                let title_str =
                    core::str::from_utf8(&win.title_bytes[..win.title_len]).unwrap_or("App");
                fb.draw_text(win.bounds.x + 8, win.bounds.y + 6, title_str, Color::WHITE);

                let close_x = win.bounds.x + (win.bounds.width as i32) - 18;
                let min_x = win.bounds.x + (win.bounds.width as i32) - 36;
                fb.draw_text(min_x, win.bounds.y + 6, "-", Color::WHITE);
                fb.draw_text(close_x, win.bounds.y + 6, "X", Color::RED);
            }
        }

        // Render Start Menu Popup if active
        if self.start_menu_open {
            let menu_rect = Rect {
                x: 10,
                y: (fb.height as i32) - 260,
                width: 220,
                height: 215,
            };
            fb.fill_rect(
                menu_rect,
                Color {
                    r: 30,
                    g: 34,
                    b: 44,
                    a: 255,
                },
            );
            fb.draw_text(
                20,
                (fb.height as i32) - 250,
                "AWE_OS Applications",
                Color::WHITE,
            );
            let apps = [
                "1. Terminal",
                "2. FileManager",
                "3. PackageCenter",
                "4. Settings",
                "5. SysMon",
                "6. TextEditor",
            ];
            for (idx, app) in apps.iter().enumerate() {
                fb.draw_text(
                    25,
                    (fb.height as i32) - 220 + (idx as i32) * 26,
                    app,
                    Color {
                        r: 180,
                        g: 190,
                        b: 210,
                        a: 255,
                    },
                );
            }
        }

        // Render Pointer Cursor
        fb.draw_cursor(self.pointer_x, self.pointer_y, Color::YELLOW);
    }

    pub const fn window_count(&self) -> usize {
        let mut count = 0;
        let mut i = 0;
        while i < MAX_WINDOWS {
            if self.windows[i].is_some() {
                count += 1;
            }
            i += 1;
        }
        count
    }
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compositor_creates_and_focuses_windows() {
        let mut c = Compositor::new();
        let a = c
            .create_window(Rect {
                x: 0,
                y: 0,
                width: 800,
                height: 600,
            })
            .unwrap();
        let b = c
            .create_window(Rect {
                x: 10,
                y: 10,
                width: 400,
                height: 300,
            })
            .unwrap();
        c.focus(b).unwrap();
        assert_eq!(c.window_count(), 2);
        assert!(
            c.windows
                .iter()
                .flatten()
                .find(|w| w.id == b)
                .unwrap()
                .focused
        );
        assert!(
            !c.windows
                .iter()
                .flatten()
                .find(|w| w.id == a)
                .unwrap()
                .focused
        );
    }

    #[test]
    fn terminal_and_notifications_work() {
        let mut term = TerminalBackend::new();
        term.write_char(b'A');
        term.write_char(b'W');
        term.write_char(b'E');
        assert_eq!(term.grid[0][0], b'A');
        assert_eq!(term.grid[0][1], b'W');
        assert_eq!(term.grid[0][2], b'E');

        let mut notif = NotificationDaemon::new();
        let nid = notif.push_notification(0xABCD, 100).unwrap();
        assert_eq!(nid, 1);
    }

    #[test]
    fn multi_monitor_and_clipboard_work() {
        let mut c = Compositor::new();
        let mid = c
            .register_monitor(
                Rect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
                true,
                true,
            )
            .unwrap();
        assert_eq!(mid, 0);

        c.clipboard.copy(b"AWEOS Clipboard").unwrap();
        assert_eq!(c.clipboard.text(), b"AWEOS Clipboard");
    }

    #[test]
    fn font_rendering_and_glyph_drawing_work() {
        let glyph_a = get_glyph(b'A');
        assert_eq!(*glyph_a, [0x18, 0x3C, 0x66, 0x66, 0x7E, 0x66, 0x66, 0x00]);

        let glyph_lower_a = get_glyph(b'a');
        assert_eq!(
            *glyph_lower_a,
            [0x00, 0x00, 0x3C, 0x06, 0x3E, 0x66, 0x3E, 0x00]
        );

        let glyph_num_5 = get_glyph(b'5');
        assert_eq!(
            *glyph_num_5,
            [0x7E, 0x60, 0x7C, 0x06, 0x06, 0x66, 0x3C, 0x00]
        );

        let glyph_symbol = get_glyph(b'>');
        assert_eq!(
            *glyph_symbol,
            [0x30, 0x18, 0x0C, 0x06, 0x0C, 0x18, 0x30, 0x00]
        );

        let mut buf = [0u8; 100 * 100 * 4];
        let mut fb = Framebuffer {
            width: 100,
            height: 100,
            stride: 100,
            buffer: &mut buf,
            gpu_accel: false,
        };

        fb.draw_text(10, 10, "AWEOS 2026!", Color::WHITE);

        // Check that pixels were rendered for 'A' at offset (10, 10)
        // 'A' row 0 has bits 0x18 at cols 3 and 4 -> x = 13, 14, y = 10
        let offset_13_10 = ((10 * 100 + 13) * 4) as usize;
        assert_eq!(buf[offset_13_10], 255); // Color::WHITE b = 255
    }

    #[test]
    fn test_theme_switching_and_window_layout() {
        let mut c = Compositor::new();
        assert!(c.theme.dark_mode);

        let win_id = c
            .create_window(Rect {
                x: 0,
                y: 0,
                width: 400,
                height: 300,
            })
            .unwrap();

        c.center_window(win_id, 800, 600).unwrap();
        let win = c.windows.iter().flatten().find(|w| w.id == win_id).unwrap();
        assert_eq!(win.bounds.x, 200);
        assert_eq!(win.bounds.y, 150);

        let mut rect = Rect {
            x: 13,
            y: 27,
            width: 100,
            height: 100,
        };
        rect.snap_to_grid(10);
        assert_eq!(rect.x, 10);
        assert_eq!(rect.y, 30);

        c.apply_theme(Theme::default_light());
        assert!(!c.theme.dark_mode);
        let win_after = c.windows.iter().flatten().find(|w| w.id == win_id).unwrap();
        assert_eq!(win_after.bg_color, Theme::default_light().bg_color);
    }

    #[test]
    fn test_start_menu_controls_and_app_installer() {
        let mut c = Compositor::new();
        assert!(!c.start_menu_open);
        c.toggle_start_menu();
        assert!(c.start_menu_open);

        c.install_app_package(10);
        assert!(c.is_app_installed(10));
        c.remove_app_package(10);
        assert!(!c.is_app_installed(10));

        let win_id = c
            .create_window(Rect {
                x: 10,
                y: 10,
                width: 200,
                height: 150,
            })
            .unwrap();
        let _ = c.destroy_window(win_id);
        assert_eq!(c.window_count(), 0);
    }
}
