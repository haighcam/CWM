use x11rb::{
    connection::Connection,
    rust_connection::RustConnection,
    protocol::{xproto::*, randr::*},
    properties::*,
    x11_utils::Serialize,
    COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME, NONE
};
use std::collections::HashMap;
use log::info;
use crate::utils::{Rect, stack_::Stack};
use crate::{Connections, AtomCollection, CwmRes, Hooks};
use crate::config::{Theme, Keys};


mod monitor;
mod tag;
mod events;
use monitor::Monitor;
use tag::Tag;
use events::EventHandler;

enum WindowLocation {
    Client(usize, usize),
    Panel(usize),
    DesktopWindow(usize)
}

pub struct WindowManager {
    root: Window,
    conn: Connections,
    tags: Vec<Tag>,
    free_tags: Vec<usize>,
    monitors: Vec<Monitor>,
    free_monitors: Vec<usize>,
    focused_monitor: usize,
    theme: Theme,
    atoms: AtomCollection,
    windows: HashMap<Window, WindowLocation>,
    hooks: Hooks,
    running: bool
}

impl WindowManager {
    fn unmanage_window(&mut self, win: Window) -> CwmRes<()> {
        if let Some(location) = self.windows.remove(&win) {
            //info!("unmanage window, {} {:?}", win, location);
            match location {
                WindowLocation::Client(tag, client) => self.unmanage_client(tag, client)?,
                WindowLocation::DesktopWindow(mon) => self.desktop_window_unregister(mon, win),
                WindowLocation::Panel(mon) => self.panel_unregister(mon, win)?,
                _ => ()
            }
        }
        Ok(())
    }

    fn new(keys: &Keys) -> CwmRes<Self> {
        let (dpy, pref_screen) = RustConnection::connect(None).unwrap();
        let root = dpy.setup().roots[pref_screen].root;
        let atoms_cookie = AtomCollection::new(&dpy)?;
        let monitors_cookie = get_monitors(&dpy, root, true)?;
        let theme = Theme::default();
        let tags = (&["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"]).iter().enumerate().map(|(id, &name)| Tag::new(id, name)).collect();
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
            root,
            conn: Connections::new(dpy),
            monitors: Vec::new(),
            free_monitors: Vec::new(),
            tags,
            free_tags: Vec::new(),
            focused_monitor: 0,
            theme,
            atoms,
            windows: HashMap::new(),
            hooks: Hooks::default(),
            running: true
        };
        for (id, monitor) in monitors.monitors.into_iter().enumerate() {
            if monitor.primary {
                wm.focused_monitor = id;
            }
            info!("addng monitor {:?}", monitor);
            wm.add_monitor(id, monitor)?;
        }
        Ok(wm)
    }
}

pub fn run_wm() {
    let keys = Keys::default();
    let mut wm = WindowManager::new(&keys).unwrap();
    let mut event_handler = EventHandler::new(keys);

    while wm.running {
        while let Some(event) = wm.conn.dpy.poll_for_event().unwrap_or_else(|_| {
            wm.running = false;
            None
        }) {
            event_handler.handle_event(&mut wm, event).unwrap();
        }

        //wm.handle_connections();
        wm.conn.dpy.flush().unwrap();
        wm.conn.wait_for_updates();

    }
}