use x11rb::{
    connection::Connection,
    protocol::xproto::*,
    CURRENT_TIME, NONE
};
use std::{
    collections::{HashMap, HashSet},
    cell::RefCell,
    rc::Rc
};
use log::info;
use crate::{
    client::{Client},
    config::FlagArg,
    layers::Layers,
    layout::Layout,
    monitor::Monitor,
    utils::{Stack, Rect},
    WindowManager, CWMRes
};

#[derive(Default)]
pub(crate) struct Tag {
    pub(crate) id: usize,
    pub(crate) name: String,
    pub(crate) monitor: Option<usize>,
    pub(crate) focus_stack: Stack<Window>,
    pub(crate) monitor_size: Rect,
    pub(crate) available: Rect,
    pub(crate) layout: Rc<RefCell<Layout>>,
    pub(crate) layers: Layers,
    pub(crate) clients: HashMap<Window, Rc<RefCell<Client>>>,
    pub(crate) urgent_clients: HashSet<Window>,
    pub(crate) temp: bool,
}

impl Tag {
    pub(crate) fn new(id: usize, name: impl Into<String>) -> Self {
        Tag {
            id,
            name: name.into(),
            .. Self::default()
        }
    }

    pub(crate) fn set_available(&mut self, wm: &WindowManager, available: Rect) -> CWMRes<()> {
        if available != self.available {
            self.available = available;
            Layout::resize_tiled(self.layout.clone(), wm, Some(&self.available))?;
        }
        Ok(())
    }

    pub(crate) fn focused_client(&self) -> Option<Window> {
        self.focus_stack.front().copied()
    }

    pub(crate) fn set_monitor(&mut self, wm: &WindowManager, monitor: &mut Monitor) -> CWMRes<()> {
        if let Some(old_monitor) = self.monitor {
            if monitor.id == old_monitor {
                return Ok(())
            }
        } else {
            for client in self.clients.values().map(|x| x.borrow()) {
                if !client.flags.hidden {
                    client.show(wm)?;
                }
            }
        }
        // resize the windows
        let available = monitor.free_rect();
        Layout::resize_all(self.layout.clone(), wm, &available, &self.monitor_size, &monitor.size)?;
        
        self.monitor.replace(monitor.id);
        self.monitor_size.copy(&monitor.size);
        self.available = available;
        self.set_active_window(wm, self.focus_stack.front().and_then(|id| self.clients.get(id).map(|x| x.borrow().name.clone())).unwrap_or(None));
        info!("tag {} monitor set to {}. size: {:?}, available: {:?}, root layout size: {:?}", self.id, monitor.id, self.monitor_size, self.available, self.layout.borrow().rect);
        Ok(())
    }

    pub(crate) fn hide(&mut self, wm: &WindowManager) -> CWMRes<()> {
        self.monitor.take();
        for client in self.clients.values().map(|x| x.borrow()) {
            if !client.flags.hidden {
                client.hide(wm)?;
            }
        }
        Ok(())
    }

    pub(crate) fn focus_client(&mut self, wm: &WindowManager, win: Window) -> CWMRes<()> {
        if let Some(client) = self.clients.get(&win).map(|x| x.borrow()) {
            self.focus_stack.unlink_node(client.stack_pos);
            if let Some(win) = self.focus_stack.front() {
                change_window_attributes(&wm.conn.dpy, *win, &ChangeWindowAttributesAux::new().border_pixel(wm.theme.border_color_unfocused))?;
            }
            self.focus_stack.link_node_front(client.stack_pos);
            set_input_focus(&wm.conn.dpy, InputFocus::PARENT, client.win, CURRENT_TIME)?;
            self.set_active_window(wm, client.name.clone());         
            if wm.theme.border_width > 0 {
                change_window_attributes(&wm.conn.dpy, win, &ChangeWindowAttributesAux::new().border_pixel(wm.theme.border_color_focused))?;
            }
        }
        Ok(())
    }

    pub(crate) fn client(&self, win: Window) -> Option<&Rc<RefCell<Client>>> {
        self.clients.get(&win)
    }

    pub(crate) fn set_fullscreen(&mut self, wm: &WindowManager, win: Window, arg: &FlagArg) -> CWMRes<()> {
        if let Some(client) = self.clients.get(&win).cloned() {
            if let Some(win) = {
                let mut client = client.borrow_mut();
                if arg.apply(&mut client.flags.fullscreen) {
                    Some(client.frame)
                } else {
                    None
                }
            } {
                self.switch_layer(wm, win)?;
            }
        }
        Ok(())
    }

    pub(crate) fn set_floating(&mut self, wm: &WindowManager, win: Window, arg: &FlagArg) -> CWMRes<()> {
        if let Some(client) = self.clients.get(&win).cloned() {
            if let Some(win) = {
                let mut client = client.borrow_mut();
                if arg.apply(&mut client.flags.floating) {
                    Some(client.frame)
                } else {
                    None
                }
            } {
                self.switch_layer(wm, win)?;
            }
        }
        Ok(())
    }

    pub(crate) fn set_aot(&mut self, wm: &WindowManager, win: Window, arg: &FlagArg) -> CWMRes<()> {
        if let Some(client) = self.clients.get(&win).cloned() {
            if let Some(win) = {
                let mut client = client.borrow_mut();
                if arg.apply(&mut client.flags.aot) {
                    Some(client.frame)
                } else {
                    None
                }
            } {
                self.switch_layer(wm, win)?;
            }
        }
        Ok(())
    }

    pub(crate) fn set_sticky(&mut self, win: Window, arg: &FlagArg) {
        if let Some(client) = self.clients.get(&win).cloned() {
            let mut client = client.borrow_mut();
            arg.apply(&mut client.flags.sticky);
        }
    }

    fn set_active_window(&self, wm: &WindowManager, name: Option<String>) {
        if let Some(monitor) = self.monitor {
            wm.hooks.borrow_mut().monitor_focus(monitor, name)
        }
    }
}

impl WindowManager {
    pub(crate) fn temp_tag(&mut self) -> Rc<RefCell<Tag>> {
        let id = self.free_tags.pop().unwrap_or_else(|| {self.tags.push(None); self.tags.len() - 1});
        let tag = Rc::new(RefCell::new(Tag {
            id,
            temp: true,
            .. Default::default()
        }));
        self.tags[id].replace(tag.clone());
        tag
    }
}