//! Mouse events to the byte sequences terminal apps ask for (xterm mouse
//! tracking): the SGR encoding (`ESC [ < b ; x ; y M/m`) when the app
//! enabled it, the legacy X10 one (`ESC [ M b x y`) otherwise.

use crate::keys::Mods;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseAction {
    Press(MouseButton),
    Release(MouseButton),
    /// Moved, with this button held (or none).
    Move(Option<MouseButton>),
    WheelUp,
    WheelDown,
}

/// What the app asked to be told about (DECSET 1000 / 1002 / 1003).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MouseTracking {
    /// Not tracking: the mouse selects text and scrolls back.
    #[default]
    Off,
    /// Presses, releases and the wheel.
    Clicks,
    /// Also moves while a button is held.
    Drags,
    /// Every move.
    Motion,
}

impl MouseTracking {
    /// Whether `action` is reported in this mode.
    pub fn reports(self, action: MouseAction) -> bool {
        match (self, action) {
            (MouseTracking::Off, _) => false,
            (_, MouseAction::Move(None)) => self == MouseTracking::Motion,
            (MouseTracking::Clicks, MouseAction::Move(_)) => false,
            _ => true,
        }
    }
}

/// Encodes a mouse event at a zero-based grid cell.
pub fn encode_mouse(action: MouseAction, row: usize, column: usize, mods: Mods, sgr: bool) -> Option<Vec<u8>> {
    let button_code = |button: MouseButton| match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    };
    let mut code: u32 = match action {
        MouseAction::Press(button) => button_code(button),
        // Legacy encoding can't say which button was released.
        MouseAction::Release(button) => if sgr { button_code(button) } else { 3 },
        MouseAction::Move(Some(button)) => 32 + button_code(button),
        MouseAction::Move(None) => 32 + 3,
        MouseAction::WheelUp => 64,
        MouseAction::WheelDown => 65,
    };
    code += 4 * u32::from(mods.shift) + 8 * u32::from(mods.alt) + 16 * u32::from(mods.ctrl);
    let (x, y) = (column + 1, row + 1);
    if sgr {
        let end = if matches!(action, MouseAction::Release(_)) { 'm' } else { 'M' };
        return Some(format!("\x1b[<{code};{x};{y}{end}").into_bytes());
    }
    // Each value is sent as one byte offset by 32: positions past 223 can't be.
    if x > 223 || y > 223 {
        return None;
    }
    Some(vec![0x1b, b'[', b'M', 32 + code as u8, 32 + x as u8, 32 + y as u8])
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods { ctrl: false, alt: false, shift: false };

    fn sgr(action: MouseAction, row: usize, column: usize, mods: Mods) -> String {
        String::from_utf8(encode_mouse(action, row, column, mods, true).unwrap()).unwrap()
    }

    #[test]
    fn sgr_encoding() {
        assert_eq!(sgr(MouseAction::Press(MouseButton::Left), 0, 0, NONE), "\x1b[<0;1;1M");
        assert_eq!(sgr(MouseAction::Release(MouseButton::Right), 4, 9, NONE), "\x1b[<2;10;5m");
        assert_eq!(sgr(MouseAction::Move(Some(MouseButton::Left)), 1, 2, NONE), "\x1b[<32;3;2M");
        assert_eq!(sgr(MouseAction::Move(None), 1, 2, NONE), "\x1b[<35;3;2M");
        assert_eq!(sgr(MouseAction::WheelDown, 0, 0, NONE), "\x1b[<65;1;1M");
        let ctrl = Mods { ctrl: true, ..NONE };
        assert_eq!(sgr(MouseAction::Press(MouseButton::Middle), 0, 0, ctrl), "\x1b[<17;1;1M");
        // SGR has no position limit.
        assert_eq!(sgr(MouseAction::WheelUp, 299, 499, NONE), "\x1b[<64;500;300M");
    }

    #[test]
    fn legacy_encoding() {
        let press = encode_mouse(MouseAction::Press(MouseButton::Left), 2, 4, NONE, false).unwrap();
        assert_eq!(press, [0x1b, b'[', b'M', 32, 32 + 5, 32 + 3]);
        let release = encode_mouse(MouseAction::Release(MouseButton::Left), 2, 4, NONE, false).unwrap();
        assert_eq!(release[3], 32 + 3);
        assert_eq!(encode_mouse(MouseAction::WheelUp, 0, 300, NONE, false), None);
    }

    #[test]
    fn tracking_modes_filter_moves() {
        let drag = MouseAction::Move(Some(MouseButton::Left));
        let hover = MouseAction::Move(None);
        let click = MouseAction::Press(MouseButton::Left);
        assert!(!MouseTracking::Off.reports(click));
        assert!(MouseTracking::Clicks.reports(click) && !MouseTracking::Clicks.reports(drag));
        assert!(MouseTracking::Drags.reports(drag) && !MouseTracking::Drags.reports(hover));
        assert!(MouseTracking::Motion.reports(hover));
    }
}
