use x11rb::protocol::{xproto::*, randr::*};
use std::collections::HashMap;
use crate::utils::{Rect, pop_set};
use crate::connections::Aux;
use super::{WindowManager, WindowLocation, tag::ClientArgs};
use anyhow::Result;
use log::info;


mod desktop_window;
mod panel;
use desktop_window::DesktopWindow;
use panel::Panel;

#[derive(Debug)]
pub struct Monitor {
    pub id: Atom,
    pub name: String,
    pub focused_tag: Atom,
    pub prev_tag: Atom,
    panels: HashMap<Window, Panel>,
    desktop_windows: HashMap<Window, DesktopWindow>,
    pub size: Rect
}

#[derive(Debug)]
pub(crate) enum ProcessWindow {
    Client(ClientArgs),
    Panel,
    Desktop
}

impl ProcessWindow {
    fn process_type(&mut self, aux: &Aux, window_type: Atom) {
        match self {
            Self::Client(args) => {
                if window_type == aux.atoms._NET_WM_WINDOW_TYPE_TOOLBAR || window_type == aux.atoms._NET_WM_WINDOW_TYPE_UTILITY {
                    args.focus = false;
                } else if window_type == aux.atoms._NET_WM_WINDOW_TYPE_DIALOG {
                    args.flags.floating = true;
                    args.centered = true;
                } else if window_type == aux.atoms._NET_WM_WINDOW_TYPE_DOCK {
                    *self = Self::Panel;
                } else if window_type == aux.atoms._NET_WM_WINDOW_TYPE_DESKTOP {
                    *self = Self::Desktop
                } else if window_type == aux.atoms._NET_WM_WINDOW_TYPE_NOTIFICATION {
                    args.managed = false;
                }
            },
            Self::Desktop => {
                if window_type == aux.atoms._NET_WM_WINDOW_TYPE_DOCK {
                    *self = Self::Panel;
                }
            },
            Self::Panel => ()
        }
    }
}

impl WindowManager {
    pub fn manage_window(&mut self, mon: Atom, win: Window) -> Result<()> {
        let type_cookie = get_property(&self.aux.dpy, false, win, self.aux.atoms._NET_WM_WINDOW_TYPE, AtomEnum::ATOM, 0, 2048).unwrap();
        let mut args = ProcessWindow::Client(ClientArgs::new(&self.aux));
        if let Ok(states) = type_cookie.reply() {
            if let Some(states) = states.value32() {
                for state in states {
                    args.process_type(&self.aux, state);
                }
            }
        }
        info!("Managing Window {:?}", args);
        match args {
            ProcessWindow::Client(mut args) => {
                self.process_args(win, &mut args)?;
                self.manage_client(win, args)?;
            },
            ProcessWindow::Desktop => self.desktop_window_register(mon, win)?,
            ProcessWindow::Panel => self.panel_register(mon, win)?
        }
        Ok(())
    }

    // maybe make sure that focused tag isn't currently viewed (that would break things)
    pub fn add_monitor(&mut self, tag: Option<Atom>, monitor: MonitorInfo) -> Result<Atom> {
        let id = monitor.name;
        let name = String::from_utf8(get_atom_name(&self.aux.dpy, id)?.reply()?.name).unwrap();
        let monitor = Monitor {
            id,
            name,
            size: Rect::new(monitor.x, monitor.y, monitor.width, monitor.height),
            focused_tag: 0,
            prev_tag: 0,
            panels: HashMap::new(),
            desktop_windows: HashMap::new(),
        };
        info!(" monitor: {:?}", monitor);
        let tag =  tag.or_else(|| pop_set(&mut self.free_tags)).unwrap_or_else(|| self.temp_tag());
        self.monitors.insert(id, monitor);
        self.focused_monitor = id;
        self.set_monitor_tag(id, tag)?;
        let monitor = self.monitors.get_mut(&id).unwrap();
        monitor.prev_tag = tag;
        self.aux.hooks.mon_open(id, &monitor.name);
        Ok(id)
    }

    pub fn set_monitor_tag(&mut self, mon: Atom, tag: Atom) -> Result<()> {
        let old_tag = self.monitors.get(&mon).unwrap().focused_tag;
        if old_tag == tag {
            return Ok(())
        }
        if let Some(tag) = self.tags.get_mut(&old_tag) {
            self.free_tags.insert(old_tag);
            tag.hide(&self.aux)?;
        }
        if let Some(old_mon) = self.tags.get(&tag).unwrap().monitor {
            self.tags.get_mut(&tag).unwrap().hide(&self.aux)?;
            if let Some(old_tag) = self.tags.get_mut(&old_tag) {
                old_tag.set_monitor(&mut self.aux, self.monitors.get_mut(&old_mon).unwrap())?;
            }
        } else {
            self.free_tags.remove(&tag);
        }
        self.tags.get_mut(&tag).unwrap().set_monitor(&mut self.aux, self.monitors.get_mut(&mon).unwrap())?;
        self.aux.hooks.tag_update(&self.tags, &self.tag_order, self.focused_monitor);
        Ok(())
    }

    pub fn remove_monitor(&mut self) -> Result<()> {
        Ok(())
    }

    pub fn update_monitor(&mut self, info: MonitorInfo) -> Result<()> {
        let mon = self.monitors.get_mut(&info.name).unwrap();
        mon.size = Rect::new(info.x, info.y, info.width, info.height);
        self.tags.get_mut(&mon.focused_tag).unwrap().set_tiling_size(&self.aux, mon.free_rect())
    }

    pub fn update_monitors(&mut self) -> Result<()> {
        let monitors = get_monitors(&self.aux.dpy, self.root, true)?.reply()?;
        let mut new_mons = Vec::new();
        for mon in monitors.monitors.into_iter() {
            if self.monitors.contains_key(&mon.name) {
                self.update_monitor(mon)?;
            } else {
                new_mons.push(mon);
            }
        }
        for mon in new_mons {
            let size = Rect::new(mon.x, mon.y, mon.width, mon.height);
            let mut keep = true;
            for other in self.monitors.values() {
                if other.size == size {
                    keep = false;
                    break;
                }
            }
            if keep {
                self.add_monitor(None, mon)?;
            }
        }
        Ok(())
    }
}