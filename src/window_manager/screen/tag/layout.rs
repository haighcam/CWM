use x11rb::{
    connection::Connection,
    protocol::xproto::*
};
use std::{
    collections::{HashMap, HashSet},
    cell::RefCell,
    rc::{Rc, Weak}
};
use log::info;
use crate::utils::Rect;
use super::{WindowManager, Client, Tag};

impl LayoutNodeInfo {
    pub fn resize_tiled(&self, rect: &Rect, to_process: &mut Vec<Rc<RefCell<Layout>>>) {
        let mut ch1 = self.children.0.borrow_mut();
        let mut ch2 = self.children.1.borrow_mut();
        match (ch1.absent, ch2.absent) {
            (true, true) => {}, // should not happen since the parent should be absent
            (true, false) => {
                ch2.rect.copy(rect);
                to_process.push(self.children.1.clone());
            },
            (false, true) => {
                ch1.rect.copy(rect);
                to_process.push(self.children.0.clone());
            }
            (false, false) => {
                rect.split(self.split, self.vert, &mut ch1.rect, &mut ch2.rect);
                to_process.push(self.children.0.clone());
                to_process.push(self.children.1.clone());
            }
        }
    }
    pub fn child(&self, first: bool) -> Rc<RefCell<Layout>> {
        if first { self.children.0.clone() } else { self.children.1.clone() }
    }
}

impl LayoutLeafInfo {
    pub fn client(&self) -> &Rc<RefCell<Client>> {
        &self.client
    }
}

#[derive(Clone)]
pub struct LayoutNodeInfo {
    split: f32,
    vert: bool,
    children: (Rc<RefCell<Layout>>, Rc<RefCell<Layout>>),
}

#[derive(Clone)]
pub struct LayoutLeafInfo {
    floating: Rect,
    min_size: (u16, u16),
    max_size: (u16, u16),
    client: Rc<RefCell<Client>>
}

#[derive(Clone)]
pub enum LayoutInfo {
    Node(LayoutNodeInfo),
    Leaf(LayoutLeafInfo),
    Empty
}

impl Default for LayoutInfo {
    fn default() -> Self {
        Self::Empty
    }
}

impl LayoutInfo {
    pub fn leaf(floating: Rect, min_size: (u16, u16), max_size: (u16, u16), client: Rc<RefCell<Client>>) -> Self {
        Self::Leaf(LayoutLeafInfo {floating, min_size, max_size, client})
    }

    pub fn node(split: f32, vert: bool, children: (Rc<RefCell<Layout>>, Rc<RefCell<Layout>>)) -> Self {
        Self::Node(LayoutNodeInfo {split, vert, children})
    }
}

#[derive(Default, Clone)]
pub struct Layout {
    parent: Option<(Weak<RefCell<Layout>>, bool)>, // parent and which child we are
    pub rect: Rect,
    pub info: LayoutInfo,
    pub absent: bool // this node (and all of its children) is not shown in the tiled layout
}

impl Layout {
    const SPLIT_MAX: f32 =  0.9;
    const SPLIT_MIN: f32 = 1.0 - Self::SPLIT_MAX;
}

impl Tag {
    pub fn client_under_cursor(&self, pos: &(i16, i16)) -> Option<Rc<RefCell<Client>>> {
        fn check_layout(_layout: &Rc<RefCell<Layout>>, q: &mut Vec<Rc<RefCell<Layout>>>, pos: &(i16, i16)) {
            let layout = _layout.borrow();
            if layout.rect.contains(pos) && !layout.absent {
                q.push(_layout.clone())
            }
        }
        let mut q = vec![];
        check_layout(&self.layout, &mut q, pos);
        while !q.is_empty() {
            let layout = q.pop().unwrap();
            let layout = layout.borrow();
            match &layout.info {
                LayoutInfo::Leaf(leaf) => return Some(leaf.client.clone()),
                LayoutInfo::Node(node) => {
                    check_layout(&node.children.0, &mut q, pos);
                    check_layout(&node.children.1, &mut q, pos);
                },
                _ => ()
            }
        }
        None
    }

