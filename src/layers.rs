use x11rb::{
    connection::Connection,
    protocol::xproto::*,
};
use crate::{
    tag::Tag,
    utils::{Stack, StackElem},
    WindowManager, CWMRes
};

pub(crate) enum Layer {
    Single(Option<StackElem<Window>>),
    Multi(Stack<Window>)
}

pub(crate) struct Layers(pub(crate) Vec<Layer>);

impl Default for Layers {
    fn default() -> Self {
        Self(vec![Layer::multi(), Layer::multi(), Layer::single(), Layer::multi(), Layer::multi(), Layer::single()])
    }
}

impl Layer {
    fn single() -> Self {
        Self::Single(None)
    }
    fn multi() -> Self {
        Self::Multi(Stack::new())
    }

    pub(crate) fn remove(&mut self, item: StackElem<Window>) {
        match self {
            Self::Single(item) => {item.take();},
            Self::Multi(stack) => stack.unlink_node(item)
        }
    }

    pub(crate) fn add(&mut self, item: StackElem<Window>) -> Option<StackElem<Window>> {
        match self {
            Self::Single(old_item) => old_item.replace(item),
            Self::Multi(stack) => {stack.link_node_front(item); None}
        }
    }

    pub(crate) fn add_back(&mut self, item: StackElem<Window>) -> Option<StackElem<Window>> {
        match self {
            Self::Single(old_item) => old_item.replace(item),
            Self::Multi(stack) => {stack.link_node_back(item); None}
        }
    }

    fn front(&self) -> Option<Window> {
        match self {
            Self::Single(item) => item.map(|x| *unsafe{x.as_ref()}.element()),
            Self::Multi(stack) => stack.front().copied()
        }
    }
}

impl Layers {
    pub(crate) const TILING: usize = 0;
    pub(crate) const FLOATING: usize = 1;
    pub(crate) const FULLSCREEN: usize = 2;
    pub(crate) const AOT: usize = 3;
    pub(crate) const COUNT: usize = Self::AOT * 2;

    pub(crate) fn get_layer_bound(&self, layer: usize) -> Option<Window> {
        if layer < Self::COUNT {
            for layer in layer..Self::COUNT {
                if let Some(window) = self.0[layer].front() {
                    return Some(window)
                }
            }
            None
        } else {
            None
        }
    }
}

impl Tag {
    pub(crate) fn set_layer(&mut self, wm: &WindowManager, win: Window, focused: bool) -> CWMRes<()> {
        let mut other = None;
        if let Some(client) = self.clients.get(&win).map(|x| x.borrow()) {
            let layer = client.flags.get_layer();
            if focused {
                other = self.layers.0[layer].add(client.layer_pos);
            } else {
                other = self.layers.0[layer].add_back(client.layer_pos);
            }
            let mut aux = client.get_rect().map(|x| x.aux().border_width(wm.theme.border_width as u32)).unwrap_or_else(|| self.monitor_size.aux().border_width(0));
            aux = if let Some(sibling) = self.layers.get_layer_bound(layer + if focused {1} else {0}) {
                aux.sibling(sibling).stack_mode(StackMode::BELOW)
            } else {
                aux.stack_mode(StackMode::ABOVE)
            };
            configure_window(&wm.conn.dpy, client.frame, &aux)?;
            configure_window(&wm.conn.dpy, client.win, &aux.x(None).y(None).sibling(None).stack_mode(None).border_width(0))?;
            client.set_layer(layer);
        }
        if let Some(win) = other.map(|x| *unsafe{x.as_ref()}.element()) {
            if let Some(mut client) = self.clients.get(&win).map(|x| x.borrow_mut()) {
                client.flags.fullscreen = false;
                if !client.flags.floating {
                    client.set_present(wm)?
                }
            }
            self.set_layer(wm, win, true)?;
        }
        Ok(())
    }

    #[inline]
    pub(crate) fn switch_layer(&mut self, wm: &WindowManager, win: Window) -> CWMRes<()> {
        if let Some(client) = self.clients.get(&win).map(|x| x.borrow()) {
            self.layers.0[client.layer()].remove(client.layer_pos);
            match (client.layer() % Layers::AOT == Layers::TILING, client.flags.get_layer() % Layers::AOT == Layers::TILING) {
                (false, true) => client.set_present(wm)?,
                (true, false) => client.set_absent(wm)?,
                _ => ()
            }
        }
        self.set_layer(wm, win, true)
    }
}