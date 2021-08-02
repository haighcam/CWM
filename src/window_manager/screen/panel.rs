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

use crate::utils::Rect;
use super::{Screen, WindowManager, WindowLocation};

pub struct Panel {
    win: Window,
    wm_strut: WMStrut,
}

#[derive(PartialEq, Default)]
struct WMStrut {
    left: u32,
    right: u32,
    top: u32,
    bottom: u32,
}

impl Panel {
    fn new(wm: &WindowManager<impl Connection>, win: Window) -> Self {
        let wm_strut = WMStrut::new(wm, win);
        Self { win, wm_strut }
    }

    fn update_reserved_space(&mut self, wm: &WindowManager<impl Connection>) -> bool {
        let wm_strut = WMStrut::new(wm, self.win);
        if wm_strut != self.wm_strut {
            self.wm_strut = wm_strut;
            true
        } else {
            false
        }
    }
}

impl Screen {
    pub fn panel_changed(&mut self, wm: &WindowManager<impl Connection>) {
        if let Some(mut tag) = self.tag.as_ref().map(|x| x.borrow_mut()) {
            tag.set_available(wm, self.free_rect());
        }
        // triger a hook
    }

    pub fn panel_register(&mut self, wm: &WindowManager<impl Connection>, win: Window) -> WindowLocation {
        self.panels.insert(win, Panel::new(wm, win));
        map_window(&wm.dpy, win);
        self.panel_changed(wm);
        WindowLocation::Panel(self.id)
    }

    pub fn panel_unregister(&mut self, wm: &WindowManager<impl Connection>, win: Window) {
        if let Some(panel) = self.panels.remove(&win) {
            self.panel_changed(wm)
        }
    }

    pub fn panel_property_changed(&mut self, wm: &WindowManager<impl Connection>, win: Window, atom: Atom) {
        if atom == wm.atoms._NET_WM_STRUT || atom == wm.atoms._NET_WM_STRUT_PARTIAL {
            if let Some(panel) = self.panels.get_mut(&win) {
                panel.update_reserved_space(wm);
            }
        }
    }

    fn panel_reserved_space(&self) -> WMStrut {
        self.panels.values().fold(WMStrut::default(), |x, y| x.max(&y.wm_strut))
    }

    pub fn free_rect(&self) -> Rect {
        let strut = self.panel_reserved_space();
        Rect::new(self.size.x + strut.left as i16, self.size.y + strut.top as i16, self.size.width - (strut.left + strut.right) as u16, self.size.height - (strut.top + strut.bottom) as u16)
    }
}

impl WMStrut {
    fn new(wm: &WindowManager<impl Connection>, win: Window) -> Self {
        let (left, right, top, bottom) = {
            if let Some(wm_struct_partial) = get_property(&wm.dpy, false, win, wm.atoms._NET_WM_STRUT_PARTIAL, AtomEnum::CARDINAL, 0, 12).ok().and_then(|x| x.reply().ok()) {
                let vals: Vec<u32> = wm_struct_partial.value32().unwrap().collect();
                (vals[0], vals[1], vals[2], vals[3])
            } else if let Some(wm_struct) = get_property(&wm.dpy, false, win, wm.atoms._NET_WM_STRUT, AtomEnum::CARDINAL, 0, 4).ok().and_then(|x| x.reply().ok()) {
                let vals: Vec<u32> = wm_struct.value32().unwrap().collect();
                (vals[0], vals[1], vals[2], vals[3])
            } else {
                (0, 0, 0, 0)
            }
        };
        Self { left, right, top, bottom }
    }
    fn max(mut self, other: &Self) -> Self {
        self.left = self.left.max(other.left);
        self.right = self.right.max(other.right);
        self.top = self.top.max(other.top);
        self.bottom = self.bottom.max(other.bottom);
        self
    }
}