use anyhow::Result;
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
pub mod utils;
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
mod rules;

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
        _NET_WM_STATE_DEMANDS_ATTENTION,
        _NET_ACTIVE_WINDOW,
        _NET_SUPPORTED,
        _NET_SUPPORTING_WM_CHECK,
        WM_STATE,
        WM_PROTOCOLS,
        WM_DELETE_WINDOW,
        UTF8_STRING,
    }
}

#[derive(Copy, Clone)]
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
    temp_tags: Vec<Atom>,
    tag_order: Vec<Atom>,
    free_temp: Vec<String>,
    monitors: HashMap<Atom, Monitor>,
    focused_monitor: Atom,
    prev_monitor: Atom,
    windows: HashMap<Window, WindowLocation>,
    running: bool,
    supporting: bool,
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
        change_window_attributes(
            &dpy,
            root,
            &ChangeWindowAttributesAux::new().event_mask(
                EventMask::SUBSTRUCTURE_REDIRECT
                    | EventMask::SUBSTRUCTURE_NOTIFY
                    | EventMask::STRUCTURE_NOTIFY,
            ),
        )?;
        ungrab_key(&dpy, 0, root, ModMask::ANY)?;
        ungrab_button(&dpy, ButtonIndex::ANY, root, ModMask::ANY)?;
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
            )?;
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
            )?;
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
            )?;
        }

        select_input(&dpy, root, NotifyMask::SCREEN_CHANGE)?;
        dpy.flush()?;

        let wm = Self {
            aux: Aux::new(dpy, root, pref_screen)?,
            monitors: HashMap::new(),
            tags: HashMap::new(),
            free_tags: HashSet::new(),
            temp_tags: Vec::new(),
            free_temp: Vec::new(),
            tag_order: Vec::new(),
            focused_monitor: 0,
            prev_monitor: 0,
            windows: HashMap::new(),
            running: true,
            supporting: false,
        };

        Ok(wm)
    }
}

pub fn run_wm() {
    info!("CWM Starting");
    let mut wm = match WindowManager::new() {
        Ok(wm) => wm,
        Err(e) => {
            info!("Error: {:?}", e);
            return
        }
    };
    let mut event_handler = EventHandler::new();
    wm.aux.hooks.config();
    if let Err(e) = wm.update_monitors() {
        info!("Error: {:?}", e);
        return
    }

    while wm.running {
        wm.aux.wait_for_updates();
        while let Some(event) = wm.aux.dpy.poll_for_event().unwrap_or_else(|e| {
            wm.running = false;
            info!("Error: {:?}", e);
            None
        }) {
            let _ = event_handler.handle_event(&mut wm, event);
        }

        if let Err(e) = wm.handle_connections() {
            info!("Error: {:?}", e);
            return
        }
        if let Err(e) = wm.aux.dpy.flush() {
            info!("Error: {:?}", e);
            return
        }
    }
    info!("CWM Stopping");
}
