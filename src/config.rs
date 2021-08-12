pub const IGNORED_MODS: [u16; 2] = [0, (1 << 1)]; //normal mask, ignore caplock
pub const IGNORED_MASK: u16 = !IGNORED_MODS[1];

pub struct Theme {
    pub border_width: u16,
    pub gap: u16,
    pub top_margin: i16,
    pub bottom_margin: i16,
    pub left_margin: i16,
    pub right_margin: i16,
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
            border_width: 1,
            gap: 4,
            top_margin: 4,
            left_margin: 4,
            right_margin: 4,
            bottom_margin: 4,
            window_width: 600,
            window_height: 400,
            window_min_width: 60,
            window_min_height: 40,
            border_color_focused: 0xAA006900,
            border_color_unfocused: 0xAAFFFFFF,
        }
    }
}
