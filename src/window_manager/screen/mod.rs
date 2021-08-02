use x11rb::{
    connection::Connection,
    protocol::xproto::*,
    x11_utils::Serialize,
    NONE
};
use std::{
    collections::{HashMap, HashSet},
    cell::RefCell,
    rc::Rc
};
use log::info;
use crate::{utils::Rect, config::Keys};
use super::{WindowManager, AtomCollection, WindowLocation};
mod tag;
mod desktop_window;
mod panel;
use panel::Panel;
use desktop_window::DesktopWindow;
pub use tag::Tag;
use tag::ClientArgs;

pub struct Screen {
    id: usize,
    root: Window,
    size: Rect,
    tag: Option<Rc<RefCell<Tag>>>,
    panels: HashMap<Window, Panel>,
    desktop_windows: HashMap<Window, DesktopWindow>,
    child_tags: HashSet<usize>,
}

impl Screen {
    pub fn tag(&self) -> Option<&Rc<RefCell<Tag>>> {
        self.tag.as_ref()
    }

    pub fn root(&self) -> Window {
        self.root
    }

    // if tag is not none then it should be valid, otherwise it is disregarded and a new one is created
    pub fn set_tag(&mut self, wm: &mut WindowManager<impl Connection>, tag: Option<usize>) {
        if tag.and_then(|id| self.tag.as_ref().map(|tag| (id, tag))).map(|(id, tag)| tag.borrow().id() != id).unwrap_or(true) {
            let new_tag = tag.and_then(|id| wm.tag(id)).unwrap_or(wm.temp_tag());
            if let Some(mut tag) = self.tag.as_ref().map(|x| x.borrow_mut()) {
                if let Some(screen) = new_tag.borrow().screen().and_then(|id| wm.screen(id)) {
                    let mut screen = screen.borrow_mut();
                    tag.set_screen(wm, &mut screen);
                    screen.tag = self.tag.clone();
                } else {
                    tag.hide(wm);
                }
            }
            if let Some(screen) = new_tag.borrow().screen().and_then(|id| wm.screen(id)) {
                if let Some(tag) = self.tag.as_ref() {
                    let mut screen = screen.borrow_mut();
                    tag.borrow_mut().set_screen(wm, &mut screen);
                    screen.tag = self.tag.clone();
                }
            }
            new_tag.borrow_mut().set_screen(wm, self);
            self.tag.replace(new_tag);
        }
    }

    pub fn manage_window(&mut self, wm: &WindowManager<impl Connection>, win: Window) -> (WindowLocation, Window) {
        let type_cookie = get_property(&wm.dpy, false, win, wm.atoms._NET_WM_WINDOW_TYPE, AtomEnum::ATOM, 0, 2048).unwrap();
        let mut args = ProcessWindow::Client(ClientArgs::new(wm));
        if let Ok(states) = type_cookie.reply() {
            if let Some(states) = states.value32() {
                for state in states {
                    args.process_type(state, &wm.atoms);
                }
            }
        }
        info!("window detected: {:?}", args);
        match args {
            ProcessWindow::Client(args) => self.tag.as_ref().unwrap().borrow_mut().manage(wm, win, args, self),
            ProcessWindow::Desktop => (self.desktop_window_register(wm, win), win),
            ProcessWindow::Panel => (self.panel_register(wm, win), win)
        }
    }
}

impl<X: Connection> WindowManager<X> {
    pub fn add_screen(&mut self, tag: Option<usize>, screen: &x11rb::protocol::xproto::Screen) {
        let id = self.free_screens.pop().unwrap_or_else(|| {self.screens.push(None); self.screens.len() - 1});
        let mut screen = Screen {
            id,
            root: screen.root,
            size: Rect::new(0, 0, screen.width_in_pixels, screen.height_in_pixels),
            tag: None,
            panels: HashMap::new(),
            desktop_windows: HashMap::new(),
            child_tags: HashSet::new()
        };
        change_property(&self.dpy, PropMode::REPLACE, screen.root, self.atoms._NET_SUPPORTED, AtomEnum::ATOM, 32, 1, &self.atoms._NET_ACTIVE_WINDOW.serialize());
        change_window_attributes(&self.dpy, screen.root, &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY | EventMask::STRUCTURE_NOTIFY));
        ungrab_key(&self.dpy, 0, screen.root, ModMask::ANY);
        ungrab_button(&self.dpy, ButtonIndex::ANY, screen.root, ModMask::ANY);
        let event_mask: u16 = u32::from(EventMask::BUTTON_PRESS) as u16;
        for &_m in &Keys::IGNORED_MODS {
            for (m, k, _) in self.keys.borrow().iter() {
                grab_key(&self.dpy, true, screen.root, *m | _m, *k, GrabMode::ASYNC, GrabMode::ASYNC);
            }
            grab_button(&self.dpy, false, screen.root, event_mask, GrabMode::ASYNC, GrabMode::ASYNC, screen.root, NONE, ButtonIndex::M3, u16::from(ModMask::M1) | _m);
        }
        grab_button(&self.dpy, false, screen.root, event_mask, GrabMode::SYNC, GrabMode::ASYNC, screen.root, NONE, ButtonIndex::M1, u16::from(ModMask::ANY));
        &self.dpy.flush();
        //grab_button(&self.dpy, false, screen.root, event_mask, GrabMode::ASYNC, GrabMode::ASYNC, screen.root, NONE, ButtonIndex::M1, NONE as u16);
        &self.dpy.flush();
        screen.set_tag(self, tag);
        self.screens[id].replace(Rc::new(RefCell::new(screen)));
    }
}

#[derive(Debug)]
pub enum ProcessWindow {
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
                    info!("window is a dialog window");
                    args.floating = true;
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