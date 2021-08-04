use x11rb::{
    connection::Connection,
    protocol::xproto::*,
    properties::*,
    x11_utils::Serialize,
    COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME, NONE
};
use std::collections::HashMap;
use log::info;
use crate::utils::{Rect, stack_::Stack};
use crate::{Connections, AtomCollection, CwmRes, Hooks};
use crate::config::Theme;
use super::{WindowManager, Monitor};

mod node;
mod client;
mod layer;
use node::{Node, Split, NodeContents};
use client::Client;
use layer::{Layer, StackLayer};

pub use client::ClientArgs;

pub struct Tag {
    id: usize,
    name: String,
    nodes: Vec<Node>,
    clients: Vec<Client>,
    free_nodes: Vec<usize>,
    free_clients: Vec<usize>,
    focus_stack: Stack<usize>,
    layers: [Layer; Layer::COUNT * Layer::SUBCOUNT],
    size: Rect,
    tiling_size: Rect,
    pub monitor: Option<usize>
}

impl Tag {
    pub fn new(id: usize, name: impl Into<String>) -> Self {
        Self {
            id,
            name: name.into(),
            nodes: vec![Node { absent: false, info: NodeContents::empty(), parent: None, rect: Rect::default()}],
            clients: Vec::new(),
            free_nodes: Vec::new(),
            free_clients: Vec::new(),
            focus_stack: Stack::default(),
            layers: [
                Layer::Multi(Stack::default()), Layer::Multi(Stack::default()), Layer::Single(None), 
                Layer::Multi(Stack::default()), Layer::Multi(Stack::default()), Layer::Single(None), 
                Layer::Multi(Stack::default()), Layer::Multi(Stack::default()), Layer::Single(None)
            ],
            size: Rect::default(),
            tiling_size: Rect::default(),
            monitor: None
        }
    }

    pub fn focused_client(&self) -> Option<usize> {
        self.focus_stack.front().copied()
    }

    pub fn client(&self, client: usize) -> &Client {
        &self.clients[client]
    }

    pub fn set_monitor(&mut self, conn: &Connections, monitor: &mut Monitor, atoms: &AtomCollection, hooks: &mut Hooks) -> CwmRes<()> {
        if monitor.focused_tag == self.id {
            return Ok(())
        }
        // resize the windows
        let available = monitor.free_rect();
        self.resize_all(conn, 0, &available, &monitor.size)?;
        for (id, client) in self.clients.iter().enumerate() {
            if !client.flags.hidden {
                self.show_client(conn, id, atoms)?;
            }
        }
        
        self.monitor.replace(monitor.id);
        self.size.copy(&monitor.size);
        self.tiling_size = available;
        monitor.focused_tag = self.id;
        self.set_active_window(self.focus_stack.front().map(|id| self.clients[*id].name.clone()).unwrap_or(None), hooks);
        //info!("tag {} monitor set to {}. size: {:?}, available: {:?}, root layout size: {:?}", self.id, monitor.id, self.monitor_size, self.available, self.layout.borrow().rect);
        Ok(())
    }

    pub fn hide(&mut self, conn: &Connections, atoms: &AtomCollection) -> CwmRes<()> {
        self.monitor.take();
        for (id, client) in self.clients.iter().enumerate() {
            if !client.flags.hidden {
                self.hide_client(conn, id, atoms)?;
            }
        }
        Ok(())
    }

    fn set_active_window(&self, name: Option<String>, hooks: &mut Hooks) {
        if let Some(monitor) = self.monitor {
            hooks.monitor_focus(monitor, name)
        }
    }
}


impl WindowManager {
    pub fn focused_tag(&self) -> usize {
        self.monitors[self.focused_monitor].focused_tag
    }
}