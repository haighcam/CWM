use x11rb::{
    connection::Connection,
    protocol::{xproto::*, randr::*},
    rust_connection::RustConnection,
    atom_manager, NONE
};
use std::{
    os::unix::io::{AsRawFd, RawFd},
    collections::HashMap,
    cell::{RefCell, Cell},
    rc::Rc,
};
use nix::poll::{poll, PollFd, PollFlags};

use log::info;
mod utils;
mod config;
use config::{Theme, Keys};
mod monitor;
use monitor::Monitor;
mod tag;
use tag::Tag;
mod events;
use events::EventHandler;
pub mod connections;
use connections::Connections;
mod hooks;
use hooks::Hooks;
mod desktop_window;
mod panel;
mod layout;
mod layers;
mod client;
mod error;
use error::CWMRes;

type CwmRes<T> = CWMRes<T>;

pub mod new;

atom_manager! {
    pub AtomCollection: AtomCollectionCookie {
        _NET_WM_NAME,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DOCK,
        _NET_WM_WINDOW_TYPE_TOOLBAR,
        _NET_WM_WINDOW_TYPE_UTILITY,
        _NET_WM_WINDOW_TYPE_DIALOG,
        _NET_WM_WINDOW_TYPE_DESKTOP,
        _NET_WM_WINDOW_TYPE_NOTIFICATION,
        _NET_WM_STRUT,
        _NET_WM_STRUT_PARTIAL,
        _NET_WM_STATE,
        _NET_WM_STATE_FULLSCREEN,
        _NET_WM_STATE_STICKY,
        WM_STATE,
        UTF8_STRING,
    }
}

pub struct WindowManager {
    conn: Connections,
    monitors: Vec<Option<Rc<RefCell<Monitor>>>>,
    monitor: usize,
    free_monitors: Vec<usize>,
    tags: Vec<Option<Rc<RefCell<Tag>>>>,
    free_tags: Vec<usize>,
    windows: HashMap<Window, WindowLocation>,
    atoms: AtomCollection,
    theme: Theme,
    running: Cell<bool>,
    root: Window,
    hooks: RefCell<Hooks>
}

#[derive(Debug)]
pub enum WindowLocation {
    Client(usize), // tag
    Panel(usize), // monitor
    DesktopWindow(usize), // monitor
    Unmanaged
}

impl WindowManager {
    fn monitor(&self, id: usize) -> Option<Rc<RefCell<Monitor>>> {
        self.monitors.get(id).and_then(|x| x.clone())
    }
    fn tag(&self, id: usize) -> Option<Rc<RefCell<Tag>>> {
        self.tags.get(id).and_then(|x| x.clone())
    }

    fn current_monitor(&self) -> Option<Rc<RefCell<Monitor>>> {
        self.monitors.get(self.monitor).and_then(|monitor| monitor.clone())
    }

    fn current_tag(&self) -> Option<Rc<RefCell<Tag>>> {
        self.monitors.get(self.monitor).and_then(|monitor| monitor.as_ref()).and_then(|monitor| monitor.borrow().tag().cloned())
    }

    fn manage_window(&mut self, win: Window) -> CWMRes<()> {
        if let Some(monitor) = self.current_monitor() {
            let (location, win) = monitor.borrow_mut().manage_window(self, win)?;
            self.windows.insert(win, location);
        }
        Ok(())
    }

    fn unmanage_window(&mut self, win: Window) -> CWMRes<()> {
        if let Some(location) = self.windows.remove(&win) {
            info!("unmanage window, {} {:?}", win, location);
            match location {
                WindowLocation::Client(tag) => if let Some(mut tag) = self.tags[tag].as_ref().map(|x| x.borrow_mut()) {
                    tag.unmanage(self, win)?
                },
                WindowLocation::DesktopWindow(monitor) => if let Some(mut monitor) = self.monitors[monitor].as_ref().map(|x| x.borrow_mut()) {
                    monitor.desktop_window_unregister(win)
                },
                WindowLocation::Panel(monitor) => if let Some(mut monitor) = self.monitors[monitor].as_ref().map(|x| x.borrow_mut()) {
                    monitor.panel_unregister(self, win)?
                },
                _ => ()
            }
        }
        Ok(())
    }

    fn new(keys: &Keys) -> CWMRes<Self> {
        let (dpy, pref_screen) = RustConnection::connect(None).unwrap();
        let root = dpy.setup().roots[pref_screen].root;
        let atoms_cookie = AtomCollection::new(&dpy)?;
        let monitors_cookie = get_monitors(&dpy, root, true)?;
        let theme = Theme::default();
        let tags = (&["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"]).iter().enumerate().map(|(id, &name)| Some(Rc::new(RefCell::new(Tag::new(id, name))))).collect();
        change_window_attributes(&dpy, root, &ChangeWindowAttributesAux::new().event_mask(EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY | EventMask::STRUCTURE_NOTIFY))?;
        ungrab_key(&dpy, 0, root, ModMask::ANY)?;
        ungrab_button(&dpy, ButtonIndex::ANY, root, ModMask::ANY)?;
        let event_mask: u16 = u32::from(EventMask::BUTTON_PRESS) as u16;
        for &_m in &Keys::IGNORED_MODS {
            for (m, k, _) in keys.iter() {
                grab_key(&dpy, true, root, *m | _m, *k, GrabMode::ASYNC, GrabMode::ASYNC)?;
            }
            grab_button(&dpy, false, root, event_mask, GrabMode::ASYNC, GrabMode::ASYNC, root, NONE, ButtonIndex::M3, u16::from(ModMask::M1) | _m)?;
        }
        grab_button(&dpy, false, root, event_mask, GrabMode::SYNC, GrabMode::ASYNC, root, NONE, ButtonIndex::M1, u16::from(ModMask::ANY))?;
        dpy.flush()?;
        let atoms = atoms_cookie.reply()?;
        let monitors = monitors_cookie.reply()?;

        let mut wm = Self {
            conn: Connections::new(dpy),
            monitors: Vec::new(),
            monitor: 0,
            free_monitors: Vec::new(),
            tags,
            free_tags: Vec::new(),
            windows: HashMap::new(),
            atoms,
            theme,
            running: Cell::new(true),
            root,
            hooks: RefCell::new(Hooks::default())
        };
        for (id, monitor) in monitors.monitors.into_iter().enumerate() {
            if monitor.primary {
                wm.monitor = id;
            }
            wm.add_monitor(Some(id), monitor)?;
        }
        Ok(wm)
    }
}

pub fn run_wm() {
    let keys = Keys::default();
    let mut wm = WindowManager::new(&keys).unwrap();
    let mut event_handler = EventHandler::new(keys);

    while wm.running.get() {
        while let Some(event) = wm.conn.dpy.poll_for_event().unwrap_or_else(|_| {
            wm.running.set(false);
            None
        }) {
            event_handler.handle_event(&mut wm, event).unwrap();
        }

        wm.handle_connections();
        wm.conn.dpy.flush().unwrap();
        wm.conn.wait_for_updates();

    }
}