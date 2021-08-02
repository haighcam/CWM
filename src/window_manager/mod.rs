use x11rb::{
    connection::Connection,
    protocol::xproto::*,
    atom_manager
};
use std::{
    collections::HashMap,
    cell::{RefCell, Cell},
    rc::Rc,
};
use log::info;
use crate::utils::Rect;
use crate::config::{Theme, Keys};
mod screen;
use screen::{Screen, Tag};
mod events;
use events::DragState;
mod ipc;
use ipc::IPC;

atom_manager! {
    AtomCollection: AtomCollectionCookie {
        _NET_SUPPORTED,
        _NET_ACTIVE_WINDOW,
        _NET_CLIENT_LIST,
        _NET_WM_DESKTOP,
        _NET_DESKTOP_NAMES,
        _NET_NUMBER_OF_DESKTOPS,
        _NET_CURRENT_DESKTOP,
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
    }
}

pub struct WindowManager<X: Connection> {
    dpy: X,
    screens: Vec<Option<Rc<RefCell<Screen>>>>,
    screen: usize,
    free_screens: Vec<usize>,
    tags: Vec<Option<Rc<RefCell<Tag>>>>,
    free_tags: Vec<usize>,
    windows: HashMap<Window, WindowLocation>,
    client_list: Vec<Window>,
    atoms: AtomCollection,
    theme: Theme,
    keys: Rc<RefCell<Keys>>,
    drag: DragState,
    running: Cell<bool>,
}

#[derive(Debug)]
pub enum WindowLocation {
    Client(usize), // tag
    Panel(usize), // screen
    DesktopWindow(usize), // screen
    Unmanaged
}

impl<X: Connection> WindowManager<X> {
    fn screen(&self, id: usize) -> Option<Rc<RefCell<Screen>>> {
        self.screens.get(id).and_then(|x| x.clone())
    }
    fn tag(&self, id: usize) -> Option<Rc<RefCell<Tag>>> {
        self.tags.get(id).and_then(|x| x.clone())
    }

    fn current_screen(&self) -> Option<Rc<RefCell<Screen>>> {
        self.screens.get(self.screen).and_then(|screen| screen.clone())
    }

    fn current_tag(&self) -> Option<Rc<RefCell<Tag>>> {
        self.screens.get(self.screen).and_then(|screen| screen.as_ref()).and_then(|screen| screen.borrow().tag().map(|x| x.clone()))
    }

    fn manage_window(&mut self, win: Window) {
        if let Some(screen) = self.current_screen() {
            let (location, win) = screen.borrow_mut().manage_window(self, win);
            self.windows.insert(win, location);
        }
    }

    fn unmanage_window(&mut self, win: Window) {
        if let Some(location) = self.windows.remove(&win) {
            info!("unmanage window, {} {:?}", win, location);
            match location {
                WindowLocation::Client(tag) => if let Some(mut tag) = self.tags[tag].as_ref().map(|x| x.borrow_mut()) {
                    tag.unmanage(self, win)
                },
                WindowLocation::DesktopWindow(screen) => if let Some(mut screen) = self.screens[screen].as_ref().map(|x| x.borrow_mut()) {
                    screen.desktop_window_unregister(win)
                },
                WindowLocation::Panel(screen) => if let Some(mut screen) = self.screens[screen].as_ref().map(|x| x.borrow_mut()) {
                    screen.panel_unregister(self, win)
                },
                _ => ()
            }
        }
    }

    fn new(dpy: X, pref_screen: usize) -> Self {
        let atoms_cookie = AtomCollection::new(&dpy).unwrap();
        let theme = Theme::default();
        let keys = Rc::new(RefCell::new(Keys::default()));
        let tags = (&["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"]).iter().enumerate().map(|(id, &name)| Some(Rc::new(RefCell::new(Tag::new(id, name))))).collect();
        let roots = dpy.setup().roots.clone();
        let atoms = atoms_cookie.reply().unwrap();

        let mut wm = Self {
            dpy,
            screens: Vec::new(),
            screen: pref_screen,
            free_screens: Vec::new(),
            tags,
            free_tags: Vec::new(),
            windows: HashMap::new(),
            client_list: Vec::new(),
            atoms,
            theme,
            keys,
            drag: DragState::default(),
            running: Cell::new(true)
        };
        roots.iter().enumerate().for_each(|(tag_id, screen)| wm.add_screen(Some(tag_id), screen));
        wm
    }

    fn run(mut self) {
        while self.running.get() {
            self.dpy.flush();
            match self.dpy.wait_for_event() {
                Ok(ev) => {
                    self.handle_event(ev);
                },
                Err(e) => {
                    self.running.set(false);
                }
            }
        }
    }
}

pub fn run_wm() {
    let mut wm = x11rb::connect(None).map(|(dpy, screen)| WindowManager::new(dpy, screen)).unwrap();
    let mut ipc = IPC::new();

    while wm.running.get() {
        while let Some(event) = wm.dpy.poll_for_event().unwrap_or_else(|e| {
            wm.running.set(false);
            None
        }) {
            wm.handle_event(event)
        }

        ipc.update(&mut wm);
    }

    wm.run();
}