use x11rb::{
    protocol::{Event, xproto::*},
    NONE, CURRENT_TIME
};
use std::process::{Command, Stdio};

use log::info;

use crate::{
    config::{Keys, WMCommand},
    WindowManager, WindowLocation, CWMRes
};

pub(crate) struct EventHandler {
    keys: Keys,
    drag: DragState
}

impl WMCommand {
    fn run(&self, wm: &mut WindowManager) -> CWMRes<()> {
        use WMCommand::*;
        match self {
            KillClient => {
                info!("Killing Client");
                if let Some(win) = wm.current_tag().and_then(|tag| tag.borrow().focused_client()) {
                    wm.unmanage_window(win)?
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
                        tag.set_fullscreen(wm, win, arg)?
                    }
                }
            },
            AlwaysOnTop(arg) => {
                info!("AlwaysOnTop {:?}", arg);
                if let Some(tag) = wm.current_tag() {
                    let mut tag = tag.borrow_mut();
                    if let Some(win) = tag.focused_client() {
                        tag.set_aot(wm, win, arg)?
                    }
                }
            },
            Floating(arg) => {
                info!("Floating {:?}", arg);
                if let Some(tag) = wm.current_tag() {
                    let mut tag = tag.borrow_mut();
                    if let Some(win) = tag.focused_client() {
                        tag.set_floating(wm, win, arg)?
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
        Ok(())
    }
}


// for window things
// map request -> initial managing
// property notify -> change in state (window and panels)
// destroy notify -> unmanage window (try both desktop win and panel remove if it is not a client)
// unmap notify -> unmanage clients (set window state to withdrawn (0))

impl EventHandler {
    pub fn new(keys: Keys) -> Self {
        Self {
            keys,
            drag: DragState::default()
        }
    }
    
    pub(crate) fn handle_event(&mut self, wm: &mut WindowManager, e: Event) -> CWMRes<()> {
        match e {
            Event::ButtonPress(ev) => self.handle_button_press(wm, ev),
            Event::MotionNotify(ev) => self.handle_motion_notify(wm, ev),
            Event::ButtonRelease(ev) => self.handle_button_release(wm, ev),
            Event::DestroyNotify(ev) => self.handle_destroy_notify(wm, ev),
            Event::EnterNotify(ev) => self.handle_enter_notify(wm, ev),
            Event::KeyPress(ev) => self.handle_key_press(wm, ev),
            Event::MapRequest(ev) => self.handle_map_request(wm, ev),
            Event::ClientMessage(ev) => self.handle_client_message(wm, ev),
            Event::ConfigureRequest(ev) => self.handle_configure_request(wm, ev),
            Event::PropertyNotify(ev) => self.handle_property_notify(wm, ev),
            Event::UnmapNotify(ev) => self.handle_unmap_notify(wm, ev),
            _e => Ok(())//info!("Unhandled Event: {:?}", e)
        }
    }

    fn handle_key_press(&mut self, wm: &mut WindowManager, e: KeyPressEvent) -> CWMRes<()> {
        info!("Handling Key Press");
        let keycode = e.detail;
        let mods = e.state & Keys::IGNORED_MASK;
        for (m, k, cmd) in self.keys.iter() {
            if *m == mods.into() && *k == keycode {
                cmd.run(wm)?;
            }
        }
        Ok(())
    }
    fn handle_enter_notify(&mut self, wm: &mut WindowManager, e: EnterNotifyEvent) -> CWMRes<()> {
        info!("Handling Enter {}({})", e.event, e.child);
        if let Some(tag) = wm.current_tag() {
            tag.borrow_mut().focus_client(wm, e.event)?;
        }
        Ok(())
    }
    fn handle_map_request(&mut self, wm: &mut WindowManager, e: MapRequestEvent) -> CWMRes<()> {
        info!("Handling Map Request");
        wm.manage_window(e.window)?;
        Ok(())
    }
    fn handle_destroy_notify(&mut self, wm: &mut WindowManager, e: DestroyNotifyEvent) -> CWMRes<()> {
        info!("Handling Destroy Notify {}, {}", e.event, e.window);
        wm.unmanage_window(e.event)?;
        wm.unmanage_window(e.window)?;
        Ok(())
    }
    fn handle_unmap_notify(&mut self, wm: &mut WindowManager, e: UnmapNotifyEvent) -> CWMRes<()> {
        info!("Handling Unmap Notify {}, {}", e.event, e.window);
        wm.unmanage_window(e.event)?;
        wm.unmanage_window(e.window)?;
        Ok(())
    }
    fn handle_property_notify(&mut self, wm: &mut WindowManager, e: PropertyNotifyEvent) -> CWMRes<()> {
        let atom = get_atom_name(&wm.conn.dpy, e.atom).unwrap().reply().unwrap();
        info!("Handling Property Notify. Property {}", String::from_utf8(atom.name).unwrap());
        Ok(())
    }
    fn handle_client_message(&mut self, wm: &mut WindowManager, e: ClientMessageEvent) -> CWMRes<()> {
        let name = get_atom_name(&wm.conn.dpy, e.type_).unwrap().reply().unwrap();
        info!("Handling Client Message {}", String::from_utf8(name.name).unwrap());
        Ok(())
    }
    fn handle_configure_request(&mut self, wm: &mut WindowManager, e: ConfigureRequestEvent) -> CWMRes<()> {
        info!("Handling Configure Request");
        if let Some(window) = wm.windows.get(&e.window) {
            match window {
                WindowLocation::Panel(_) | WindowLocation::DesktopWindow(_) => {
                    configure_window(&wm.conn.dpy, e.window, &ConfigureWindowAux::from_configure_request(&e))?;

                },
                _ => ()
            }
        }
        Ok(())
    }
    fn handle_button_press(&mut self, wm: &mut WindowManager, e: ButtonPressEvent) -> CWMRes<()> {
        let win = e.child;
        info!("Handling Button Press {}", win);
        let mods = e.state & Keys::IGNORED_MASK;
        if win != 0 && self.drag.button == 0 {
            if mods == 0 && e.detail == 1 {
                info!("Raising Client");
                if let Some(tag) = wm.current_tag() {
                    tag.borrow_mut().switch_layer(wm, win)?;
                }
                allow_events(&wm.conn.dpy, Allow::REPLAY_POINTER, CURRENT_TIME)?;
            } else {
                self.drag.button = match e.detail {
                    1 => 1,
                    3 => if let Some(tag) = wm.current_tag() {
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
                    info!("Move / Resize ({})", self.drag.button);
                    self.drag.win = win;
                    self.drag.prev = (e.root_x, e.root_y);
                    grab_pointer(&wm.conn.dpy, false, wm.root, u32::from(EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION | EventMask::POINTER_MOTION_HINT) as u16, GrabMode::ASYNC, GrabMode::ASYNC, wm.root, NONE, CURRENT_TIME)?; 
                }
            }
        }
        Ok(())
    }
    fn handle_motion_notify(&mut self, wm: &mut WindowManager, _e: MotionNotifyEvent) -> CWMRes<()> {
        info!("Handling Motion");
        if let Some(monitor) = wm.current_monitor() {
            let monitor = monitor.borrow();
            let poin = query_pointer(&wm.conn.dpy, wm.root).unwrap().reply().unwrap();
            if let Some(tag) = monitor.tag() {
                match self.drag.button {
                    1 => tag.borrow_mut().move_client(wm, self.drag.win, (poin.root_x - self.drag.prev.0, poin.root_y - self.drag.prev.1), &(poin.root_x, poin.root_y))?,
                    3 => tag.borrow_mut().resize_client(wm, self.drag.win, (poin.root_x - self.drag.prev.0, poin.root_y - self.drag.prev.1), self.drag.left, self.drag.top)?,
                    _ => ()
                }
            }
            self.drag.prev = (poin.root_x, poin.root_y);
        }
        Ok(())
    }
    fn handle_button_release(&mut self, wm: &mut WindowManager, e: ButtonReleaseEvent) -> CWMRes<()> {
        info!("Handling Button Release");
        if e.detail == self.drag.button {
            self.drag.button = 0;
            ungrab_pointer(&wm.conn.dpy, CURRENT_TIME)?;
        }
        Ok(())
    }
    
}

#[derive(Default)]
pub(crate) struct DragState {
    button: u8,
    win: Window,
    prev: (i16, i16),
    left: bool,
    top: bool
}