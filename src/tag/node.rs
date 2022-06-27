use anyhow::Result;
use log::info;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use super::{Client, Tag};
use crate::utils::{pop_set, three_mut, Rect};
use crate::Aux;

#[derive(PartialEq, Serialize, Deserialize, Debug, Copy, Clone)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

impl Side {
    pub const MIN: f32 = 0.1;
    pub const MAX: f32 = 1.0 - Self::MIN;

    pub fn get_split(&self) -> (Split, bool) {
        use Side::*;
        match self {
            Left => (Split::Vertical, true),
            Right => (Split::Vertical, false),
            Top => (Split::Horizontal, true),
            Bottom => (Split::Horizontal, false),
        }
    }

    pub fn parse_amt(&self, amt: i16) -> (i16, i16) {
        use Side::*;
        match self {
            Left => (-amt, 0),
            Right => (amt, 0),
            Top => (0, -amt),
            Bottom => (0, amt),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Split {
    Horizontal,
    Vertical,
}

#[derive(Clone, Debug)]
pub struct NodeInfo {
    pub split: Split,
    pub ratio: f32,
    pub first_child: usize,
    pub second_child: usize,
}

impl NodeInfo {
    fn get_child(&self, first: bool) -> usize {
        if first {
            self.first_child
        } else {
            self.second_child
        }
    }
}

#[derive(Clone, Debug)]
pub struct LeafInfo {
    pub floating: Rect,
    pub min_size: (u16, u16),
    pub max_size: (u16, u16),
    client: usize,
}

impl LeafInfo {
    pub fn new(client: usize, min_size: (u16, u16), max_size: (u16, u16), floating: Rect) -> Self {
        Self {
            client,
            min_size,
            max_size,
            floating,
        }
    }
}

#[derive(Clone, Debug)]
pub enum NodeContents {
    Node(NodeInfo),
    Leaf(LeafInfo),
    Empty,
}

impl NodeContents {
    pub fn empty() -> Self {
        Self::Empty
    }

    pub fn leaf(client: usize, min_size: (u16, u16), max_size: (u16, u16), floating: Rect) -> Self {
        Self::Leaf(LeafInfo {
            client,
            min_size,
            max_size,
            floating,
        })
    }

    pub fn node(split: Split, ratio: f32, first_child: usize, second_child: usize) -> Self {
        Self::Node(NodeInfo {
            split,
            ratio,
            first_child,
            second_child,
        })
    }
}

#[derive(Debug)]
pub struct Node {
    pub parent: Option<(usize, bool)>,
    pub absent: bool,
    pub rect: Rect,
    pub info: NodeContents,
}

impl Tag {
    pub fn get_node_rect(&self, node: usize) -> &Rect {
        &self.nodes[node].rect
    }

    pub fn get_node_client(&self, node: usize) -> Option<usize> {
        if let NodeContents::Leaf(leaf) = &self.nodes[node].info {
            Some(leaf.client)
        } else {
            None
        }
    }

    pub fn get_rect(&self, client: usize) -> Option<Rect> {
        let client = &self.clients[client];
        if client.flags.fullscreen {
            Some(self.size.clone())
        } else {
            let node = &self.nodes[client.node];
            if !client.flags.floating {
                Some(node.rect.clone())
            } else if let NodeContents::Leaf(leaf) = &node.info {
                Some(leaf.floating.clone())
            } else {
                None
            }
        }
    }

    fn resize_node(
        &mut self,
        aux: &Aux,
        node: usize,
        to_process: &mut Vec<usize>,
        force_process: bool,
    ) {
        if let Some((_child1, _child2)) = if let NodeContents::Node(node) = &self.nodes[node].info {
            Some((node.first_child, node.second_child))
        } else {
            None
        } {
            info!("{} {} {}", node, _child1, _child2);
            let (node, child1, child2) =
                three_mut(&mut self.nodes, (node, _child1, _child2)).unwrap();
            if let NodeContents::Node(info) = &node.info {
                if self.monocle {
                    child1.rect.copy(&self.tiling_size);
                    child2.rect.copy(&self.tiling_size);
                    to_process.push(_child2);
                    to_process.push(_child1);
                } else {
                    match (child1.absent, child2.absent) {
                        (true, false) => {
                            child2.rect.copy(&node.rect);
                            to_process.push(_child2);
                            if force_process {
                                to_process.push(_child1);
                            }
                        }
                        (false, true) => {
                            child1.rect.copy(&node.rect);
                            to_process.push(_child1);
                            if force_process {
                                to_process.push(_child2);
                            }
                        }
                        (false, false) => {
                            node.rect.split(
                                &info.split,
                                info.ratio,
                                &mut child1.rect,
                                &mut child2.rect,
                                aux.theme.gap,
                            );
                            to_process.push(_child2);
                            to_process.push(_child1);
                        }
                        _ => (),
                    }
                }
            }
        }
    }

    fn add_node(&mut self, node: Node) -> usize {
        if let Some(idx) = self.free_nodes.pop() {
            self.nodes[idx] = node;
            idx
        } else {
            self.nodes.push(node);
            self.nodes.len() - 1
        }
    }

    pub fn split_leaf(
        &mut self,
        aux: &mut Aux,
        leaf_idx: usize,
        absent: bool,
        idx: usize,
        info: NodeContents,
    ) -> Result<()> {
        let ((split, first), ratio) = aux
            .selection
            .presel(&aux.dpy, self.id, leaf_idx)?
            .map_or_else(
                || {
                    let rect = self.nodes[leaf_idx].rect.clone();
                    (
                        (
                            if rect.width > rect.height {
                                Split::Vertical
                            } else {
                                Split::Horizontal
                            },
                            false,
                        ),
                        0.5,
                    )
                },
                |presel| (presel.side.get_split(), presel.amt),
            );
        let (node1, node2, leaf_absent) = {
            let leaf = &self.nodes[leaf_idx];
            (
                Node {
                    parent: Some((leaf_idx, !first)),
                    rect: self.tiling_size.clone(),
                    absent: leaf.absent,
                    info: leaf.info.clone(),
                },
                Node {
                    parent: Some((leaf_idx, first)),
                    rect: self.tiling_size.clone(),
                    absent,
                    info,
                },
                leaf.absent,
            )
        };
        let first_child = self.add_node(node1);
        let second_child = self.add_node(node2);
        let node = &mut self.nodes[leaf_idx];
        let mut idx2 = None;
        if let NodeContents::Leaf(leaf) = &node.info {
            self.clients[leaf.client].node = first_child;
            idx2 = Some(leaf.client);
        }
        node.info = NodeContents::Node(NodeInfo {
            split,
            ratio,
            first_child: if first { second_child } else { first_child },
            second_child: if first { first_child } else { second_child },
        });
        self.clients[idx].node = second_child;
        // recompute child sizes of node
        if leaf_absent && !absent {
            self.propagate_absent(aux, leaf_idx)?;
        } else if !(self.monocle || (leaf_absent && absent)) {
            self.resize_node(aux, leaf_idx, &mut vec![], false);
        }
        if !leaf_absent && idx2.is_some() {
            self.apply_pos_size(aux, idx2.unwrap(), &self.nodes[first_child].rect, true)?;
        }
        Ok(())
    }

    fn propagate_absent(&mut self, aux: &Aux, node: usize) -> Result<()> {
        let mut parent = Some(node);
        let mut prev_parent = node;
        while parent.is_some() {
            prev_parent = parent.unwrap();
            parent = {
                if let Some(absent) = {
                    if let NodeContents::Node(node) = &self.nodes[prev_parent].info {
                        Some(
                            self.nodes[node.first_child].absent
                                && self.nodes[node.second_child].absent,
                        )
                    } else {
                        None
                    }
                } {
                    let node = &mut self.nodes[prev_parent];
                    if node.absent != absent {
                        node.absent = absent;
                        node.parent.map(|x| x.0)
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        }
        let mut q = vec![prev_parent];
        while !q.is_empty() {
            let node_ = q.pop().unwrap();
            let node = &self.nodes[node_];
            match &node.info {
                NodeContents::Node(_) => self.resize_node(aux, node_, &mut q, false),
                NodeContents::Leaf(leaf) => {
                    if !node.absent {
                        self.apply_pos_size(aux, leaf.client, &node.rect, true)?
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }

    fn get_split_parent(&self, node: usize, split_dir: Side) -> (Option<(usize, bool)>, usize) {
        let mut _parent = self.nodes[node].parent;
        let node_rect = &self.nodes[node].rect;
        let mut i = 0;
        while _parent.is_some() {
            _parent = {
                let parent = &self.nodes[_parent.unwrap().0];
                match (parent, &split_dir) {
                    (
                        Node {
                            info:
                                NodeContents::Node(NodeInfo {
                                    split: Split::Vertical,
                                    ..
                                }),
                            rect,
                            ..
                        },
                        Side::Left,
                    ) if rect.x < node_rect.x => break,
                    (
                        Node {
                            info:
                                NodeContents::Node(NodeInfo {
                                    split: Split::Vertical,
                                    ..
                                }),
                            rect,
                            ..
                        },
                        Side::Right,
                    ) if (rect.x + rect.width as i16) > (node_rect.x + node_rect.width as i16) => {
                        break
                    }
                    (
                        Node {
                            info:
                                NodeContents::Node(NodeInfo {
                                    split: Split::Horizontal,
                                    ..
                                }),
                            rect,
                            ..
                        },
                        Side::Top,
                    ) if rect.y < node_rect.y => break,
                    (
                        Node {
                            info:
                                NodeContents::Node(NodeInfo {
                                    split: Split::Horizontal,
                                    ..
                                }),
                            rect,
                            ..
                        },
                        Side::Bottom,
                    ) if (rect.y + rect.height as i16)
                        > (node_rect.y + node_rect.height as i16) =>
                    {
                        break
                    }
                    _ => (),
                }
                parent.parent
            };
            i += 1;
        }
        (_parent, i)
    }

    fn rotate_nodes(&mut self, _node: usize, _child1: usize, _child2: usize, rev: bool) {
        let (node, child1, child2) = three_mut(&mut self.nodes, (_node, _child1, _child2)).unwrap();
        if let NodeContents::Node(info) = &mut node.info {
            match &info.split {
                Split::Horizontal => {
                    if !rev {
                        child1.parent = Some((_node, false));
                        child2.parent = Some((_node, true));
                        info.first_child = _child2;
                        info.second_child = _child1;
                        info.ratio = 1.0 - info.ratio;
                    }
                    info.split = Split::Vertical;
                }
                Split::Vertical => {
                    if rev {
                        child1.parent = Some((_node, false));
                        child2.parent = Some((_node, true));
                        info.first_child = _child2;
                        info.second_child = _child1;
                        info.ratio = 1.0 - info.ratio;
                    }
                    info.split = Split::Horizontal;
                }
            }
        }
    }

    pub fn rotate(&mut self, aux: &Aux, node: usize, rev: bool) -> Result<()> {
        let mut q = vec![node];
        while !q.is_empty() {
            let node = q.pop().unwrap();
            if let NodeContents::Node(info) = &self.nodes[node].info {
                let (first_child, second_child) = (info.first_child, info.second_child);
                q.push(first_child);
                q.push(second_child);
                self.rotate_nodes(node, first_child, second_child, rev);
            }
        }
        self.resize_tiled(aux, node, None)?;
        Ok(())
    }

    pub fn client_under_cursor(&self, root: usize, pos: &(i16, i16)) -> Option<usize> {
        #[inline]
        fn check_node(tag: &Tag, node_: usize, q: &mut Vec<usize>, pos: &(i16, i16)) {
            let node = &tag.nodes[node_];
            if node.rect.contains(pos) && !node.absent {
                q.push(node_)
            }
        }
        let mut q = vec![];
        check_node(self, root, &mut q, pos);
        while !q.is_empty() {
            let node = &self.nodes[q.pop().unwrap()];
            match &node.info {
                NodeContents::Leaf(leaf) => return Some(leaf.client),
                NodeContents::Node(node) => {
                    check_node(self, node.first_child, &mut q, pos);
                    check_node(self, node.second_child, &mut q, pos);
                }
                _ => (),
            }
        }
        None
    }

    pub fn move_client(
        &mut self,
        aux: &Aux,
        client_: usize,
        delta: (i16, i16),
        pos: &(i16, i16),
    ) -> Result<()> {
        let client = &self.clients[client_];
        if !client.flags.fullscreen {
            if client.flags.floating {
                if let NodeContents::Leaf(leaf) = &mut self.nodes[client.node].info {
                    leaf.floating.x += delta.0;
                    leaf.floating.y += delta.1;
                }
                if let NodeContents::Leaf(leaf) = &self.nodes[client.node].info {
                    self.apply_pos_size(aux, client_, &leaf.floating, true)?;
                }
            } else if !self.nodes[client.node].rect.contains(pos) {
                if let Some(other) = self.client_under_cursor(0, pos) {
                    let other_node = self.clients[other].node;
                    let node = client.node;
                    self.clients[other].node = node;
                    self.clients[client_].node = other_node;
                    let info = self.nodes[node].info.clone();
                    self.nodes[node].info = self.nodes[other_node].info.clone();
                    self.nodes[other_node].info = info;
                    self.apply_pos_size(aux, client_, &self.nodes[other_node].rect, true)?;
                    self.apply_pos_size(aux, other, &self.nodes[node].rect, true)?;
                }
            }
        }
        Ok(())
    }

    pub fn move_side(&mut self, aux: &Aux, client_: usize, side: Side, amount: u16) -> Result<()> {
        info!("moving {:?}", side);
        let client = &self.clients[client_];
        if !client.flags.fullscreen {
            if client.flags.floating {
                if let NodeContents::Leaf(leaf) = &mut self.nodes[client.node].info {
                    let delta = side.parse_amt(amount as i16);
                    leaf.floating.x += delta.0;
                    leaf.floating.y += delta.1;
                }
                if let NodeContents::Leaf(leaf) = &self.nodes[client.node].info {
                    self.apply_pos_size(aux, client_, &leaf.floating, true)?;
                }
            } else if let Some(other) = self.get_neighbour(client_, side) {
                let other_node = self.clients[other].node;
                let node = client.node;
                self.clients[other].node = node;
                self.clients[client_].node = other_node;
                let info = self.nodes[node].info.clone();
                self.nodes[node].info = self.nodes[other_node].info.clone();
                self.nodes[other_node].info = info;
                self.apply_pos_size(aux, client_, &self.nodes[other_node].rect, true)?;
                self.apply_pos_size(aux, other, &self.nodes[node].rect, true)?;
            }
        }
        Ok(())
    }

    pub fn get_neighbour(&self, client: usize, side: Side) -> Option<usize> {
        let node = self.clients[client].node;
        let parent = self.get_split_parent(node, side).0;
        if let Some((parent, first)) = parent {
            if let NodeContents::Node(node) = &self.nodes[parent].info {
                let mut siblings = HashSet::new();
                let mut q = vec![node.get_child(!first)];
                let (split, first) = side.get_split();
                while !q.is_empty() {
                    let item = &self.nodes[q.pop().unwrap()];
                    if !item.absent {
                        match &item.info {
                            NodeContents::Leaf(leaf) => {
                                siblings.insert(leaf.client);
                            }
                            NodeContents::Node(node) => {
                                if node.split != split {
                                    q.push(node.get_child(!first))
                                } else {
                                    q.push(node.first_child);
                                    q.push(node.second_child);
                                }
                            }
                            _ => (),
                        }
                    }
                }
                match siblings.len() {
                    0 => (),
                    1 => return siblings.into_iter().next(),
                    _ => {
                        return self
                            .focus_stack
                            .iter()
                            .find(|x| siblings.contains(x))
                            .copied()
                    }
                }
            }
        }
        None
    }

    pub fn resize_client(
        &mut self,
        aux: &mut Aux,
        client: usize,
        delta: (i16, i16),
        left: bool,
        top: bool,
    ) -> Result<()> {
        let (fullscreen, floating, node) = {
            let client = &self.clients[client];
            (client.flags.fullscreen, client.flags.floating, client.node)
        };
        if !fullscreen {
            if floating {
                if let NodeContents::Leaf(leaf) = &mut self.nodes[node].info {
                    if left {
                        let width = leaf
                            .floating
                            .width
                            .overflowing_sub(delta.0 as u16)
                            .0
                            .min(leaf.max_size.0)
                            .max(leaf.min_size.0);
                        leaf.floating.x -= width as i16 - leaf.floating.width as i16;
                        leaf.floating.width = width;
                    } else {
                        leaf.floating.width = leaf
                            .floating
                            .width
                            .overflowing_add(delta.0 as u16)
                            .0
                            .min(leaf.max_size.0)
                            .max(leaf.min_size.0);
                    }
                    if top {
                        let height = leaf
                            .floating
                            .height
                            .overflowing_sub(delta.1 as u16)
                            .0
                            .min(leaf.max_size.1)
                            .max(leaf.min_size.1);
                        leaf.floating.y -= height as i16 - leaf.floating.height as i16;
                        leaf.floating.height = height;
                    } else {
                        leaf.floating.height = leaf
                            .floating
                            .height
                            .overflowing_add(delta.1 as u16)
                            .0
                            .min(leaf.max_size.1)
                            .max(leaf.min_size.1);
                    }
                }
                if let NodeContents::Leaf(leaf) = &self.nodes[node].info {
                    self.apply_pos_size(aux, client, &leaf.floating, true)?;
                }
            } else {
                let (parent_h, depth1) =
                    self.get_split_parent(node, if left { Side::Left } else { Side::Right });
                let (parent_v, depth2) =
                    self.get_split_parent(node, if top { Side::Top } else { Side::Bottom });
                let mut q = vec![];
                if let Some((parent_, _)) = parent_h {
                    let parent = &mut self.nodes[parent_];
                    if let NodeContents::Node(node) = &mut parent.info {
                        let diff = delta.0 as f32 / parent.rect.width as f32;
                        node.ratio = (node.ratio + diff).min(Side::MAX).max(Side::MIN);
                    }
                    if parent_v.is_none() || depth1 > depth2 {
                        q.push(parent_);
                    }
                }
                if let Some((parent_, _)) = parent_v {
                    let parent = &mut self.nodes[parent_];
                    if let NodeContents::Node(node) = &mut parent.info {
                        let diff = delta.1 as f32 / parent.rect.height as f32;
                        node.ratio = (node.ratio + diff).min(Side::MAX).max(Side::MIN);
                    }
                    if q.is_empty() {
                        q.push(parent_)
                    }
                }
                while !q.is_empty() {
                    let node_ = q.pop().unwrap();
                    let node = &self.nodes[node_];
                    match &node.info {
                        NodeContents::Node(_) => self.resize_node(aux, node_, &mut q, false),
                        NodeContents::Leaf(leaf) => {
                            if !node.absent {
                                self.apply_pos_size(aux, leaf.client, &node.rect, true)?
                            }
                        }
                        _ => (),
                    }
                }
                aux.resize_selection(self)?;
            }
        }
        Ok(())
    }

    pub fn set_absent(&mut self, aux: &Aux, client: usize, absent: bool) -> Result<()> {
        if let Some(parent) = {
            let node = &mut self.nodes[self.clients[client].node];
            if node.absent != absent {
                node.absent = absent;
                node.parent.map(|x| x.0)
            } else {
                None
            }
        } {
            self.propagate_absent(aux, parent)?;
        }
        if !absent {
            self.resize_tiled(aux, self.clients[client].node, None)?;
        }
        Ok(())
    }

    pub fn set_tiling_size(&mut self, aux: &Aux, mut tiling_size: Rect) -> Result<()> {
        tiling_size.x += aux.theme.gap as i16 + aux.theme.left_margin;
        tiling_size.y += aux.theme.gap as i16 + aux.theme.top_margin;
        tiling_size.width -=
            (aux.theme.gap as i16 * 2 + aux.theme.right_margin + aux.theme.left_margin) as u16;
        tiling_size.height -=
            (aux.theme.gap as i16 * 2 + aux.theme.bottom_margin + aux.theme.top_margin) as u16;
        if tiling_size != self.tiling_size {
            self.tiling_size.copy(&tiling_size);
            self.resize_tiled(aux, 0, Some(&tiling_size))?;
        }
        Ok(())
    }

    pub fn resize_tiled(&mut self, aux: &Aux, node: usize, size: Option<&Rect>) -> Result<()> {
        if let Some(size) = size {
            self.nodes[node].rect.copy(size);
        }
        let mut q = vec![0];
        while !q.is_empty() {
            let node_ = q.pop().unwrap();
            let node = &self.nodes[node_];
            match &node.info {
                NodeContents::Node(..) => self.resize_node(aux, node_, &mut q, false),
                NodeContents::Leaf(leaf) => {
                    if !node.absent {
                        self.apply_pos_size(aux, leaf.client, &node.rect, true)?
                    }
                }
                _ => (),
            }
        }
        Ok(())
    }

    pub fn resize_all(&mut self, aux: &Aux, available: &Rect, new_size: &Rect) -> Result<()> {
        let mut tiling_size = &mut self.nodes[0].rect;
        tiling_size.x = available.x + aux.theme.gap as i16 + aux.theme.left_margin;
        tiling_size.y = available.y + aux.theme.gap as i16 + aux.theme.top_margin;
        tiling_size.width = available.width
            - (aux.theme.gap as i16 * 2 + aux.theme.right_margin + aux.theme.left_margin) as u16;
        tiling_size.height = available.height
            - (aux.theme.gap as i16 * 2 + aux.theme.bottom_margin + aux.theme.top_margin) as u16;
        if *tiling_size != self.tiling_size {
            self.tiling_size.copy(tiling_size)
        }
        let mut q = vec![0];
        while !q.is_empty() {
            let node_ = q.pop().unwrap();
            let node = &self.nodes[node_];
            match &node.info {
                NodeContents::Node(..) => self.resize_node(aux, node_, &mut q, true),
                NodeContents::Leaf(leaf) => {
                    let leaf_client = leaf.client;
                    let floating = {
                        if let NodeContents::Leaf(leaf) = &mut self.nodes[node_].info {
                            leaf.floating.reposition(&self.size, new_size);
                            leaf.floating.clone()
                        } else {
                            Rect::default()
                        }
                    };
                    let node = &self.nodes[node_];
                    let client = &self.clients[leaf_client];
                    let (rect, border) = if client.flags.fullscreen {
                        (new_size, false)
                    } else if client.flags.floating {
                        (&floating, true)
                    } else {
                        (&node.rect, true)
                    };
                    self.apply_pos_size(aux, leaf_client, rect, border)?
                }
                _ => (),
            }
        }
        Ok(())
    }

    pub fn add_client(
        &mut self,
        aux: &mut Aux,
        client: Client,
        parent: Option<usize>,
        mut info: NodeContents,
        focus: bool,
    ) -> Result<usize> {
        let absent = client.flags.absent();
        let hidden = client.flags.hidden;
        let client = if let Some(idx) = pop_set(&mut self.free_clients) {
            self.clients[idx] = client;
            idx
        } else {
            self.clients.push(client);
            self.clients.len() - 1
        };

        if let NodeContents::Leaf(leaf) = &mut info {
            leaf.client = client;
        }

        match self.nodes[0].info {
            NodeContents::Empty => {
                self.nodes[0].info = info;
                self.nodes[0].absent = absent;
                self.clients[client].node = 0;
            }
            NodeContents::Leaf(..) => {
                self.split_leaf(aux, 0, absent, client, info)?;
            }
            NodeContents::Node(..) => {
                let leaf = parent
                    .or_else(|| self.focus_stack.front().cloned())
                    .unwrap_or_else(|| *self.hidden.back().unwrap());
                let leaf = self.clients[leaf].node;
                self.split_leaf(aux, leaf, absent, client, info)?;
            }
        }
        if !hidden {
            self.clients[client].stack_pos = if focus {
                self.focus_stack.push_front(client)
            } else {
                self.focus_stack.push_back(client)
            };
        }
        Ok(client)
    }

    pub fn remove_node(&mut self, aux: &Aux, node: usize) -> Result<()> {
        let parent = self.nodes[node].parent;
        self.nodes[node].info = NodeContents::Empty;
        self.free_nodes.push(node);
        info!("removing node {}", node);
        if let Some((parent_, first)) = parent {
            {
                let info = match &self.nodes[parent_].info {
                    NodeContents::Node(node) => {
                        let child = node.get_child(!first);
                        info!("removing node {}", child);
                        self.free_nodes.push(child);
                        let child = &self.nodes[child];
                        Some((child.info.clone(), child.absent))
                    }
                    _ => None,
                };
                self.nodes[*self.free_nodes.last().unwrap()].info = NodeContents::Empty;
                let parent = &mut self.nodes[parent_];
                if let Some((info, absent)) = info {
                    parent.info = info;
                    parent.absent = absent;
                } else {
                    parent.info = NodeContents::Empty;
                }
                if let NodeContents::Leaf(leaf) = &parent.info {
                    self.clients[leaf.client].node = parent_;
                } else if let NodeContents::Node(NodeInfo {
                    first_child,
                    second_child,
                    ..
                }) = &parent.info
                {
                    let first_child = *first_child;
                    let second_child = *second_child;
                    self.nodes[first_child].parent = Some((parent_, true));
                    self.nodes[second_child].parent = Some((parent_, false));
                }
            }
            self.resize_tiled(aux, parent_, None)?;
            self.propagate_absent(aux, parent_)?;
        }
        Ok(())
    }

    pub fn print_node(&self, node: usize, depth: usize) {
        let offset = std::iter::repeat(" ")
            .take(depth)
            .fold(String::new(), |x, y| x + y);
        match &self.nodes[node].info {
            NodeContents::Node(node) => {
                println!("{}node:", offset);
                self.print_node(node.first_child, depth + 1);
                self.print_node(node.second_child, depth + 1);
            }
            NodeContents::Leaf(_) => {
                println!("{}leaf", offset);
            }
            NodeContents::Empty => {
                println!("{}empty", offset);
            }
        }
    }
}
