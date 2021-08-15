use anyhow::Result;
use log::info;
use std::collections::{hash_map::Entry, HashSet, VecDeque};
use x11rb::protocol::xproto::*;

use super::{Monitor, };
use crate::utils::{Rect, Stack};
use crate::{Aux, Hooks, WindowManager};
use crate::connections::{HiddenSelection, SetArg};


mod client;
mod layer;
mod node;
use client::Client;
use layer::Layer;
use node::{Node, Split};

pub use node::NodeContents;

pub use client::ClientArgs;
pub use layer::StackLayer;
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
    pub size: Rect,
    tiling_size: Rect,
    focused: Option<usize>,
    pub monitor: Option<Atom>,
    urgent: HashSet<usize>,
    hidden: VecDeque<usize>,
    temp: bool,
    bg: Option<Window>,
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

    pub fn client_mut(&mut self, client: usize) -> &mut Client {
        &mut self.clients[client]
    }

    pub fn node_mut(&mut self, node: usize) -> &mut Node {
        &mut self.nodes[node]
    }

    pub fn set_monitor(&mut self, aux: &mut Aux, monitor: &mut Monitor) -> Result<()> {
        if monitor.focused_tag == self.id {
            return Ok(());
        }
        // resize the windows
        let available = monitor.free_rect();
        info!("resizing, {:?}, {:?}", self.size, monitor.size);
        self.resize_all(aux, &available, &monitor.size)?;
        info!("showing windows");
        for client in self.clients.iter() {
            if !client.flags.hidden {
                client.show(aux)?;
            }
        }

        info!("done showing windows");
        self.monitor.replace(monitor.id);
        self.bg.replace(monitor.bg);
        self.size.copy(&monitor.size);
        monitor.prev_tag = monitor.focused_tag;
        monitor.focused_tag = self.id;
        self.set_active_window(
            self.focus_stack
                .front()
                .map(|id| self.clients[*id].name.clone())
                .unwrap_or(None),
            &mut aux.hooks,
        );
        //info!("tag {} monitor set to {}. size: {:?}, available: {:?}, root layout size: {:?}", self.id, monitor.id, self.monitor_size, self.available, self.layout.borrow().rect);
        Ok(())
    }

    pub fn show_clients(&mut self, aux: &mut Aux, selection: HiddenSelection) -> Result<()> {
        match selection {
            HiddenSelection::Last => if let Some(client) = self.hidden.pop_back() {
                self.set_hidden(aux, client, &SetArg(false, false))?
            },
            HiddenSelection::First => if let Some(client) = self.hidden.pop_front() {
                self.set_hidden(aux, client, &SetArg(false, false))?
            },
            HiddenSelection::All => for client in self.hidden.drain(..).collect::<Vec<_>>() {
                self.set_hidden(aux, client, &SetArg(false, false))?
            }
        }
        Ok(())
    }

    pub fn hide(&mut self, aux: &Aux) -> Result<()> {
        self.monitor.take();
        self.bg.take();
        for client in self.clients.iter_mut() {
            if !client.flags.sticky && !client.flags.hidden {
                client.hide(aux)?;
            }
        }
        self.unset_focus(aux)?;
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
            nodes: vec![Node {
                absent: false,
                info: NodeContents::empty(),
                parent: None,
                rect: Rect::new(0, 0, 1920, 1080),
            }],
            clients: Vec::new(),
            free_nodes: Vec::new(),
            free_clients: Vec::new(),
            focus_stack: Stack::default(),
            layers: [
                Layer::Multi(Stack::default()),
                Layer::Multi(Stack::default()),
                Layer::Single(None),
                Layer::Multi(Stack::default()),
                Layer::Multi(Stack::default()),
                Layer::Single(None),
                Layer::Multi(Stack::default()),
                Layer::Multi(Stack::default()),
                Layer::Single(None),
            ],
            size: Rect::new(0, 0, 1920, 1080),
            tiling_size: Rect::default(),
            focused: None,
            monitor: None,
            urgent: HashSet::new(),
            hidden: VecDeque::new(),
            temp: false,
            bg: None,
        }
    }
}

impl WindowManager {
    pub fn focused_tag(&self) -> Atom {
        self.monitors
            .get(&self.focused_monitor)
            .unwrap()
            .focused_tag
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
        let id = intern_atom(&self.aux.dpy, false, name.as_ref())?
            .reply()?
            .atom;
        let tag = Tag {
            id,
            name,
            ..Tag::default()
        };
        match self.tags.entry(id) {
            Entry::Occupied(..) => Ok(false),
            Entry::Vacant(entry) => {
                entry.insert(tag);
                self.free_tags.insert(id);
                self.tag_order.push(id);
                Ok(true)
            }
        }
    }
}
