use x11rb::{
    connection::Connection,
    protocol::xproto::*,
    properties::*,
    x11_utils::Serialize,
    COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME, NONE
};
use std::collections::{HashSet, hash_map::Entry};
use log::info;
use crate::utils::{Rect, Stack};
use crate::{Aux, AtomCollection, Hooks};
use crate::config::Theme;
use super::{WindowManager, Monitor};
use anyhow::{Context, Result};

mod node;
mod client;
mod layer;
use node::{Node, Split, NodeContents};
use client::Client;
use layer::Layer;

pub use layer::StackLayer;
pub use client::ClientArgs;
pub use node::Side;

pub struct Tag {
    pub id: Atom,
    pub name: String,
    nodes: Vec<Node>,
    clients: Vec<Client>,
    free_nodes: Vec<usize>,
    free_clients: Vec<usize>,
    focus_stack: Stack<usize>,
    layers: [Layer; Layer::COUNT * Layer::SUBCOUNT],
    size: Rect,
    tiling_size: Rect,
    pub monitor: Option<Atom>,
    urgent: HashSet<usize>,
    temp: bool
}

impl Tag {
    pub fn empty(&self) -> bool {
        self.clients.len() == self.free_clients.len()
    }

    pub fn urgent(&self) -> bool {
        !self.urgent.is_empty()
    }

    pub fn focused_client(&self) -> Option<usize> {
        self.focus_stack.front().copied()
    }

    pub fn client(&self, client: usize) -> &Client {
        &self.clients[client]
    }

    pub fn set_monitor(&mut self, aux: &mut Aux, monitor: &mut Monitor) -> Result<()> {
        if monitor.focused_tag == self.id {
            return Ok(())
        }
        // resize the windows
        let available = monitor.free_rect();
        self.resize_all(aux, 0, &available, &monitor.size)?;
        for (id, client) in self.clients.iter().enumerate() {
            if !client.flags.hidden {
                self.show_client(aux, id)?;
            }
        }
        
        self.monitor.replace(monitor.id);
        self.size.copy(&monitor.size);
        self.tiling_size = available;
        monitor.prev_tag = monitor.focused_tag;
        monitor.focused_tag = self.id;
        self.set_active_window(self.focus_stack.front().map(|id| self.clients[*id].name.clone()).unwrap_or(None), &mut aux.hooks);
        //info!("tag {} monitor set to {}. size: {:?}, available: {:?}, root layout size: {:?}", self.id, monitor.id, self.monitor_size, self.available, self.layout.borrow().rect);
        Ok(())
    }

    pub fn hide(&mut self, aux: &Aux) -> Result<()> {
        self.monitor.take();
        for (id, client) in self.clients.iter().enumerate() {
            if !client.flags.hidden {
                self.hide_client(aux, id)?;
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

impl Default for Tag {
    fn default() -> Self {
        Tag {
            id: 0,
            name: String::new(),
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
            monitor: None,
            urgent: HashSet::new(),
            temp: false
        }
    }
}


impl WindowManager {
    pub fn focused_tag(&self) -> Atom {
        self.monitors.get(&self.focused_monitor).unwrap().focused_tag
    }

    // when the temp tag is removed, swap its id and name with the last temp tag so that they are always in order.
    pub fn temp_tag(&mut self) -> Atom {
        let id = self.temp_tags.len() as Atom;
        let name = String::new() + "temp" + &id.to_string();
        let tag = Tag {
            id,
            name,
            temp: true,
            ..Tag::default()
        };
        self.tags.insert(id, tag);
        id
    }

    pub fn add_tag(&mut self, name: impl Into<String>) -> Result<bool> {
        let name = name.into();
        let id = intern_atom(&self.aux.dpy, false, name.as_ref())?.reply()?.atom;
        let tag = Tag {
            id,
            name,
            ..Tag::default()
        };
        match self.tags.entry(id) {
            Entry::Occupied(..) => Ok(false),
            Entry::Vacant(mut entry) => {
                entry.insert(tag);
                self.free_tags.insert(id);
                self.tag_order.push(id);
                Ok(true)
            }
        }
    }
}