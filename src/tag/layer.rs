use x11rb::protocol::xproto::*;
use crate::utils::Stack;
use crate::Aux;
use super::Tag;
use anyhow::{Context, Result};
use serde::{Serialize, Deserialize};
pub enum Layer {
    Single(Option<usize>),
    Multi(Stack<usize>)
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum StackLayer {
    Below,
    Normal,
    Above
}

impl Layer {
    pub const BELOW: usize = 0;
    pub const NORMAL: usize = 1;
    pub const ABOVE: usize = 2;
    pub const COUNT: usize = 3;
    pub const TILING: usize = 0;
    pub const FLOATING: usize = 1;
    pub const FULLSCREEN: usize = 2;
    pub const SUBCOUNT: usize = 3;

    fn front(&self) -> Option<usize> {
        match self {
            Layer::Single(layer) => *layer,
            Layer::Multi(layer) => layer.front().cloned()
        }
    }

    fn push_front(&mut self, client: usize) -> (usize, Option<usize>) {
        match self {
            Layer::Single(layer) => (0, layer.replace(client)),
            Layer::Multi(layer) => (layer.push_front(client), None)
        }
    }

    fn push_back(&mut self, client: usize) -> (usize, Option<usize>) {
        match self {
            Layer::Single(layer) => (0, layer.replace(client)),
            Layer::Multi(layer) => (layer.push_back(client), None)
        }
    }

    pub fn remove(&mut self, layer_pos: usize) {
        match self {
            Layer::Single(layer) => *layer = None,
            Layer::Multi(layer) => layer.remove_node(layer_pos)
        }
    }
}

impl StackLayer {
    fn get(&self) -> usize {
        match self {
            StackLayer::Below => 0,
            StackLayer::Normal => Layer::SUBCOUNT,
            StackLayer::Above => Layer::SUBCOUNT * 2
        }
    }
}   

impl Tag {
    pub fn switch_layer(&mut self, aux: &Aux, idx: usize) -> Result<()> {
        let client = &mut self.clients[idx];
        let (prev_layer, layer_pos) = client.layer_pos;
        self.layers[prev_layer].remove(layer_pos);
        match (prev_layer % Layer::COUNT == Layer::TILING, client.flags.get_layer() % Layer::COUNT == Layer::TILING) {
            (false, true) => self.set_absent(aux, idx, false)?,
            (true, false) => self.set_absent(aux, idx, true)?,
            _ => ()
        }

        self.set_layer(aux, idx, true)
    }

    pub fn set_layer(&mut self, aux: &Aux, idx: usize, focus: bool) -> Result<()> {
        let client = &mut self.clients[idx];
        let layer = client.layer.get() + client.flags.get_layer();
        let (layer_pos, old) = if focus {
            self.layers[layer].push_front(idx)
        } else {
            self.layers[layer].push_back(idx)
        };
        client.layer_pos = (layer, layer_pos);
        let client = &self.clients[idx];
        let (mut aux1, aux2) = self.get_rect(idx).unwrap().aux(if client.flags.fullscreen {0} else {client.border_width});
        aux1 = if let Some(sibling) = self.get_layer_bound(layer + if focus {1} else {0}) {
            aux1.sibling(self.clients[sibling].frame).stack_mode(StackMode::BELOW)
        } else {
            aux1.stack_mode(StackMode::ABOVE)
        };
        configure_window(&aux.dpy, client.frame, &aux1).context(crate::code_loc!())?;
        configure_window(&aux.dpy, client.win, &aux2).context(crate::code_loc!())?;
        if let Some(idx) = old {
            self.clients[idx].flags.fullscreen = false;
            if !self.clients[idx].flags.floating {
                self.set_absent(aux, idx, false)?
            }
            self.set_layer(aux, idx, true)?
        }
        Ok(())
    }

    fn get_layer_bound(&self, layer: usize) -> Option<usize> {
        if layer < Layer::SUBCOUNT * Layer::COUNT {
            for layer in layer..(Layer::SUBCOUNT * Layer::COUNT) {
                if let Some(window) = self.layers[layer].front() {
                    return Some(window)
                }
            }
            None
        } else {
            None
        }
    }
}