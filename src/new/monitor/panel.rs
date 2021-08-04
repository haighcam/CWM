use x11rb::protocol::xproto::*;
use crate::utils::Rect;
use crate::{Connections, AtomCollection, CwmRes};

use super::{Monitor, WindowLocation, super::WindowManager};

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
    fn new(conn: &Connections, win: Window, atoms: &AtomCollection) -> CwmRes<Self> {
        let wm_strut = WMStrut::new(conn, win, atoms)?;
        Ok(Self { win, wm_strut })
    }

    fn update_reserved_space(&mut self, conn: &Connections, atoms: &AtomCollection) -> CwmRes<bool> {
        let wm_strut = WMStrut::new(conn, self.win, atoms)?;
        if wm_strut != self.wm_strut {
            self.wm_strut = wm_strut;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl WindowManager {
    pub fn panel_changed(&mut self, mon: usize) -> CwmRes<()> {
        let mon = &self.monitors[mon];
        self.tags[mon.focused_tag].set_tiling_size(&self.conn, mon.free_rect())
        // triger a hook
    }

    pub fn panel_register(&mut self, mon: usize, win: Window) -> CwmRes<()> {
        self.monitors[mon].panels.insert(win, Panel::new(&self.conn, win, &self.atoms)?);
        map_window(&self.conn.dpy, win)?;
        self.panel_changed(mon)?;
        self.windows.insert(win, WindowLocation::Panel(mon));
        Ok(())
    }

    pub fn panel_unregister(&mut self, mon: usize, win: Window) -> CwmRes<()> {
        if self.monitors[mon].panels.remove(&win).is_some() {
            self.panel_changed(mon)?;
        }
        Ok(())
    }

    pub fn panel_property_changed(&mut self, win: Window, mon: usize, atom: Atom) -> CwmRes<()> {
        if atom == self.atoms._NET_WM_STRUT || atom == self.atoms._NET_WM_STRUT_PARTIAL {
            if let Some(panel) = self.monitors[mon].panels.get_mut(&win) {
                if panel.update_reserved_space(&self.conn, &self.atoms)? {
                    self.panel_changed(mon)?
                }
            }
        }
        Ok(())
    }
}
impl Monitor {
    fn panel_reserved_space(&self) -> WMStrut {
        self.panels.values().fold(WMStrut::default(), |x, y| x.max(&y.wm_strut))
    }

    pub fn free_rect(&self) -> Rect {
        let strut = self.panel_reserved_space();
        Rect::new(self.size.x + strut.left as i16, self.size.y + strut.top as i16, self.size.width - (strut.left + strut.right) as u16, self.size.height - (strut.top + strut.bottom) as u16)
    }
}

impl WMStrut {
    fn new(conn: &Connections, win: Window, atoms: &AtomCollection) -> CwmRes<Self> {
        let (left, right, top, bottom) = {
            let wm_struct_partial = get_property(&conn.dpy, false, win, atoms._NET_WM_STRUT_PARTIAL, AtomEnum::CARDINAL, 0, 12)?.reply()?;
            if wm_struct_partial.length != 0 {
                let vals: Vec<u32> = wm_struct_partial.value32().unwrap().collect();
                (vals[0], vals[1], vals[2], vals[3])
            } else {
                let wm_struct = get_property(&conn.dpy, false, win, atoms._NET_WM_STRUT, AtomEnum::CARDINAL, 0, 4)?.reply()?;
                if wm_struct.length != 0 {
                    let vals: Vec<u32> = wm_struct.value32().unwrap().collect();
                    (vals[0], vals[1], vals[2], vals[3])
                } else {
                    (0, 0, 0, 0)
                }
            }
        };
        Ok(Self { left, right, top, bottom })
    }
    fn max(mut self, other: &Self) -> Self {
        self.left = self.left.max(other.left);
        self.right = self.right.max(other.right);
        self.top = self.top.max(other.top);
        self.bottom = self.bottom.max(other.bottom);
        self
    }
}