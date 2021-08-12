use anyhow::{Context, Result};
use log::info;
use x11rb::protocol::xproto::*;

use super::Monitor;
use crate::utils::Rect;
use crate::{Aux, WindowLocation, WindowManager};

#[derive(Debug)]
pub struct Panel {
    win: Window,
    wm_strut: WMStrut,
}

#[derive(PartialEq, Default, Debug)]
struct WMStrut {
    left: u32,
    right: u32,
    top: u32,
    bottom: u32,
}

impl Panel {
    fn new(aux: &Aux, win: Window) -> Result<Self> {
        let wm_strut = WMStrut::new(aux, win)?;
        Ok(Self { win, wm_strut })
    }

    fn update_reserved_space(&mut self, aux: &Aux) -> Result<bool> {
        let wm_strut = WMStrut::new(aux, self.win)?;
        if wm_strut != self.wm_strut {
            self.wm_strut = wm_strut;
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

impl WindowManager {
    pub fn panel_changed(&mut self, mon: Atom) -> Result<()> {
        let mon = self.monitors.get(&mon).unwrap();
        self.tags
            .get_mut(&mon.focused_tag)
            .unwrap()
            .set_tiling_size(&self.aux, mon.free_rect())
        // triger a hook
    }

    pub fn panel_register(&mut self, mut mon: Atom, win: Window) -> Result<()> {
        let rect: Rect = get_geometry(&self.aux.dpy, win)?.reply()?.into();
        for new_mon in self.monitors.values() {
            if new_mon.size.contains_rect(&rect) {
                mon = new_mon.id;
                break;
            }
        }
        info!("panel registered {} mon: {}", win, mon);
        self.monitors
            .get_mut(&mon)
            .unwrap()
            .panels
            .insert(win, Panel::new(&self.aux, win)?);
        change_window_attributes(
            &self.aux.dpy,
            win,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .context(crate::code_loc!())?;
        map_window(&self.aux.dpy, win).context(crate::code_loc!())?;
        self.panel_changed(mon)?;
        self.windows.insert(win, WindowLocation::Panel(mon));
        Ok(())
    }

    pub fn panel_unregister(&mut self, mon: Atom, win: Window) -> Result<()> {
        if self
            .monitors
            .get_mut(&mon)
            .unwrap()
            .panels
            .remove(&win)
            .is_some()
        {
            change_window_attributes(
                &self.aux.dpy,
                win,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::NO_EVENT),
            )
            .context(crate::code_loc!())?;
            self.panel_changed(mon)?;
        }
        Ok(())
    }

    pub fn panel_property_changed(&mut self, win: Window, mon: Atom, atom: Atom) -> Result<()> {
        info!("property changed");
        if atom == self.aux.atoms._NET_WM_STRUT || atom == self.aux.atoms._NET_WM_STRUT_PARTIAL {
            if let Some(panel) = self.monitors.get_mut(&mon).unwrap().panels.get_mut(&win) {
                if panel.update_reserved_space(&self.aux)? {
                    self.panel_changed(mon)?
                }
            }
        }
        Ok(())
    }
}
impl Monitor {
    fn panel_reserved_space(&self) -> WMStrut {
        self.panels
            .values()
            .fold(WMStrut::default(), |x, y| x.max(&y.wm_strut))
    }

    pub fn free_rect(&self) -> Rect {
        let strut = self.panel_reserved_space();
        Rect::new(
            self.size.x + strut.left as i16,
            self.size.y + strut.top as i16,
            self.size.width - (strut.left + strut.right) as u16,
            self.size.height - (strut.top + strut.bottom) as u16,
        )
    }
}

impl WMStrut {
    fn new(aux: &Aux, win: Window) -> Result<Self> {
        let (left, right, top, bottom) = {
            let wm_struct_partial = get_property(
                &aux.dpy,
                false,
                win,
                aux.atoms._NET_WM_STRUT_PARTIAL,
                AtomEnum::CARDINAL,
                0,
                12,
            )
            .context(crate::code_loc!())?
            .reply()
            .context(crate::code_loc!())?;
            if wm_struct_partial.length != 0 {
                let vals: Vec<u32> = wm_struct_partial.value32().unwrap().collect();
                (vals[0], vals[1], vals[2], vals[3])
            } else {
                let wm_struct = get_property(
                    &aux.dpy,
                    false,
                    win,
                    aux.atoms._NET_WM_STRUT,
                    AtomEnum::CARDINAL,
                    0,
                    4,
                )
                .context(crate::code_loc!())?
                .reply()
                .context(crate::code_loc!())?;
                if wm_struct.length != 0 {
                    let vals: Vec<u32> = wm_struct.value32().unwrap().collect();
                    (vals[0], vals[1], vals[2], vals[3])
                } else {
                    (0, 0, 0, 0)
                }
            }
        };
        Ok(Self {
            left,
            right,
            top,
            bottom,
        })
    }
    fn max(mut self, other: &Self) -> Self {
        self.left = self.left.max(other.left);
        self.right = self.right.max(other.right);
        self.top = self.top.max(other.top);
        self.bottom = self.bottom.max(other.bottom);
        self
    }
}
