use x11rb::{
    connection::Connection,
    protocol::{xproto::*, randr::*},
    properties::*,
    x11_utils::Serialize,
    COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME, NONE
};
use std::collections::HashMap;
use crate::utils::{Rect, stack_::Stack};
use crate::{Connections, AtomCollection, CwmRes};
use crate::config::Theme;
use super::{WindowManager, WindowLocation, tag::ClientArgs};


mod desktop_window;
mod panel;
use desktop_window::DesktopWindow;
use panel::Panel;

pub struct Monitor {
    pub id: usize,
    pub focused_tag: usize,
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
    fn process_type(&mut self, window_type: Atom, atoms: &AtomCollection) {
        match self {
            Self::Client(args) => {
                if window_type == atoms._NET_WM_WINDOW_TYPE_TOOLBAR || window_type == atoms._NET_WM_WINDOW_TYPE_UTILITY {
                    args.focus = false;
                } else if window_type == atoms._NET_WM_WINDOW_TYPE_DIALOG {
                    args.flags.floating = true;
                    args.centered = true;
                } else if window_type == atoms._NET_WM_WINDOW_TYPE_DOCK {
                    *self = Self::Panel;
                } else if window_type == atoms._NET_WM_WINDOW_TYPE_DESKTOP {
                    *self = Self::Desktop
                } else if window_type == atoms._NET_WM_WINDOW_TYPE_NOTIFICATION {
                    args.managed = false;
                }
            },
            Self::Desktop => {
                if window_type == atoms._NET_WM_WINDOW_TYPE_DOCK {
                    *self = Self::Panel;
                }
            },
            Self::Panel => ()
        }
    }
}

impl WindowManager {
    pub fn manage_window(&mut self, mon: usize, win: Window) -> CwmRes<()> {
        let type_cookie = get_property(&self.conn.dpy, false, win, self.atoms._NET_WM_WINDOW_TYPE, AtomEnum::ATOM, 0, 2048).unwrap();
        let mut args = ProcessWindow::Client(ClientArgs::new(&self.theme));
        if let Ok(states) = type_cookie.reply() {
            if let Some(states) = states.value32() {
                for state in states {
                    args.process_type(state, &self.atoms);
                }
            }
        }
        //info!("window detected: {:?}", args);
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
    pub fn add_monitor(&mut self, tag: usize, monitor: MonitorInfo) -> CwmRes<()> {
        let monitor = Monitor {
            id: 0,
           // monitor_id: monitor.name,
            size: Rect::new(monitor.x, monitor.y, monitor.width, monitor.height),
            focused_tag: self.tags.len(),
            panels: HashMap::new(),
            desktop_windows: HashMap::new(),
        };
        let idx = {
            if let Some(idx) = self.free_monitors.pop() {
                self.monitors[idx] = monitor;
                idx
            } else {
                self.monitors.push(monitor);
                self.monitors.len() - 1
            }
        };
        
        self.monitors[idx].id = idx;
        self.set_monitor_tag(idx, tag)?;
        Ok(())
    }

    pub fn set_monitor_tag(&mut self, mon: usize, tag: usize) -> CwmRes<()> {
        let old_tag = self.monitors[mon].focused_tag;
        if old_tag == tag {
            return Ok(())
        }
        if let Some(old_tag) = self.tags.get_mut(old_tag) {
            old_tag.hide(&self.conn, &self.atoms);
        }
        if let Some(old_mon) = self.tags[tag].monitor {
            self.tags[tag].hide(&self.conn, &self.atoms);
            if let Some(old_tag) = self.tags.get_mut(old_tag) {
                old_tag.set_monitor(&self.conn, &mut self.monitors[old_mon], &self.atoms, &mut self.hooks);
            }
        }
        self.tags[tag].set_monitor(&self.conn, &mut self.monitors[mon], &self.atoms, &mut self.hooks);
        Ok(())
    }
}