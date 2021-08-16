use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use x11rb::{
    atom_manager,
    connection::Connection,
    protocol::{randr::*, xproto::*},
    rust_connection::RustConnection,
    NONE,
};

use log::info;
mod config;
mod utils;
use config::IGNORED_MODS;
mod monitor;
use monitor::Monitor;
mod tag;
use tag::Tag;
mod events;
use events::EventHandler;
pub mod connections;
use connections::Aux;
mod hooks;
use hooks::Hooks;

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
        _NET_WM_DESKTOP,
        WM_STATE,
        WM_PROTOCOLS,
        WM_DELETE_WINDOW,
        UTF8_STRING,
    }
}

enum WindowLocation {
    Client(Atom, usize),
    Panel(Atom),
    DesktopWindow(Atom),
    Monitor(Atom),
    _Unmanaged,
}

pub struct WindowManager {
    aux: Aux,
    tags: HashMap<Atom, Tag>,
    free_tags: HashSet<Atom>,
    temp_tags: HashSet<Atom>,
    tag_order: Vec<Atom>,
    monitors: HashMap<Atom, Monitor>,
    focused_monitor: Atom,
    prev_monitor: Atom,
    windows: HashMap<Window, WindowLocation>,
    running: bool,
}

impl WindowManager {
    fn unmanage_window(&mut self, win: Window) -> Result<()> {
        if let Some(location) = self.windows.remove(&win) {
            //info!("unmanage window, {} {:?}", win, location);
            match location {
                WindowLocation::Client(tag, client) => self.unmanage_client(tag, client)?,
                WindowLocation::DesktopWindow(mon) => self.desktop_window_unregister(mon, win),
                WindowLocation::Panel(mon) => self.panel_unregister(mon, win)?,
                _ => (),
            }
        }
        Ok(())
    }

    fn new() -> Result<Self> {
        let (dpy, pref_screen) = RustConnection::connect(None).unwrap();
        let root = dpy.setup().roots[pref_screen].root;
        let monitors_cookie = get_monitors(&dpy, root, true).context(crate::code_loc!())?;
        change_window_attributes(
            &dpy,
            root,
            &ChangeWindowAttributesAux::new().event_mask(
                EventMask::SUBSTRUCTURE_REDIRECT
                    | EventMask::SUBSTRUCTURE_NOTIFY
                    | EventMask::STRUCTURE_NOTIFY,
            ),
        )
        .context(crate::code_loc!())?;
        ungrab_key(&dpy, 0, root, ModMask::ANY).context(crate::code_loc!())?;
        ungrab_button(&dpy, ButtonIndex::ANY, root, ModMask::ANY).context(crate::code_loc!())?;
        let event_mask: u16 = u32::from(EventMask::BUTTON_PRESS) as u16;
        for &_m in &IGNORED_MODS {
            grab_button(
                &dpy,
                false,
                root,
                event_mask,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                root,
                NONE,
                ButtonIndex::M3,
                u16::from(ModMask::M4) | _m,
            )
            .context(crate::code_loc!())?;
            grab_button(
                &dpy,
                false,
                root,
                event_mask,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
                root,
                NONE,
                ButtonIndex::M1,
                u16::from(ModMask::M4) | _m,
            )
            .context(crate::code_loc!())?;
            grab_button(
                &dpy,
                false,
                root,
                event_mask,
                GrabMode::SYNC,
                GrabMode::ASYNC,
                root,
                NONE,
                ButtonIndex::M1,
                _m,
            )
            .context(crate::code_loc!())?;
        }

        dpy.flush().context(crate::code_loc!())?;
        let monitors = monitors_cookie.reply().context(crate::code_loc!())?;
        select_input(&dpy, root, NotifyMask::SCREEN_CHANGE).context(crate::code_loc!())?;
        let mut wm = Self {
            aux: Aux::new(dpy, root)?,
            monitors: HashMap::new(),
            tags: HashMap::new(),
            free_tags: HashSet::new(),
            temp_tags: HashSet::new(),
            tag_order: Vec::new(),
            focused_monitor: 0,
            prev_monitor: 0,
            windows: HashMap::new(),
            running: true,
        };

        for tag in ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"] {
            wm.add_tag(tag)?;
        }

        wm.update_monitors()?;

        Ok(wm)
    }
}

pub fn run_wm() {
    let mut wm = WindowManager::new().unwrap();
    let mut event_handler = EventHandler::new();
    wm.aux.hooks.config();

    while wm.running {
        wm.aux.wait_for_updates();
        while let Some(event) = wm.aux.dpy.poll_for_event().unwrap_or_else(|_| {
            wm.running = false;
            None
        }) {
            event_handler.handle_event(&mut wm, event).unwrap();
        }

        wm.handle_connections().unwrap();
        wm.aux.dpy.flush().unwrap();
    }
}

#[macro_export]
macro_rules! code_loc {
    () => {
        format!("{}:{}", file!(), line!())
    };
}