    pub fn move_client(&mut self, wm: &WindowManager<impl Connection>, win: Window, delta: (i16, i16), pos: &(i16, i16)) {
        if let Some(client) = self.clients.get(&win) {
            let client = client.borrow();
            let mut layout = client.layout_mut();
            if !client.flags.fullscreen {
                if client.flags.floating {
                    let mut layout = layout.borrow_mut();
                    match &mut layout.info {
                        LayoutInfo::Leaf(leaf)  => {
                            leaf.floating.x += delta.0;
                            leaf.floating.y += delta.1;
                            client.apply_pos_size(wm, &leaf.floating);
                        },
                        _ => ()
                    }
                } else {
                    if !layout.borrow().rect.contains(pos) {
                        if let Some(other) = self.client_under_cursor(pos) {
                            let other = other.borrow();
                            let mut other_layout = other.layout_mut();
                            {
                                let mut layout = layout.borrow_mut();
                                let mut other_layout = other_layout.borrow_mut();
                                let info = other_layout.info.clone();
                                other_layout.info = layout.info.clone();
                                layout.info = info;
                                client.apply_pos_size(wm, &other_layout.rect);
                                other.apply_pos_size(wm, &layout.rect);
                            }
                            let temp = other_layout.clone();
                            *other_layout = layout.clone();
                            *layout = temp;
                        }
                    }
                }
            }
        }
    }
    pub fn resize_client(&mut self, wm: &WindowManager<impl Connection>, win: Window, delta: (i16, i16), left: bool, top: bool) {
        if let Some(client) = self.clients.get(&win) {
            let (fullscreen, floating, layout) = {
                let client = client.borrow();
                let layout = client.layout();
                (client.flags.fullscreen, client.flags.floating, layout.clone())
            };
            if !fullscreen {
                if floating {
                    let mut layout = layout.borrow_mut();
                    match &mut layout.info {
                        LayoutInfo::Leaf(leaf)  => {
                            if left {
                                leaf.floating.x += delta.0;
                                leaf.floating.width = leaf.floating.width.overflowing_sub(delta.0 as u16).0.min(leaf.max_size.0).max(leaf.min_size.0);
                            } else {
                                leaf.floating.width = leaf.floating.width.overflowing_add(delta.0 as u16).0.min(leaf.max_size.0).max(leaf.min_size.0);
                            }
                            if top {
                                leaf.floating.y += delta.1;
                                leaf.floating.height = leaf.floating.height.overflowing_sub(delta.1 as u16).0.min(leaf.max_size.1).max(leaf.min_size.1);
                            } else {
                                leaf.floating.height = leaf.floating.height.overflowing_add(delta.1 as u16).0.min(leaf.max_size.1).max(leaf.min_size.1);
                            }
                            client.borrow().apply_pos_size(wm, &leaf.floating);
                        },
                        _ => ()
                    }
                } else {
                    let ((parent_h, depth1), (parent_v, depth2)) = {
                        let layout = layout.borrow();
                        (layout.get_split_parent(if left {Side::Left} else {Side::Right}), layout.get_split_parent(if top {Side::Top} else {Side::Bottom})) // merge into one search if it is too slow (but it shouldn't be)
                    };
                    let mut q = vec![];
                    if let Some(parent) = parent_h {
                        {
                            let parent: &mut Layout = &mut parent.borrow_mut();
                            match &mut parent.info {
                                LayoutInfo::Node(node)  => {
                                    let diff = delta.0 as f32 / parent.rect.width as f32;
                                    info!("diff {}", diff);
                                    node.split = (node.split + diff).min(Layout::SPLIT_MAX).max(Layout::SPLIT_MIN);
                                },
                                _ => ()
                            }
                        }
                        if parent_v.is_none() || depth1 > depth2 {
                            q.push(parent);
                        }
                    }
                    if let Some(parent) = parent_v {
                        {
                            let parent: &mut Layout = &mut parent.borrow_mut();
                            match &mut parent.info {
                                LayoutInfo::Node(node)  => {
                                    let diff = delta.1 as f32 / parent.rect.height as f32;
                                    node.split = (node.split + diff).min(Layout::SPLIT_MAX).max(Layout::SPLIT_MIN);
                                },
                                _ => ()
                            }
                        }
                        if q.is_empty() {
                            q.push(parent);
                        }
                    }
                    while !q.is_empty() {
                        let layout = q.pop().unwrap();
                        let layout: &mut Layout = &mut layout.borrow_mut();
                        match &mut layout.info {
                            LayoutInfo::Node(node) => {
                                node.resize_tiled(&layout.rect, &mut q);
                            },
                            LayoutInfo::Leaf(leaf) => {
                                let client = leaf.client.borrow();
                                if !client.flags.fullscreen && !client.flags.floating {
                                    client.apply_pos_size(wm, &layout.rect);
                                }
                            },
                            _ => ()
                        }
                    }
                }
            }
        }
    }
}

#[derive(PartialEq)]
enum Side {
    Left,
    Right,
    Top,
    Bottom
}

impl Layout {
    pub fn print(&self, depth: usize) {
        let offset = std::iter::repeat(" ").take(depth).fold(String::new(), |x, y| x + y);
        match &self.info {
            LayoutInfo::Node(node) => {
                println!("{}node:", offset);
                node.children.0.borrow().print(depth+1);
                node.children.1.borrow().print(depth+1);
            },
            LayoutInfo::Leaf(_) => {
                println!("{}leaf", offset);
            }
            LayoutInfo::Empty => {
                println!("{}empty", offset);
            }
        }
    }

    pub fn new(rect: Rect, parent: Option<(Weak<RefCell<Layout>>, bool)>, absent: bool) -> Self {
        Self { parent, rect, info: LayoutInfo::Empty, absent }
    }

