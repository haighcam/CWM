use anyhow::{Context, Result};
use log::info;
use serde::{Deserialize, Serialize};
use x11rb::protocol::xproto::*;

use super::Tag;
use crate::utils::Stack;
use crate::Aux;

pub enum Layer {
    Single(Option<usize>),
    Multi(Stack<usize>),
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum StackLayer {
    Below,
    Normal,
    Above,
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
            Layer::Multi(layer) => layer.front().cloned(),
        }
    }

    fn back(&self) -> Option<usize> {
        match self {
            Layer::Single(layer) => *layer,
            Layer::Multi(layer) => layer.back().cloned(),
        }
    }

    fn push_front(&mut self, client: usize) -> (usize, Option<usize>) {
        match self {
            Layer::Single(layer) => (0, layer.replace(client)),
            Layer::Multi(layer) => (layer.push_front(client), None),
        }
    }

    fn push_back(&mut self, client: usize) -> (usize, Option<usize>) {
        match self {
            Layer::Single(layer) => (0, layer.replace(client)),
            Layer::Multi(layer) => (layer.push_back(client), None),
        }
    }

    pub fn remove(&mut self, layer_pos: usize) {
        match self {
            Layer::Single(layer) => *layer = None,
            Layer::Multi(layer) => layer.remove_node(layer_pos),
        }
    }
}

impl StackLayer {
    fn get(&self) -> usize {
        match self {
            StackLayer::Below => 0,
            StackLayer::Normal => Layer::SUBCOUNT,
            StackLayer::Above => Layer::SUBCOUNT * 2,
        }
    }
}

impl Tag {
    pub fn switch_layer(&mut self, aux: &Aux, idx: usize) -> Result<()> {
        let client = &mut self.clients[idx];
        let (prev_layer, layer_pos) = client.layer_pos;
        self.layers[prev_layer].remove(layer_pos);
        match (
            prev_layer % Layer::COUNT == Layer::TILING,
            client.flags.get_layer() % Layer::COUNT == Layer::TILING,
        ) {
            (false, true) => self.set_absent(aux, idx, false)?,
            (true, false) => self.set_absent(aux, idx, true)?,
            _ => (),
        }

        self.set_layer(aux, idx, true)
    }

    pub fn set_layer(&mut self, aux: &Aux, idx: usize, focus: bool) -> Result<()> {
        let client = &self.clients[idx];
        let layer = client.layer.get() + client.flags.get_layer();
        let mut conf_aux = self.get_rect(idx).unwrap().aux(if client.flags.fullscreen {
            0
        } else {
            client.border_width
        });

        if let Some(sibling) = self.get_layer_bound_below(layer + if focus { 1 } else { 0 }) {
            conf_aux = conf_aux.sibling(sibling).stack_mode(StackMode::BELOW);
        } else if let Some(sibling) = self.get_layer_bound_above(layer + if focus { 1 } else { 0 })
        {
            conf_aux = conf_aux.sibling(sibling).stack_mode(StackMode::ABOVE);
        }
        configure_window(&aux.dpy, client.frame, &conf_aux).context(crate::code_loc!())?;
        configure_window(
            &aux.dpy,
            client.win,
            &conf_aux
                .x(None)
                .y(None)
                .border_width(None)
                .stack_mode(None)
                .sibling(None),
        )
        .context(crate::code_loc!())?;
        let client = &mut self.clients[idx];
        let (layer_pos, old) = if focus {
            self.layers[layer].push_front(idx)
        } else {
            self.layers[layer].push_back(idx)
        };
        client.layer_pos = (layer, layer_pos);
        if let Some(idx) = old {
            self.clients[idx].flags.fullscreen = false;
            if !self.clients[idx].flags.floating {
                self.set_absent(aux, idx, false)?
            }
            self.set_layer(aux, idx, true)?
        }
        Ok(())
    }

    fn get_layer_bound_below(&self, layer: usize) -> Option<u32> {
        if layer > Layer::SUBCOUNT * Layer::COUNT {
            None
        } else {
            self.layers[layer..]
                .iter()
                .filter_map(|x| x.back())
                .map(|x| self.clients[x].frame)
                .next()
        }
    }

    fn get_layer_bound_above(&self, layer: usize) -> Option<u32> {
        if layer > Layer::SUBCOUNT * Layer::COUNT {
            None
        } else {
            self.layers[..layer]
                .iter()
                .rev()
                .filter_map(|x| x.front())
                .map(|x| self.clients[x].frame)
                .next()
        }
    }
}
