use x11rb::{
    connection::Connection,
    properties::{WmHints, WmClass, WmSizeHints},
    protocol::xproto::*,
    x11_utils::Serialize,
    CURRENT_TIME, NONE
};
use std::{
    collections::{HashMap, HashSet},
    cell::RefCell,
    rc::Rc
};
use log::info;
use crate::{
    config::FlagArg,
    utils::{Stack, Rect}
};
use super::{WindowManager, Screen, WindowLocation};

mod layout;
use layout::Layout;
mod layers;
use layers::Layers;
mod client;
pub use client::{Client, ClientArgs};

#[derive(Default)]
pub struct Tag {
    id: usize,
    name: String,
    screen: Option<usize>,
    last_screen: Option<usize>,
    focus_stack: Stack<Window>,
    screen_size: Rect,
    available: Rect,
    root: Option<Window>,
    layout: Rc<RefCell<Layout>>,
    layers: Layers,
    clients: HashMap<Window, Rc<RefCell<Client>>>,
    client_list: Vec<Window>,
    urgent_clients: HashSet<Window>,
    temp: bool,
}

impl Tag {
    pub fn new(id: usize, name: impl Into<String>) -> Self {
        Tag {
            id,
            .. Self::default()
        }
    }

    pub fn set_available(&mut self, wm: &WindowManager<impl Connection>, available: Rect) {
        if available != self.available {
            self.available = available;
            Layout::resize_tiled(self.layout.clone(), wm, Some(&self.available));
        }
    }

    pub fn id(&self) -> usize {self.id}
    pub fn screen(&self) -> Option<usize> {self.screen}

    pub fn focused_client(&self) -> Option<Window> {
        self.focus_stack.front().map(|x| *x)
    }

    pub fn set_screen(&mut self, wm: &WindowManager<impl Connection>, screen: &mut Screen) {
        if let Some(old_screen) = self.screen {
            if screen.id == old_screen {
                return
            }
        } else {
            for client in self.clients.values().map(|x| x.borrow()) {
                if !client.flags.hidden {
                    client.show(wm)
                }
            }
        }
        // resize the windows
        let available = screen.free_rect();
        Layout::resize_all(self.layout.clone(), wm, &available, &self.screen_size, &screen.size);
        // reparent all windows to the new root,
        if screen.child_tags.insert(self.id) {
            if let Some(old_screen) = self.screen.or(self.last_screen).and_then(|id| wm.screen(id)) {
                old_screen.borrow_mut().child_tags.remove(&self.id);
            }
            for win in self.clients.keys() {
                reparent_window(&wm.dpy, *win, screen.root, 0, 0);
            }
        }
        self.last_screen.replace(screen.id);
        self.screen.replace(screen.id);
        self.screen_size.copy(&screen.size);
        self.root.replace(screen.root);
        self.available = available;
        self.ewmh_set_active_window(wm, self.focus_stack.front().and_then(|id| self.clients.get(id).map(|x| x.borrow().win())).unwrap_or(NONE));
        info!("tag {} screen set to {}. size: {:?}, available: {:?}, root layout size: {:?}", self.id, screen.id, self.screen_size, self.available, self.layout.borrow().rect);
        //show all of the windows

        // reset the layout (if changed then apply those changes)
    }

    pub fn hide(&mut self, wm: &WindowManager<impl Connection>) {
        self.screen.take();
        for client in self.clients.values().map(|x| x.borrow()) {
            if !client.flags.hidden {
                client.hide(wm)
            }
        }
    }

    pub fn focus_client(&mut self, wm: &WindowManager<impl Connection>, win: Window) {
        if let Some(client) = self.clients.get(&win).map(|x| x.borrow()) {
            self.focus_stack.unlink_node(client.stack_pos());
            if let Some(win) = self.focus_stack.front() {
                change_window_attributes(&wm.dpy, *win, &ChangeWindowAttributesAux::new().border_pixel(wm.theme.border_color_unfocused));
            }
            self.focus_stack.link_node_front(client.stack_pos());
            set_input_focus(&wm.dpy, InputFocus::PARENT, client.win(), CURRENT_TIME);
            self.ewmh_set_active_window(wm, client.win());         
            if wm.theme.border_width > 0 {
                change_window_attributes(&wm.dpy, win, &ChangeWindowAttributesAux::new().border_pixel(wm.theme.border_color_focused));
            }
        }
    }

    pub fn client(&self, win: Window) -> Option<&Rc<RefCell<Client>>> {
        self.clients.get(&win)
    }

    pub fn set_fullscreen(&mut self, wm: &WindowManager<impl Connection>, win: Window, arg: &FlagArg) {
        if let Some(client) = self.clients.get(&win).map(|x| x.clone()) {
            if let Some(win) = {
                let mut client = client.borrow_mut();
                if arg.apply(&mut client.flags.fullscreen) {
                    Some(client.frame())
                } else {
                    None
                }
            } {
                self.switch_layer(wm, win);
            }
        }
    }

    pub fn set_floating(&mut self, wm: &WindowManager<impl Connection>, win: Window, arg: &FlagArg) {
        if let Some(client) = self.clients.get(&win).map(|x| x.clone()) {
            if let Some(win) = {
                let mut client = client.borrow_mut();
                if arg.apply(&mut client.flags.floating) {
                    Some(client.frame())
                } else {
                    None
                }
            } {
                self.switch_layer(wm, win);
            }
        }
    }

    pub fn set_aot(&mut self, wm: &WindowManager<impl Connection>, win: Window, arg: &FlagArg) {
        if let Some(client) = self.clients.get(&win).map(|x| x.clone()) {
            if let Some(win) = {
                let mut client = client.borrow_mut();
                if arg.apply(&mut client.flags.aot) {
                    Some(client.frame())
                } else {
                    None
                }
            } {
                self.switch_layer(wm, win);
            }
        }
    }

    pub fn set_sticky(&mut self, win: Window, arg: &FlagArg) {
        if let Some(client) = self.clients.get(&win).map(|x| x.clone()) {
            let mut client = client.borrow_mut();
            arg.apply(&mut client.flags.sticky);
        }
    }

    fn ewmh_set_active_window(&self, wm: &WindowManager<impl Connection>, win: Window) {
        if let Some(root) = self.root {
            change_property(&wm.dpy, PropMode::REPLACE, root, wm.atoms._NET_ACTIVE_WINDOW, AtomEnum::WINDOW, 32, 1, &win.serialize());
        }
    }
}

impl<X: Connection> WindowManager<X> {
    pub fn temp_tag(&mut self) -> Rc<RefCell<Tag>> {
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