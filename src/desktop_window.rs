use x11rb::{
    connection::Connection,
    protocol::xproto::*,
};

use crate::{
    monitor::Monitor,
    WindowManager, WindowLocation, CWMRes
};

pub(crate) struct DesktopWindow { }

impl Monitor {
    pub(crate) fn desktop_window_register(&mut self, wm: &WindowManager, win: Window) -> CWMRes<WindowLocation> {
        self.desktop_windows.insert(win, DesktopWindow {});
        map_window(&wm.conn.dpy, win)?;
        Ok(WindowLocation::DesktopWindow(self.id))
    }

    pub(crate) fn desktop_window_unregister(&mut self, win: Window) {
        self.desktop_windows.remove(&win);
    }
}