use x11rb::{
    connection::Connection,
    protocol::{xproto::*, randr::*},
    x11_utils::Serialize,
    NONE
};
use std::{
    collections::{HashMap, HashSet},
    cell::RefCell,
    rc::Rc,
};
use log::info;
use crate::{
    desktop_window::DesktopWindow,
    panel::Panel,
    client::ClientArgs,
    tag::Tag,
    utils::Rect, 
    WindowManager, AtomCollection, WindowLocation, CWMRes
};

pub(crate) struct Monitor {
    pub(crate) id: usize,
    pub(crate) monitor_id: u32,
    pub(crate) size: Rect,
    pub(crate) tag: Option<Rc<RefCell<Tag>>>,
    pub(crate) panels: HashMap<Window, Panel>,
    pub(crate) desktop_windows: HashMap<Window, DesktopWindow>,
}

impl Monitor {
    pub(crate) fn tag(&self) -> Option<&Rc<RefCell<Tag>>> {
        self.tag.as_ref()
    }

    // if tag is not none then it should be valid, otherwise it is disregarded and a new one is created
    pub(crate) fn set_tag(&mut self, wm: &mut WindowManager, tag: Option<usize>) -> CWMRes<()> {
        if tag.and_then(|id| self.tag().map(|tag| (id, tag))).map(|(id, tag)| tag.borrow().id != id).unwrap_or(true) {
            let new_tag = tag.and_then(|id| wm.tag(id)).unwrap_or_else(|| wm.temp_tag());
            if let Some(mut tag) = self.tag().map(|x| x.borrow_mut()) {
                if let Some(screen) = new_tag.borrow().monitor.and_then(|id| wm.monitor(id)) {
                    let mut screen = screen.borrow_mut();
                    tag.set_monitor(wm, &mut screen)?;
                    screen.tag = self.tag.clone();
                } else {
                    tag.hide(wm)?;
                }
            }
            if let Some(screen) = new_tag.borrow().monitor.and_then(|id| wm.monitor(id)) {
                if let Some(tag) = self.tag() {
                    let mut screen = screen.borrow_mut();
                    tag.borrow_mut().set_monitor(wm, &mut screen)?;
                    screen.tag = self.tag.clone();
                }
            }
            new_tag.borrow_mut().set_monitor(wm, self)?;
            self.tag.replace(new_tag);
        }
        Ok(())
    }

    pub(crate) fn manage_window(&mut self, wm: &WindowManager, win: Window) -> CWMRes<(WindowLocation, Window)> {
        let type_cookie = get_property(&wm.conn.dpy, false, win, wm.atoms._NET_WM_WINDOW_TYPE, AtomEnum::ATOM, 0, 2048).unwrap();
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
            ProcessWindow::Client(args) => self.tag().unwrap().borrow_mut().manage(wm, win, args, self),
            ProcessWindow::Desktop => self.desktop_window_register(wm, win).map(|x| (x, win)),
            ProcessWindow::Panel => self.panel_register(wm, win).map(|x| (x, win))
        }
    }
}

impl WindowManager {
    pub(crate) fn add_monitor(&mut self, tag: Option<usize>, monitor: MonitorInfo) -> CWMRes<()> {
        let id = {
            let hooks = &mut self.hooks.borrow_mut().monitor_focused;
            let monitors = &mut self.monitors;
            self.free_monitors.pop().unwrap_or_else(|| {monitors.push(None); hooks.push((Vec::new(), None)); monitors.len() - 1})
        };
        
        let mut monitor = Monitor {
            id,
            monitor_id: monitor.name,
            size: Rect::new(monitor.x, monitor.y, monitor.width, monitor.height),
            tag: None,
            panels: HashMap::new(),
            desktop_windows: HashMap::new(),
        };
        
        monitor.set_tag(self, tag)?;
        self.monitors[id].replace(Rc::new(RefCell::new(monitor)));
        Ok(())
    }
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