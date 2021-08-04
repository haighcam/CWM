use x11rb::protocol::xproto::*;
use super::utils::keymap_xmodmap;
use serde::{Serialize, Deserialize, de::DeserializeOwned};


pub struct Theme {
    pub(crate) border_width: u16,
    pub(crate) window_width: u16,
    pub(crate) window_height: u16,
    pub(crate) window_min_width: u16,
    pub(crate) window_min_height: u16,
    pub(crate) border_color_focused: u32,
    pub(crate) border_color_unfocused: u32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            border_width: 5,
            window_width: 600,
            window_height: 400,
            window_min_width: 60,
            window_min_height: 40,
            border_color_focused: 0x006900,
            border_color_unfocused: 0xFFFFFF,
        }
    }
}

pub(crate) type Key = (ModMask, Keycode, WMCommand);
pub(crate) struct Keys(Vec<Key>);

impl Default for Keys {
    fn default() -> Self {
        let keymap = keymap_xmodmap();
        Self(vec![
            (ModMask::M1, *keymap.get(&"d".to_string()).unwrap(), WMCommand::Spawn("rofia".to_string(), vec![])),
            (ModMask::M1, *keymap.get(&"Return".to_string()).unwrap(), WMCommand::Spawn("st".to_string(), vec![])),
            (ModMask::M1, *keymap.get(&"f".to_string()).unwrap(), WMCommand::Fullscreen(FlagArg { val: true, toggle: true })),
            (ModMask::M1, *keymap.get(&"s".to_string()).unwrap(), WMCommand::Sticky(FlagArg { val: true, toggle: true })),
            (ModMask::M1, *keymap.get(&"a".to_string()).unwrap(), WMCommand::AlwaysOnTop(FlagArg { val: true, toggle: true })),
            (ModMask::M1, *keymap.get(&"space".to_string()).unwrap(), WMCommand::Floating(FlagArg { val: true, toggle: true })),
            (ModMask::M1, *keymap.get(&"q".to_string()).unwrap(), WMCommand::KillClient),
            (ModMask::M1 | ModMask::SHIFT, *keymap.get(&"q".to_string()).unwrap(), WMCommand::CloseWM)
        ])
    }
}

impl Keys {
    pub(crate) const IGNORED_MODS: [u16; 2] = [0, 1 << 1]; // normal combo, ignore caplock
    pub(crate) const IGNORED_MASK: u16 = !Self::IGNORED_MODS[1];
    pub(crate) fn iter(&self) -> std::slice::Iter<Key> {
        self.0.iter()
    }
}

pub type FlagArg = SetArg<bool>;

#[derive(Serialize, Deserialize, Debug)]
pub struct SetArg<T: PartialEq + Clone> {
    val: T,
    toggle: bool
}

impl<T: PartialEq + Clone> SetArg<T> {
    pub fn apply_arg(&self, arg: &mut T, last: T) -> bool {
        if *arg != self.val {
            *arg = self.val.clone();
            true
        } else if self.toggle && last != self.val {
            *arg = last;
            true
        } else {
            false
        }
    }
}

impl SetArg<bool> {
    pub fn apply(&self, arg: &mut bool) -> bool {
        if *arg != self.val {
            *arg = self.val;
            true
        } else if self.toggle {
            *arg^=true;
            true
        } else {
            false
        }
    }
}

pub(crate) enum WMCommand {
    Spawn(String, Vec<String>),
    KillClient,
    CloseWM,
    Fullscreen(FlagArg),
    AlwaysOnTop(FlagArg),
    Floating(FlagArg),
    Sticky(FlagArg)
}
