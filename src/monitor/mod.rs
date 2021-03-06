use super::{tag::ClientArgs, WindowLocation, WindowManager};
use crate::connections::{Aux, SetArg};
use crate::utils::{pop_set_ord, Rect};
use anyhow::Result;
use log::info;
use std::collections::{HashMap, HashSet};
use x11rb::connection::Connection;
use x11rb::protocol::{randr::*, xproto::*};
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT};

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
    pub sticky: HashSet<usize>,
    pub size: Rect,
    pub bg: Window,
}

#[derive(Debug)]
pub(crate) enum ProcessWindow {
    Client(ClientArgs),
    Panel,
    Desktop,
}

impl ProcessWindow {
    fn process_type(&mut self, aux: &Aux, window_type: Atom) {
        match self {
            Self::Client(args) => {
                if window_type == aux.atoms._NET_WM_WINDOW_TYPE_TOOLBAR
                    || window_type == aux.atoms._NET_WM_WINDOW_TYPE_UTILITY
                {
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
            }
            Self::Desktop => {
                if window_type == aux.atoms._NET_WM_WINDOW_TYPE_DOCK {
                    *self = Self::Panel;
                }
            }
            Self::Panel => (),
        }
    }
}

impl WindowManager {
    pub fn manage_window(&mut self, mon: Atom, win: Window) -> Result<()> {
        let type_cookie = get_property(
            &self.aux.dpy,
            false,
            win,
            self.aux.atoms._NET_WM_WINDOW_TYPE,
            AtomEnum::ATOM,
            0,
            2048,
        )
        .unwrap();
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
            }
            ProcessWindow::Desktop => self.desktop_window_register(mon, win)?,
            ProcessWindow::Panel => self.panel_register(mon, win)?,
        }
        Ok(())
    }

    // maybe make sure that focused tag isn't currently viewed (that would break things)
    pub fn add_monitor(&mut self, tag: Option<Atom>, monitor: MonitorInfo) -> Result<Atom> {
        let id = monitor.name;
        if self.monitors.is_empty() {
            self.focused_monitor = id;
            self.prev_monitor = id;
        }
        let name = String::from_utf8(get_atom_name(&self.aux.dpy, id)?.reply()?.name).unwrap();
        let bg = self.aux.dpy.generate_id()?;
        let monitor = Monitor {
            id,
            name,
            size: Rect::new(monitor.x, monitor.y, monitor.width, monitor.height),
            focused_tag: 0,
            prev_tag: 0,
            panels: HashMap::new(),
            desktop_windows: HashMap::new(),
            sticky: HashSet::new(),
            bg,
        };
        info!(" monitor: {:?}", monitor);
        let tag = tag
            .or_else(|| pop_set_ord(&mut self.free_tags, &self.tag_order))
            .map_or_else(|| self.temp_tag(), Ok)?;
        self.monitors.insert(id, monitor);
        // self.focused_monitor = id;
        self.set_monitor_tag(id, tag)?;
        let monitor = self.monitors.get_mut(&id).unwrap();
        monitor.prev_tag = tag;
        let aux = CreateWindowAux::new().event_mask(EventMask::ENTER_WINDOW);
        create_window(
            &self.aux.dpy,
            COPY_DEPTH_FROM_PARENT,
            bg,
            self.aux.root,
            monitor.size.x,
            monitor.size.y,
            monitor.size.width,
            monitor.size.height,
            0,
            WindowClass::COPY_FROM_PARENT,
            COPY_FROM_PARENT,
            &aux,
        )?;
        configure_window(
            &self.aux.dpy,
            bg,
            &ConfigureWindowAux::new().stack_mode(StackMode::BELOW),
        )?;
        if !self.supporting {
            self.supporting = true;
            self.aux.dpy.change_property32(
                PropMode::REPLACE,
                self.aux.root,
                self.aux.atoms._NET_SUPPORTING_WM_CHECK,
                AtomEnum::WINDOW,
                &[bg],
            )?;
            self.aux.dpy.change_property32(
                PropMode::REPLACE,
                bg,
                self.aux.atoms._NET_SUPPORTING_WM_CHECK,
                AtomEnum::WINDOW,
                &[bg],
            )?;
        }
        map_window(&self.aux.dpy, bg)?;
        self.windows.insert(bg, WindowLocation::Monitor(id));
        self.aux.hooks.mon_open(id, &monitor.name, bg);
        Ok(id)
    }

    pub fn switch_monitor_tag(&mut self, mon: Atom, tag: SetArg<Atom>) -> Result<()> {
        if let Some((mut focused_tag, prev_tag)) =
            self.monitors.get(&mon).map(|x| (x.focused_tag, x.prev_tag))
        {
            if tag.apply_arg(&mut focused_tag, prev_tag) {
                self.set_monitor_tag(mon, focused_tag)?;
                self.aux.selection.hide(
                    &self.aux.dpy,
                    self.monitors.get(&mon).map(|x| x.prev_tag),
                    None,
                )?;
            }
        }
        if self.focused_monitor == mon {
            let tag = self.tags.get_mut(&self.focused_tag()).unwrap();
            tag.set_focus(&mut self.aux)?;
        }
        Ok(())
    }

    pub fn set_monitor_tag(&mut self, mon: Atom, tag: Atom) -> Result<()> {
        let old_tag = self.monitors.get(&mon).unwrap().focused_tag;
        if old_tag == tag {
            return Ok(());
        }
        let old_valid = if let Some(tag) = self.tags.get_mut(&old_tag) {
            tag.hide(&mut self.aux)?;
            true
        } else {
            false
        };
        if let Some(old_mon) = self.tags.get(&tag).unwrap().monitor {
            self.tags.get_mut(&tag).unwrap().hide(&mut self.aux)?;
            if old_valid {
                self.tags
                    .get_mut(&old_tag)
                    .unwrap()
                    .set_monitor(&mut self.aux, self.monitors.get_mut(&old_mon).unwrap())?;
                let mut sticky = self
                    .monitors
                    .get_mut(&old_mon)
                    .unwrap()
                    .sticky
                    .drain()
                    .collect::<Vec<_>>();
                for client in sticky.iter_mut() {
                    *client = self.move_client(tag, *client, SetArg(old_tag, false))?
                }
                self.monitors
                    .get_mut(&old_mon)
                    .unwrap()
                    .sticky
                    .extend(sticky);
            }
        } else {
            self.free_tags.remove(&tag);
            self.free_tags.insert(old_tag);
        }
        self.tags
            .get_mut(&tag)
            .unwrap()
            .set_monitor(&mut self.aux, self.monitors.get_mut(&mon).unwrap())?;
        if old_valid {
            let mut sticky = self
                .monitors
                .get_mut(&mon)
                .unwrap()
                .sticky
                .drain()
                .collect::<Vec<_>>();
            for client in sticky.iter_mut() {
                *client = self.move_client(old_tag, *client, SetArg(tag, false))?
            }
            self.monitors.get_mut(&mon).unwrap().sticky.extend(sticky);
        }
        self.aux
            .hooks
            .tag_update(&self.tags, &self.tag_order, self.focused_monitor);
        Ok(())
    }

    pub fn remove_monitor(&mut self, mon: Atom) -> Result<()> {
        if let Some(mon) = self.monitors.remove(&mon) {
            info!("removing mon {} {}", mon.name, mon.id);
            self.windows.remove(&mon.bg);
            destroy_window(&self.aux.dpy, mon.bg)?;
            let tag = self.tags.get_mut(&mon.focused_tag).unwrap();
            for client in mon.sticky {
                tag.client_mut(client).flags.sticky = false;
            }
            self.tags
                .get_mut(&mon.focused_tag)
                .unwrap()
                .hide(&mut self.aux)?;
            self.free_tags.insert(mon.focused_tag);
            self.aux.hooks.mon_close(mon.id, &mon.name);
        }
        if self.tags.len() > self.monitors.len() {
            if let Some(tag) = self
                .temp_tags
                .iter()
                .find(|x| self.free_tags.contains(x))
                .or_else(|| self.temp_tags.last())
                .cloned()
            {
                self.remove_tag(tag)?
            }
        }
        Ok(())
    }

    pub fn update_monitor(&mut self, info: MonitorInfo) -> Result<()> {
        let mon = self.monitors.get_mut(&info.name).unwrap();
        mon.size = Rect::new(info.x, info.y, info.width, info.height);
        configure_window(&self.aux.dpy, mon.bg, &mon.size.aux(0))?;
        self.tags.get_mut(&mon.focused_tag).unwrap().resize_all(
            &self.aux,
            &mon.free_rect(),
            &mon.size,
        )
    }

    pub fn update_monitors(&mut self) -> Result<()> {
        let monitors = get_monitors(&self.aux.dpy, self.aux.root, true)?.reply()?;
        let mut new_mons = Vec::new();
        let mut keep_monitors = HashSet::new();
        for mon in monitors.monitors.into_iter() {
            if self.monitors.contains_key(&mon.name) {
                keep_monitors.insert(mon.name);
                self.update_monitor(mon)?;
            } else {
                new_mons.push(mon);
            }
        }
        let remove: Vec<_> = self
            .monitors
            .keys()
            .filter(|x| !keep_monitors.contains(x))
            .cloned()
            .collect();
        for mon in remove {
            self.remove_monitor(mon)?;
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

    pub fn set_sticky(&mut self, tag: Atom, client: usize, arg: &SetArg<bool>) {
        let tag = self.tags.get_mut(&tag).unwrap();
        if let Some(mon) = tag.monitor {
            let sticky = &mut tag.client_mut(client).flags.sticky;
            if arg.apply(sticky) {
                let mon = self.monitors.get_mut(&mon).unwrap();
                if *sticky {
                    mon.sticky.insert(client);
                } else {
                    mon.sticky.remove(&client);
                }
            }
        }
    }

    pub fn set_focus(&mut self, mon: Atom) -> Result<()> {
        if mon != self.focused_monitor {
            info!("focusing mon {}", mon);
            if let Some(tag) = self
                .monitors
                .get(&self.focused_monitor)
                .map(|x| x.focused_tag)
            {
                self.tags.get_mut(&tag).unwrap().unset_focus(&self.aux)?;
                self.prev_monitor = self.focused_monitor;
            }
            self.focused_monitor = mon;
            let tag = self
                .monitors
                .get(&self.focused_monitor)
                .unwrap()
                .focused_tag;
            self.tags.get_mut(&tag).unwrap().set_focus(&mut self.aux)?;
        }
        self.aux
            .hooks
            .tag_update(&self.tags, &self.tag_order, self.focused_monitor);
        Ok(())
    }
}
