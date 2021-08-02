use x11rb::{
    connection::Connection,
    protocol::{Event, xproto::*},
    NONE, CURRENT_TIME
};
use std::{
    collections::{HashMap, HashSet},
    cell::RefCell,
    rc::Rc,
    process::{Command, Stdio}
};

use log::{info, warn};

use crate::{utils::Rect, config::{Keys, WMCommand}};
use super::{WindowManager, WindowLocation};

impl WMCommand {
    fn run(&self, wm: &mut WindowManager<impl Connection>) {
        use WMCommand::*;
        match self {
            KillClient => {
                info!("Killing Client");
                if let Some(win) = wm.current_tag().and_then(|tag| tag.borrow().focused_client()) {
                    wm.unmanage_window(win)
                }
            },
            CloseWM => {
                wm.running.set(false);
                info!("Exiting");
            },
            Spawn(cmd, args) => {
                info!("Spawning");
                Command::new(cmd).args(args).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
            },
            Fullscreen(arg) => {
                info!("Fullscreen {:?}", arg);
                if let Some(tag) = wm.current_tag() {
                    let mut tag = tag.borrow_mut();
                    if let Some(win) = tag.focused_client() {
                        tag.set_fullscreen(wm, win, arg)
                    }
                }
            },
            AlwaysOnTop(arg) => {
                info!("AlwaysOnTop {:?}", arg);
                if let Some(tag) = wm.current_tag() {
                    let mut tag = tag.borrow_mut();
                    if let Some(win) = tag.focused_client() {
                        tag.set_aot(wm, win, arg)
                    }
                }
            },
            Floating(arg) => {
                info!("Floating {:?}", arg);
                if let Some(tag) = wm.current_tag() {
                    let mut tag = tag.borrow_mut();
                    if let Some(win) = tag.focused_client() {
                        tag.set_floating(wm, win, arg)
                    }
                }
            },
            Sticky(arg) => {
                info!("Sticky {:?}", arg);
                if let Some(tag) = wm.current_tag() {
                    let mut tag = tag.borrow_mut();
                    if let Some(win) = tag.focused_client() {
                        tag.set_sticky(win, arg)
                    }
                }
            }
        }
    }
}


// for window things
// map request -> initial managing
// property notify -> change in state (window and panels)
// destroy notify -> unmanage window (try both desktop win and panel remove if it is not a client)
// unmap notify -> unmanage clients (set window state to withdrawn (0))

impl<X: Connection> WindowManager<X> {
    pub fn handle_event(&mut self, e: Event) {
        match e {
            Event::ButtonPress(ev) => self.handle_button_press(ev),
            Event::MotionNotify(ev) => self.handle_motion_notify(ev),
            Event::ButtonRelease(ev) => self.handle_button_release(ev),
            Event::DestroyNotify(ev) => self.handle_destroy_notify(ev),
            Event::EnterNotify(ev) => self.handle_enter_notify(ev),
            Event::KeyPress(ev) => self.handle_key_press(ev),
            Event::MapRequest(ev) => self.handle_map_request(ev),
            Event::ClientMessage(ev) => self.handle_client_message(ev),
            Event::ConfigureRequest(ev) => self.handle_configure_request(ev),
            e => ()//info!("Unhandled Event: {:?}", e)
        }
    }

