use anyhow::Result;
use log::info;
use std::collections::{hash_map::Entry, HashSet, VecDeque};
use x11rb::protocol::xproto::*;

use super::Monitor;
use crate::connections::{HiddenSelection, SetArg};
use crate::utils::{pop_set_ord, Rect, Stack};
use crate::{Aux, Hooks, WindowManager};

mod client;
mod layer;
mod node;
use layer::Layer;
use node::Node;

pub use node::NodeContents;

pub use client::{Client, ClientArgs, ClientFlags};
pub use layer::StackLayer;
pub use node::{Side, Split};

pub struct Tag {
    pub id: Atom,
    pub name: String,
    nodes: Vec<Node>,
    clients: Vec<Client>,
    free_nodes: Vec<usize>,
    free_clients: HashSet<usize>,
    focus_stack: Stack<usize>,
    layers: [Layer; Layer::COUNT * Layer::SUBCOUNT],
    pub size: Rect,
    tiling_size: Rect,
    focused: Option<usize>,
    pub monitor: Option<Atom>,
    urgent: HashSet<usize>,
    psuedo_urgent: HashSet<usize>,
    hidden: VecDeque<usize>,
    monocle: bool,
    temp: bool,
    bg: Option<Window>,
}

impl Tag {
    pub fn empty(&self) -> bool {
        self.clients.len() == self.free_clients.len()
    }

    pub fn urgent(&self) -> bool {
        !(self.urgent.is_empty() && self.psuedo_urgent.is_empty())
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

    pub fn clients(&self) -> &[Client] {
        self.clients.as_ref()
    }

    pub fn clients_mut(&mut self) -> &mut [Client] {
        self.clients.as_mut()
    }

    pub fn node(&self, node: usize) -> &Node {
        &self.nodes[node]
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

    pub fn set_monocle(&mut self, aux: &Aux, arg: &SetArg<bool>) -> Result<()> {
        if arg.apply(&mut self.monocle) {
            self.resize_tiled(aux, 0, None)?;
        }
        Ok(())
    }

    pub fn show_clients(&mut self, aux: &mut Aux, selection: HiddenSelection) -> Result<()> {
        match selection {
            HiddenSelection::Last => {
                if let Some(client) = self.hidden.pop_back() {
                    self.set_hidden(aux, client, &SetArg(false, false))?
                }
            }
            HiddenSelection::First => {
                if let Some(client) = self.hidden.pop_front() {
                    self.set_hidden(aux, client, &SetArg(false, false))?
                }
            }
            HiddenSelection::All => {
                for client in self.hidden.drain(..).collect::<Vec<_>>() {
                    self.set_hidden(aux, client, &SetArg(false, false))?
                }
            }
        }
        Ok(())
    }

    pub fn hide(&mut self, aux: &mut Aux) -> Result<()> {
        self.monitor.take();
        self.bg.take();
        for client in self.clients.iter_mut() {
            if !client.flags.sticky && !client.flags.hidden {
                client.hide(aux, self.id)?;
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
            free_clients: HashSet::new(),
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
            psuedo_urgent: HashSet::new(),
            hidden: VecDeque::new(),
            temp: false,
            monocle: false,
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
    pub fn temp_tag(&mut self) -> Result<Atom> {
        let name = self
            .free_temp
            .pop()
            .unwrap_or_else(|| String::from("temp_") + &(self.temp_tags.len()).to_string());
        let id = intern_atom(&self.aux.dpy, false, name.as_ref())?
            .reply()?
            .atom;
        let tag = Tag {
            id,
            name,
            temp: true,
            ..Tag::default()
        };
        self.tags.insert(id, tag);
        self.tag_order.push(id);
        self.temp_tags.push(id);
        Ok(id)
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
            Entry::Occupied(..) => return Ok(false),
            Entry::Vacant(entry) => {
                entry.insert(tag);
                self.free_tags.insert(id);
                self.tag_order.push(id);
            }
        }
        if let Some(tag) = self.temp_tags.pop() {
            self.remove_tag(tag)?
        }
        self.aux
            .hooks
            .tag_update(&self.tags, &self.tag_order, self.focused_monitor);
        Ok(true)
    }

    pub fn remove_tag(&mut self, tag: Atom) -> Result<()> {
        if self.free_tags.is_empty() {
            return Ok(());
        }
        let mon = self.tags.get(&tag).unwrap().monitor;
        let new_tag = if let Some(mon_) = mon {
            let mon = self.monitors.get(&mon_).unwrap();
            let new_tag = if mon.prev_tag != mon.focused_tag {
                mon.prev_tag
            } else {
                pop_set_ord(&mut self.free_tags, &self.tag_order).unwrap()
            };
            self.switch_monitor_tag(mon_, SetArg(new_tag, false))?;
            self.monitors.get_mut(&mon_).unwrap().prev_tag = new_tag;
            new_tag
        } else {
            self.monitors
                .get(&self.focused_monitor)
                .unwrap()
                .focused_tag
        };
        for client in {
            let tag = self.tags.get(&tag).unwrap();
            (0..tag.clients().len())
                .filter(|i| !tag.free_clients.contains(i))
                .collect::<Vec<_>>()
        } {
            self.move_client(tag, client, SetArg(new_tag, false))?;
        }
        self.tag_order.retain(|id| id != &tag);
        let tag = self.tags.remove(&tag).unwrap();
        if tag.temp {
            self.temp_tags.retain(|id| id != &tag.id);
            self.free_temp.push(tag.name);
        }
        self.aux
            .hooks
            .tag_update(&self.tags, &self.tag_order, self.focused_monitor);
        Ok(())
    }
}
