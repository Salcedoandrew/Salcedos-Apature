//! One action per physical keypress.
//!
//! macOS hands only ⌘ (and ⌃) key equivalents to the menu bar, so the bare
//! keys the menus show — T, X, C, ⌥C, V, Z, B, L, S, the number keys — never
//! reach a menu item from the keyboard. The app's own key handler dispatches
//! them on every platform instead, and ⌘ keys stay the menu's job. Should a
//! press ever arrive through both paths, [`KeyOnce`] lets the first through
//! and drops the copy from the other path.

use std::time::{Duration, Instant};

use crate::commands::Command;

/// Where a command came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Menu,
    Keys,
}

/// Does the app's key handler act on this press? Never while a text field
/// has focus (captions, rename, dialog fields: the key is text), and not for
/// ⌘ keys when a native menu exists (the menu delivers those).
pub fn handler_takes(has_menu: bool, text_focus: bool, command_key: bool) -> bool {
    !text_focus && !(command_key && has_menu)
}

/// Drops the second delivery of one keypress.
#[derive(Default)]
pub struct KeyOnce {
    recent: Vec<(Command, Source, Instant)>,
}

impl KeyOnce {
    /// One press reaches both paths within the same frame or the next;
    /// two deliberate presses are further apart than this.
    pub const WINDOW: Duration = Duration::from_millis(150);

    /// True if `cmd` from `src` should run: false when the same command just
    /// ran from the other path (an echo of the same keypress).
    pub fn admit(&mut self, cmd: &Command, src: Source, now: Instant) -> bool {
        self.recent
            .retain(|(_, _, t)| now.saturating_duration_since(*t) < Self::WINDOW);
        if let Some(i) = self
            .recent
            .iter()
            .position(|(c, s, _)| c == cmd && *s != src)
        {
            self.recent.remove(i);
            return false;
        }
        self.recent.push((cmd.clone(), src, now));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_key_fires_once_whichever_path_delivers_it() {
        let t0 = Instant::now();
        let mut k = KeyOnce::default();
        // Handler first, menu echo second: one dispatch.
        assert!(k.admit(&Command::ToggleTag, Source::Keys, t0));
        assert!(!k.admit(&Command::ToggleTag, Source::Menu, t0));
        // Menu first, handler echo second: one dispatch.
        let t1 = t0 + Duration::from_secs(1);
        assert!(k.admit(&Command::SetRating(-1), Source::Menu, t1));
        assert!(!k.admit(
            &Command::SetRating(-1),
            Source::Keys,
            t1 + Duration::from_millis(16)
        ));
        // A press through one path only: just that one.
        let t2 = t1 + Duration::from_secs(1);
        assert!(k.admit(&Command::ToggleZoom, Source::Keys, t2));
    }

    #[test]
    fn deliberate_repeats_and_other_commands_still_fire() {
        let t0 = Instant::now();
        let mut k = KeyOnce::default();
        // Two presses of T: two toggles, even quickly (same path).
        assert!(k.admit(&Command::ToggleTag, Source::Keys, t0));
        assert!(k.admit(
            &Command::ToggleTag,
            Source::Keys,
            t0 + Duration::from_millis(40)
        ));
        // A different command from the menu in the same frame is not an echo.
        assert!(k.admit(&Command::ToggleCompare, Source::Menu, t0));
        // A menu pick well after a keypress is its own action.
        let later = t0 + Duration::from_millis(40) + KeyOnce::WINDOW;
        assert!(k.admit(&Command::ToggleTag, Source::Menu, later));
    }

    #[test]
    fn handler_routing_rule() {
        // Bare key with a native menu (macOS): the handler takes it.
        assert!(handler_takes(true, false, false));
        // A text field has focus: never, menu or not.
        assert!(!handler_takes(true, true, false));
        assert!(!handler_takes(false, true, false));
        // ⌘ keys go only through the menu when there is one.
        assert!(!handler_takes(true, false, true));
        // Without a native menu the handler does everything.
        assert!(handler_takes(false, false, true));
    }
}