    pub fn parent(&self) -> Option<(Rc<RefCell<Layout>>, bool)> {
        self.parent.as_ref().and_then(|(x, first)| x.upgrade().map(|x| (x, *first)))
    }

    pub fn propagate_absent(wm: &WindowManager<impl Connection>, layout: Rc<RefCell<Layout>>) {
        let mut parent = Some(layout);
        let mut prev_parent = None;
        while parent.is_some() {
            info!("propagating absent");
            prev_parent = parent.take();
            parent = {
                let mut parent = prev_parent.as_ref().unwrap().borrow_mut();
                match &parent.info {
                    LayoutInfo::Node(node) => {
                        info!("{} {}", node.children.0.borrow().absent, node.children.1.borrow().absent);
                        let absent = node.children.0.borrow().absent && node.children.1.borrow().absent;
                        if parent.absent != absent {
                            parent.absent = absent;
                            parent.parent().map(|x| x.0)
                        } else {
                            None
                        }
                    },
                    _ => None
                }
            }
        }
        if let Some(parent) = prev_parent {
            let mut q = vec![parent];
            while !q.is_empty() {
                let layout = q.pop().unwrap();
                let layout = layout.borrow();
                match &layout.info {
                    LayoutInfo::Node(node) => node.resize_tiled(&layout.rect, &mut q),
                    LayoutInfo::Leaf(leaf) => if !layout.absent {
                        leaf.client.borrow().apply_pos_size(wm, &layout.rect)
                    },
                    _ => ()
                }
            }
        }
    }

    fn get_split_parent(&self, split_dir: Side) -> (Option<Rc<RefCell<Layout>>>, usize) {
        let mut _parent = self.parent().map(|x| x.0);
        let mut i = 0;
        while _parent.is_some() {
            _parent = {
                let parent = _parent.as_ref().unwrap().borrow();
                match (&parent as &Layout, &split_dir) {
                    (Layout { info: LayoutInfo::Node(LayoutNodeInfo{vert: true, ..}), rect, .. }, Side::Left)
                        if rect.x < self.rect.x => break,
                    (Layout { info: LayoutInfo::Node(LayoutNodeInfo{vert: true, ..}), rect, .. }, Side::Right) 
                        if (rect.x + rect.width as i16) > (self.rect.x + self.rect.width as i16) => break,
                    (Layout { info: LayoutInfo::Node(LayoutNodeInfo{vert: false, ..}), rect, .. }, Side::Top) 
                        if rect.y < self.rect.y => break,
                    (Layout { info: LayoutInfo::Node(LayoutNodeInfo{vert: false, ..}), rect, .. }, Side::Bottom) 
                        if (rect.y + rect.height as i16) > (self.rect.y + self.rect.height as i16) => break,
                    _ => ()
                }
                parent.parent().map(|x| x.0)
            };
            i += 1;
        }
        (_parent, i)
    }

    pub fn tiled(&self) -> &Rect {
        &self.rect
    }

    pub fn floating(&self) -> Option<&Rect> {
        match &self.info {
            LayoutInfo::Leaf(LayoutLeafInfo { floating, .. }) => Some(floating),
            _ => None
        }
    }

    pub fn resize_tiled(root: Rc<RefCell<Layout>>, wm: &WindowManager<impl Connection>, available: Option<&Rect>) {
        available.map(|x| root.borrow_mut().rect.copy(x));
        let mut q = vec![root];
        while !q.is_empty() {
            if let Some(layout) = q.pop() {
                let layout: &mut Layout = &mut layout.borrow_mut();
                match &mut layout.info {
                    LayoutInfo::Leaf(leaf) => if !layout.absent {
                        leaf.client.borrow().apply_pos_size(wm, &layout.rect);
                    },
                    LayoutInfo::Node(node) => node.resize_tiled(&layout.rect, &mut q), 
                    _ => ()
                }
            }            
        }
    }

    pub fn resize_all(root: Rc<RefCell<Layout>>, wm: &WindowManager<impl Connection>, available: &Rect, size: &Rect, new_size: &Rect) {
        root.borrow_mut().rect.copy(available);
        let mut q = vec![root];
        while !q.is_empty() {
            if let Some(layout) = q.pop() {
                let layout: &mut Layout = &mut layout.borrow_mut();
                match &mut layout.info {
                    LayoutInfo::Leaf(leaf) => {
                        leaf.floating.x = (leaf.floating.x as f32 / size.width as f32 * new_size.width as f32).round() as _;
                        leaf.floating.y = (leaf.floating.y as f32 / size.height as f32 * new_size.height as f32).round() as _;
                        let client = leaf.client.borrow();
                        let rect = if client.flags.fullscreen {
                            &new_size
                        } else if client.flags.floating {
                            &leaf.floating
                        } else {
                            &layout.rect
                        };
                        client.apply_pos_size(wm, rect);
                    },
                    LayoutInfo::Node(node) => node.resize_tiled(&layout.rect, &mut q), 
                    _ => ()
                }
            }            
        }
    }
}