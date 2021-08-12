use anyhow::{Context, Result};
use x11rb::protocol::xproto::*;

use super::{WindowLocation, WindowManager};

#[derive(Debug)]
pub struct DesktopWindow {}

impl WindowManager {
    pub fn desktop_window_register(&mut self, mon: Atom, win: Window) -> Result<()> {
        self.monitors
            .get_mut(&mon)
            .unwrap()
            .desktop_windows
            .insert(win, DesktopWindow {});
        map_window(&self.aux.dpy, win).context(crate::code_loc!())?;
        self.windows.insert(win, WindowLocation::DesktopWindow(mon));
        Ok(())
    }

    pub fn desktop_window_unregister(&mut self, mon: Atom, win: Window) {
        self.monitors
            .get_mut(&mon)
            .unwrap()
            .desktop_windows
            .remove(&win);
    }
}
