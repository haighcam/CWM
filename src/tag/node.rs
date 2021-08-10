use super::Tag;
use crate::utils::{three_mut, Rect};
use crate::Aux;
use anyhow::Result;
use log::info;
use serde::{Deserialize, Serialize};

#[derive(PartialEq, Serialize, Deserialize, Debug)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
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
    floating: Rect,
    min_size: (u16, u16),
    max_size: (u16, u16),
    client: usize,
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

impl Split {
    pub const MIN: f32 = 0.1;
    pub const MAX: f32 = 1.0 - Self::MIN;
}

impl Tag {
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

    fn resize_node(&mut self, aux: &Aux, node: usize, to_process: &mut Vec<usize>) {
        if let Some((_child1, _child2)) = if let NodeContents::Node(node) = &self.nodes[node].info {
            Some((node.first_child, node.second_child))
        } else {
            None
        } {
            info!("{} {} {}", node, _child1, _child2);
            let (node, child1, child2) =
                three_mut(&mut self.nodes, (node, _child1, _child2)).unwrap();
            if let NodeContents::Node(info) = &node.info {
                match (child1.absent, child2.absent) {
                    (true, false) => {
                        child2.rect.copy(&node.rect);
                        to_process.push(_child2);
                    }
                    (false, true) => {
                        child1.rect.copy(&node.rect);
                        to_process.push(_child1);
                    }
                    (false, false) => {
                        node.rect.split(
                            info.ratio,
                            info.split == Split::Vertical,
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
        aux: &Aux,
        leaf_idx: usize,
        split: Option<Split>,
        absent: bool,
        idx: usize,
        info: NodeContents,
    ) -> Result<()> {
        info!("{:?}", self.nodes);
        let rect = self.nodes[leaf_idx].rect.clone();
        let split = split.unwrap_or_else(|| {
            if rect.width > rect.height {
                Split::Vertical
            } else {
                Split::Horizontal
            }
        });

        let (node1, node2, leaf_absent) = {
            let leaf = &self.nodes[leaf_idx];
            (
                Node {
                    parent: Some((leaf_idx, true)),
                    rect: Rect::default(),
                    absent: leaf.absent,
                    info: leaf.info.clone(),
                },
                Node {
                    parent: Some((leaf_idx, false)),
                    rect: Rect::default(),
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
            ratio: 0.5,
            first_child,
            second_child,
        });
        self.clients[idx].node = second_child;
        // recompute child sizes of node
        if leaf_absent && !absent {
            self.propagate_absent(aux, leaf_idx)?;
        } else if !(leaf_absent && absent) {
            self.resize_node(aux, leaf_idx, &mut vec![]);
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
                NodeContents::Node(_) => self.resize_node(aux, node_, &mut q),
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

    fn get_split_parent(&self, node: usize, split_dir: Side) -> (Option<usize>, usize) {
        let mut _parent = self.nodes[node].parent.map(|x| x.0);
        let node_rect = &self.nodes[node].rect;
        let mut i = 0;
        while _parent.is_some() {
            _parent = {
                let parent = &self.nodes[_parent.unwrap()];
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
                parent.parent.map(|x| x.0)
            };
            i += 1;
        }
        (_parent, i)
    }

    fn client_under_cursor(&self, pos: &(i16, i16)) -> Option<usize> {
        #[inline]
        fn check_node(tag: &Tag, node_: usize, q: &mut Vec<usize>, pos: &(i16, i16)) {
            let node = &tag.nodes[node_];
            if node.rect.contains(pos) && !node.absent {
                q.push(node_)
            }
        }
        let mut q = vec![];
        check_node(self, 0, &mut q, pos);
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
                if let Some(other) = self.client_under_cursor(pos) {
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

    pub fn resize_client(
        &mut self,
        aux: &Aux,
        client: usize,
        delta: (i16, i16),
        left: bool,
        top: bool,
    ) -> Result<()> {
        let (fullscreen, floating, node) = {
            let client = &self.clients[client];
            (
                client.flags.fullscreen,
                client.flags.fullscreen,
                client.node,
            )
        };
        if !fullscreen {
            if floating {
                if let NodeContents::Leaf(leaf) = &mut self.nodes[node].info {
                    if left {
                        leaf.floating.x += delta.0;
                        leaf.floating.width = leaf
                            .floating
                            .width
                            .overflowing_sub(delta.0 as u16)
                            .0
                            .min(leaf.max_size.0)
                            .max(leaf.min_size.0);
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
                        leaf.floating.y += delta.1;
                        leaf.floating.height = leaf
                            .floating
                            .height
                            .overflowing_sub(delta.1 as u16)
                            .0
                            .min(leaf.max_size.1)
                            .max(leaf.min_size.1);
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
                if let Some(parent_) = parent_h {
                    let parent = &mut self.nodes[parent_];
                    if let NodeContents::Node(node) = &mut parent.info {
                        let diff = delta.0 as f32 / parent.rect.width as f32;
                        node.ratio = (node.ratio + diff).min(Split::MAX).max(Split::MIN);
                    }
                    if parent_v.is_none() || depth1 > depth2 {
                        q.push(parent_);
                    }
                }
                if let Some(parent_) = parent_v {
                    let parent = &mut self.nodes[parent_];
                    if let NodeContents::Node(node) = &mut parent.info {
                        let diff = delta.1 as f32 / parent.rect.height as f32;
                        node.ratio = (node.ratio + diff).min(Split::MAX).max(Split::MIN);
                    }
                    if q.is_empty() {
                        q.push(parent_)
                    }
                }
                while !q.is_empty() {
                    let node_ = q.pop().unwrap();
                    let node = &self.nodes[node_];
                    match &node.info {
                        NodeContents::Node(_) => self.resize_node(aux, node_, &mut q),
                        NodeContents::Leaf(leaf) => {
                            if !node.absent {
                                self.apply_pos_size(aux, leaf.client, &node.rect, true)?
                            }
                        }
                        _ => (),
                    }
                }
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
            self.resize_tiled(aux, 0, Some(&tiling_size))?;
            self.tiling_size = tiling_size;
        }
        Ok(())
    }

    fn resize_tiled(&mut self, aux: &Aux, node: usize, size: Option<&Rect>) -> Result<()> {
        if let Some(size) = size {
            self.nodes[node].rect.copy(size);
        }
        let mut q = vec![0];
        while !q.is_empty() {
            let node_ = q.pop().unwrap();
            let node = &self.nodes[node_];
            match &node.info {
                NodeContents::Node(..) => self.resize_node(aux, node_, &mut q),
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

    pub fn resize_all(
        &mut self,
        aux: &Aux,
        node: usize,
        available: &Rect,
        new_size: &Rect,
    ) -> Result<()> {
        self.nodes[node].rect.copy(available);
        let mut q = vec![node];
        while !q.is_empty() {
            let node_ = q.pop().unwrap();
            let node = &self.nodes[node_];
            match &node.info {
                NodeContents::Node(..) => self.resize_node(aux, node_, &mut q),
                NodeContents::Leaf(leaf) => {
                    if !node.absent {
                        let leaf_client = leaf.client;
                        let floating = {
                            if let NodeContents::Leaf(leaf) = &mut self.nodes[node_].info {
                                leaf.floating.x = (leaf.floating.x as f32
                                    / self.tiling_size.width as f32
                                    * new_size.width as f32)
                                    .round() as _;
                                leaf.floating.y = (leaf.floating.y as f32
                                    / self.tiling_size.height as f32
                                    * new_size.height as f32)
                                    .round() as _;
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
                }
                _ => (),
            }
        }
        Ok(())
    }

    pub fn remove_node(&mut self, aux: &Aux, node: usize) -> Result<()> {
        let parent = self.nodes[node].parent;
        self.nodes[node].info = NodeContents::Empty;
        self.free_nodes.push(node);
        if let Some((parent_, first)) = parent {
            {
                let info = match &self.nodes[parent_].info {
                    NodeContents::Node(node) => {
                        let child = node.get_child(!first);
                        self.free_nodes.push(child);
                        let child = &self.nodes[child];
                        Some((child.info.clone(), child.absent))
                    }
                    _ => None,
                };
                let parent = &mut self.nodes[parent_];
                if let Some((info, absent)) = info {
                    parent.info = info;
                    parent.absent = absent;
                } else {
                    parent.info = NodeContents::Empty;
                }
                if let NodeContents::Leaf(leaf) = &parent.info {
                    self.clients[leaf.client].node = parent_;
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
