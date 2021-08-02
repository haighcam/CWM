use x11rb::protocol::xproto::*;
use super::utils::keymap_xmodmap;

pub struct Theme {
    pub border_width: u16,
    pub window_width: u16,
    pub window_height: u16,
    pub window_min_width: u16,
    pub window_min_height: u16,
    pub border_color_focused: u32,
    pub border_color_unfocused: u32,
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

pub type Key = (ModMask, Keycode, WMCommand);
pub struct Keys(Vec<Key>);

impl Default for Keys {
    fn default() -> Self {
        let keymap = keymap_xmodmap();
        Self(vec![
            (ModMask::M1, *keymap.get(&"d".to_string()).unwrap(), WMCommand::Spawn("rofia".to_string(), vec![])),
            (ModMask::M1, *keymap.get(&"Return".to_string()).unwrap(), WMCommand::Spawn("st".to_string(), vec![])),
            (ModMask::M1, *keymap.get(&"f".to_string()).unwrap(), WMCommand::Fullscreen(FlagArg::Toggle)),
            (ModMask::M1, *keymap.get(&"s".to_string()).unwrap(), WMCommand::Sticky(FlagArg::Toggle)),
            (ModMask::M1, *keymap.get(&"a".to_string()).unwrap(), WMCommand::AlwaysOnTop(FlagArg::Toggle)),
            (ModMask::M1, *keymap.get(&"space".to_string()).unwrap(), WMCommand::Floating(FlagArg::Toggle)),
            (ModMask::M1, *keymap.get(&"q".to_string()).unwrap(), WMCommand::KillClient),
            (ModMask::M1 | ModMask::SHIFT, *keymap.get(&"q".to_string()).unwrap(), WMCommand::CloseWM)
        ])
    }
}

impl Keys {
    pub const IGNORED_MODS: [u16; 2] = [0, 1 << 1]; // normal combo, ignore caplock
    pub const IGNORED_MASK: u16 = !Self::IGNORED_MODS[1];
    pub fn iter(&self) -> std::slice::Iter<Key> {
        self.0.iter()
    }
}

#[derive(Debug)]
pub enum FlagArg {
    Set,
    Clear,
    Toggle
}

impl FlagArg {
    pub fn apply(&self, flag: &mut bool) -> bool {
        match self {
            FlagArg::Set => if !*flag {*flag=true; true} else {false},
            FlagArg::Clear => if *flag {*flag=false; true} else {false},
            FlagArg::Toggle => {*flag^=true; true}
        }
    }
}

pub enum WMCommand {
    Spawn(String, Vec<String>),
    KillClient,
    CloseWM,
    Fullscreen(FlagArg),
    AlwaysOnTop(FlagArg),
    Floating(FlagArg),
    Sticky(FlagArg)
}
