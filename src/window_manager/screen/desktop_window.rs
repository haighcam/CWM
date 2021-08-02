use x11rb::{
    connection::Connection,
    protocol::xproto::*,
    COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME, atom_manager
};
use log::info;
use std::{
    collections::HashMap,
    cell::RefCell,
    rc::Rc
};

use super::{WindowManager, Screen, WindowLocation};

pub struct DesktopWindow {
    win: Window
}

impl Screen {
    pub fn desktop_window_register(&mut self, wm: &WindowManager<impl Connection>, win: Window) -> WindowLocation {
        self.desktop_windows.insert(win, DesktopWindow {win});
        map_window(&wm.dpy, win);
        WindowLocation::DesktopWindow(self.id)
    }

    pub fn desktop_window_unregister(&mut self, win: Window) {
        self.desktop_windows.remove(&win);
    }

    pub fn desktop_window_for_each(&self, f: impl Fn(&DesktopWindow) -> ()) {
        self.desktop_windows.values().for_each(|win| f(win));
    }
}

impl DesktopWindow {
    pub fn window(&self) -> Window {
        self.win
    }
}