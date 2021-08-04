use x11rb::protocol::xproto::*;
use super::{WindowManager, WindowLocation, CwmRes};

pub struct DesktopWindow { }

impl WindowManager {
    pub fn desktop_window_register(&mut self, mon: usize, win: Window) -> CwmRes<()> {
        self.monitors[mon].desktop_windows.insert(win, DesktopWindow {});
        map_window(&self.conn.dpy, win)?;
        self.windows.insert(win, WindowLocation::DesktopWindow(mon));
        Ok(())
    }

    pub fn desktop_window_unregister(&mut self, mon: usize, win: Window) {
        self.monitors[mon].desktop_windows.remove(&win);
    }
}