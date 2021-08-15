use anyhow::{Context, Result};
use log::info;
use x11rb::{
    protocol::{randr::*, xproto::*, Event},
    CURRENT_TIME, NONE,
};

use super::config::IGNORED_MASK;
use super::connections::SetArg;
use super::{WindowLocation, WindowManager};
use super::tag::NodeContents;

pub(crate) struct EventHandler {
    drag: DragState,
}

// for window things
// map request -> initial managing
// property notify -> change in state (window and panels)
// destroy notify -> unmanage window (try both desktop win and panel remove if it is not a client)
// unmap notify -> unmanage clients (set window state to withdrawn (0))

impl EventHandler {
    pub fn new() -> Self {
        Self {
            drag: DragState::default(),
        }
    }

    pub fn handle_event(&mut self, wm: &mut WindowManager, e: Event) -> Result<()> {
        match e {
            Event::ButtonPress(ev) => self.handle_button_press(wm, ev),
            Event::MotionNotify(ev) => self.handle_motion_notify(wm, ev),
            Event::ButtonRelease(ev) => self.handle_button_release(wm, ev),
            Event::DestroyNotify(ev) => self.handle_destroy_notify(wm, ev),
            Event::EnterNotify(ev) => self.handle_enter_notify(wm, ev),
            Event::MapRequest(ev) => self.handle_map_request(wm, ev),
            Event::ClientMessage(ev) => self.handle_client_message(wm, ev),
            Event::ConfigureRequest(ev) => self.handle_configure_request(wm, ev),
            Event::PropertyNotify(ev) => self.handle_property_notify(wm, ev),
            Event::UnmapNotify(ev) => self.handle_unmap_notify(wm, ev),
            Event::RandrScreenChangeNotify(ev) => self.handle_randr_norify(wm, ev),
            _e => {
                //info!("Unhandled Event: {:?}", _e);
                Ok(())
            }
        }
    }

    fn handle_randr_norify(
        &mut self,
        wm: &mut WindowManager,
        e: ScreenChangeNotifyEvent,
    ) -> Result<()> {
        info!("Randr event {:?}", e);
        wm.update_monitors()?;
        Ok(())
    }