    fn handle_key_press(&mut self, e: KeyPressEvent) {
        info!("Handling Key Press");
        let keycode = e.detail;
        let mods = e.state & Keys::IGNORED_MASK;
        for (m, k, cmd) in self.keys.clone().borrow().iter() {
            if *m == mods.into() && *k == keycode {
                cmd.run(self);
            }
        }
    }
    fn handle_enter_notify(&mut self, e: EnterNotifyEvent) {
        info!("Handling Enter {}({})", e.event, e.child);
        if let Some(tag) = self.current_tag() {
            tag.borrow_mut().focus_client(self, e.event);
        }
    }
    fn handle_map_request(&mut self, e: MapRequestEvent) {
        info!("Handling Map Request");
        self.manage_window(e.window);
    }
    fn handle_destroy_notify(&mut self, e: DestroyNotifyEvent) {
        info!("Handling Destroy Notify {}, {}", e.event, e.window);
        self.unmanage_window(e.event);
        self.unmanage_window(e.window);
    }
    fn handle_unmap_notify(&mut self, e: UnmapNotifyEvent) {
        info!("Handling Unmap Notify {}, {}", e.event, e.window);
        self.unmanage_window(e.event);
        self.unmanage_window(e.window);
    }
    fn handle_property_notify(&mut self, e: PropertyNotifyEvent) {
        let atom = get_atom_name(&self.dpy, e.atom).unwrap().reply().unwrap();
        info!("Handling Property Notify. Property {}", String::from_utf8(atom.name).unwrap());
    }
    fn handle_client_message(&mut self, e: ClientMessageEvent) {
        let name = get_atom_name(&self.dpy, e.type_).unwrap().reply().unwrap();
        info!("Handling Client Message {}", String::from_utf8(name.name).unwrap());
    }
    fn handle_configure_request(&mut self, e: ConfigureRequestEvent) {
        info!("Handling Configure Request");
        if let Some(window) = self.windows.get(&e.window) {
            match window {
                WindowLocation::Panel(_) | WindowLocation::DesktopWindow(_) => {
                    configure_window(&self.dpy, e.window, &ConfigureWindowAux::from_configure_request(&e));

                },
                _ => ()
            }
        }
    }
    fn handle_button_press(&mut self, e: ButtonPressEvent) {
        let win = e.child;
        info!("Handling Button Press {}", win);
        let mods = e.state & Keys::IGNORED_MASK;
        if win != 0 && self.drag.button == 0 {
            if mods == 0 && e.detail == 1 {
                if let Some(tag) = self.current_tag() {
                    tag.borrow_mut().switch_layer(self, win);
                }
            } else {
                self.drag.button = match e.detail {
                    1 => 1,
                    3 => if let Some(tag) = self.current_tag() {
                        if let Some(client) = tag.borrow().client(win) {
                            if let Some(rect) = client.borrow().get_rect() {
                                let center = (rect.x + (rect.width / 2) as i16, rect.y + (rect.height / 2) as i16);
                                self.drag.left = center.0 > e.root_x;
                                self.drag.top = center.1 > e.root_y;
                                3
                            } else {
                                0
                            }
                        } else {
                            0
                        }
                    } else {
                        0
                    },
                    _ => 0
                };
                if self.drag.button != 0 {
                    if let Some(screen) = self.current_screen() {
                        self.drag.win = win;
                        self.drag.prev = (e.root_x, e.root_y);
                        let root = screen.borrow().root();
                        grab_pointer(&self.dpy, false, root, u32::from(EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION | EventMask::POINTER_MOTION_HINT) as u16, GrabMode::ASYNC, GrabMode::ASYNC, root, NONE, CURRENT_TIME); 
                    } else {
                        self.drag.button = 0;
                    }
                }
            }
        }
        allow_events(&self.dpy, Allow::REPLAY_POINTER, CURRENT_TIME);
    }
    fn handle_motion_notify(&mut self, _e: MotionNotifyEvent) {
        info!("Handling Motion");
        if let Some(screen) = self.current_screen() {
            let screen = screen.borrow();
            let poin = query_pointer(&self.dpy, screen.root()).unwrap().reply().unwrap();
            if let Some(mut tag) = screen.tag() {
                match self.drag.button {
                    1 => tag.borrow_mut().move_client(self, self.drag.win, (poin.root_x - self.drag.prev.0, poin.root_y - self.drag.prev.1), &(poin.root_x, poin.root_y)),
                    3 => tag.borrow_mut().resize_client(self, self.drag.win, (poin.root_x - self.drag.prev.0, poin.root_y - self.drag.prev.1), self.drag.left, self.drag.top),
                    _ => ()
                }
            }
            self.drag.prev = (poin.root_x, poin.root_y);
        }
    }
    fn handle_button_release(&mut self, e: ButtonReleaseEvent) {
        info!("Handling Button Release");
        if e.detail == self.drag.button {
            self.drag.button = 0;
            ungrab_pointer(&self.dpy, CURRENT_TIME);
        }
    }
}

#[derive(Default)]
pub struct DragState {
    button: u8,
    win: Window,
    prev: (i16, i16),
    left: bool,
    top: bool
}
/*
#[derive(PartialEq)]
pub enum DragState {
    None,
    Move { win: Window, prev: (i16, i16) },
    Resize { win: Window, prev: (i16, i16), left: bool, top: bool }
}

impl DragState {
    fn button(&self) -> u8 {
        match self {
            Self::None => 0,
            Self::Move {..} => 1,
            Self::Resize {..} => 3,
        }
    }
}
*/