    fn handle_enter_notify(&mut self, wm: &mut WindowManager, e: EnterNotifyEvent) -> Result<()> {
        info!("Handling Enter {}({})", e.event, e.child);
        match wm.windows.get(&e.event) {
            Some(WindowLocation::Client(tag, client)) => {
                let (tag, client) = (*tag, *client);
                if let Some(mon) = wm.tags.get(&tag).unwrap().monitor {
                    wm.set_focus(mon)?;
                }
                let tag = wm.tags.get_mut(&tag).unwrap();
                if tag.client(client).ignore_unmaps == 0 {
                    tag.focus_client(&mut wm.aux, client)?;
                }
            }
            Some(WindowLocation::Monitor(mon)) => {
                wm.set_focus(*mon)?;
            }
            _ => (),
        }
        Ok(())
    }
    fn handle_map_request(&mut self, wm: &mut WindowManager, e: MapRequestEvent) -> Result<()> {
        info!("Handling Map Request {:?}", e);
        match wm.windows.get(&e.window) {
            Some(WindowLocation::Client(..)) => {
                // wants to be mapped
                Ok(())
            }
            None => wm.manage_window(wm.focused_monitor, e.window),
            _ => Ok(()),
        }
    }
    fn handle_destroy_notify(
        &mut self,
        wm: &mut WindowManager,
        e: DestroyNotifyEvent,
    ) -> Result<()> {
        info!("Handling Destroy Notify {}, {}", e.event, e.window);
        wm.unmanage_window(e.window)?;
        Ok(())
    }
    fn handle_unmap_notify(&mut self, wm: &mut WindowManager, e: UnmapNotifyEvent) -> Result<()> {
        info!("Handling Unmap Notify {}, {}", e.event, e.window);
        info!("{:?}", e);
        let mut unmap = true;
        if let Some(WindowLocation::Client(tag, client)) = wm.windows.get(&e.window) {
            let client = wm.tags.get_mut(tag).unwrap().client_mut(*client);
            if client.ignore_unmaps != 0 {
                info!("ignore unmap {}", client.ignore_unmaps);
                client.ignore_unmaps -= 1;
                unmap = false;
            }
        }
        if unmap {
            wm.unmanage_window(e.window)?;
        }
        Ok(())
    }
    fn handle_property_notify(
        &mut self,
        wm: &mut WindowManager,
        e: PropertyNotifyEvent,
    ) -> Result<()> {
        let atom = get_atom_name(&wm.aux.dpy, e.atom).unwrap().reply().unwrap();
        info!(
            "Handling Property Notify. Property {}, {}",
            String::from_utf8(atom.name).unwrap(),
            e.window
        );
        match wm.windows.get(&e.window) {
            Some(WindowLocation::Client(tag, client)) => wm.client_property(*tag, *client, e.atom),
            Some(WindowLocation::Panel(mon)) => {
                wm.panel_property_changed(e.window, *mon, e.atom)?
            }
            _ => (),
        }
        Ok(())
    }
    fn handle_client_message(
        &mut self,
        wm: &mut WindowManager,
        e: ClientMessageEvent,
    ) -> Result<()> {
        let name = get_atom_name(&wm.aux.dpy, e.type_)
            .unwrap()
            .reply()
            .unwrap();
        info!(
            "Handling Client Message {}",
            String::from_utf8(name.name).unwrap()
        );
        Ok(())
    }
    fn handle_configure_request(
        &mut self,
        wm: &mut WindowManager,
        e: ConfigureRequestEvent,
    ) -> Result<()> {
        info!("Handling Configure Request");
        if let Some(window) = wm.windows.get(&e.window) {
            match window {
                WindowLocation::Panel(_) | WindowLocation::DesktopWindow(_) => {
                    configure_window(
                        &wm.aux.dpy,
                        e.window,
                        &ConfigureWindowAux::from_configure_request(&e),
                    )
                    .context(crate::code_loc!())?;
                }
                _ => (),
            }
        }
        Ok(())
    }
    fn handle_button_press(&mut self, wm: &mut WindowManager, e: ButtonPressEvent) -> Result<()> {
        let win = e.child;
        info!("Handling Button Press {}", win);
        let mods = e.state & IGNORED_MASK;
        if mods == 0 && e.detail == 1 {
            if self.drag.button == 0 {
                if let Some(WindowLocation::Client(tag, client)) = wm.windows.get(&win) {
                    info!("Raising Client");
                    wm.tags
                        .get_mut(tag)
                        .unwrap()
                        .switch_layer(&wm.aux, *client)?;
                }
            }
            allow_events(&wm.aux.dpy, Allow::REPLAY_POINTER, CURRENT_TIME)
                .context(crate::code_loc!())?;
        } else if self.drag.button == 0 {
            if let Some(WindowLocation::Client(tag, client)) = wm.windows.get(&win) {
                self.drag.button = match e.detail {
                    1 => 1,
                    3 => {
                        if let Some(rect) = wm.tags.get(tag).unwrap().get_rect(*client) {
                            let center = (
                                rect.x + (rect.width / 2) as i16,
                                rect.y + (rect.height / 2) as i16,
                            );
                            self.drag.left = center.0 > e.root_x;
                            self.drag.top = center.1 > e.root_y;
                            3
                        } else {
                            0
                        }
                    }
                    _ => 0,
                };
                if self.drag.button != 0 {
                    info!("Move / Resize ({})", self.drag.button);
                    self.drag.win = *client;
                    self.drag.prev = (e.root_x, e.root_y);
                    grab_pointer(
                        &wm.aux.dpy,
                        false,
                        wm.aux.root,
                        u32::from(
                            EventMask::BUTTON_RELEASE
                                | EventMask::POINTER_MOTION
                                | EventMask::POINTER_MOTION_HINT,
                        ) as u16,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                        wm.aux.root,
                        NONE,
                        CURRENT_TIME,
                    )
                    .context(crate::code_loc!())?;
                }
            }
        }
        Ok(())
    }
    fn handle_motion_notify(
        &mut self,
        wm: &mut WindowManager,
        _e: MotionNotifyEvent,
    ) -> Result<()> {
        info!("Handling Motion");
        let tag = wm.focused_tag();
        let tag = wm.tags.get_mut(&tag).unwrap();
        let poin = query_pointer(&wm.aux.dpy, wm.aux.root)
            .context(crate::code_loc!())?
            .reply()
            .context(crate::code_loc!())?;
        match self.drag.button {
            1 => {
                let pos = (poin.root_x, poin.root_y);
                if !wm
                    .monitors
                    .get(&wm.focused_monitor)
                    .unwrap()
                    .size
                    .contains(&pos)
                {
                    info!("{:?}", pos);
                    let old_tag = tag.id;
                    for mon in wm.monitors.values() {
                        if mon.size.contains(&pos) {
                            wm.focused_monitor = mon.id;
                            break;
                        }
                    }
                    if old_tag != wm.focused_tag() {
                        let tag = wm.tags.get_mut(&old_tag).unwrap();
                        let size = tag.size.clone();
                        let node = &mut tag.node_mut(tag.client(self.drag.win).node);
                        if let NodeContents::Leaf(leaf) = &mut node.info {
                            if pos.0 < size.x {
                                leaf.floating.x += size.width as i16;
                            } else if pos.0 >= size.x + size.width as i16 {
                                leaf.floating.x -= size.width as i16;
                            }
                            if pos.1 < size.y {
                                leaf.floating.y += size.height as i16;
                            } else if pos.1 >= size.y + size.height as i16 {
                                leaf.floating.y -= size.height as i16;
                            }
                        }
                    }
                    self.drag.win = wm.move_client(old_tag, self.drag.win, SetArg(wm.focused_tag(), false))?;
                } else {
                    tag.move_client(
                        &wm.aux,
                        self.drag.win,
                        (
                            poin.root_x - self.drag.prev.0,
                            poin.root_y - self.drag.prev.1,
                        ),
                        &pos,
                    )?
                }
            }
            3 => tag.resize_client(
                &wm.aux,
                self.drag.win,
                (
                    poin.root_x - self.drag.prev.0,
                    poin.root_y - self.drag.prev.1,
                ),
                self.drag.left,
                self.drag.top,
            )?,
            _ => (),
        }
        self.drag.prev = (poin.root_x, poin.root_y);
        Ok(())
    }
    fn handle_button_release(
        &mut self,
        wm: &mut WindowManager,
        e: ButtonReleaseEvent,
    ) -> Result<()> {
        info!("Handling Button Release");
        if e.detail == self.drag.button {
            self.drag.button = 0;
            ungrab_pointer(&wm.aux.dpy, CURRENT_TIME).context(crate::code_loc!())?;
        }
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct DragState {
    button: u8,
    win: usize,
    prev: (i16, i16),
    left: bool,
    top: bool,
}
