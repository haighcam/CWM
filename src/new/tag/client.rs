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
use crate::config::{Theme, FlagArg, SetArg};
use super::{Layer, StackLayer, Split, Tag, super::{WindowManager, WindowLocation}, node::NodeContents};

#[derive(Debug)]
pub struct ClientFlags {
    pub urgent: bool,
    pub hidden: bool,
    pub floating: bool,
    pub fullscreen: bool,
    pub sticky: bool
}

impl ClientFlags {
    pub fn get_layer(&self) -> usize {
        match self {
            Self { fullscreen: true, .. } => Layer::FULLSCREEN,
            Self { floating: true, .. } => Layer::FLOATING,
            Self { .. } => Layer::TILING,
        }
    }

    fn absent(&self) -> bool {
        self.floating | self.fullscreen | self.hidden
    }
}

#[derive(Debug)]
pub struct ClientArgs {
    pub focus: bool,
    pub flags: ClientFlags,
    pub centered: bool,
    pub managed: bool,
    min_size: (u16, u16),
    max_size: (u16, u16),
    size: (u16, u16),
    pos: Option<(i16, i16)>,
    layer: StackLayer,
    class: Option<String>,
    instance: Option<String>,
    name: Option<String>,
    tag: Option<usize>,
    parent: Option<usize>, // a leaf
    split: Option<Split>
}

impl ClientArgs {
    pub fn new(theme: &Theme) -> Self {
        Self {
            focus: true,
            flags: ClientFlags {
                fullscreen: false,
                floating: false,
                urgent: false,
                sticky: false,
                hidden: false,
            },
            centered: false,
            managed: true,
            min_size: (theme.window_min_width, theme.window_min_height),
            size: (theme.window_width, theme.window_height),
            max_size: (std::u16::MAX, std::u16::MAX),
            pos: None,
            class: None,
            name: None,
            instance: None,
            layer: StackLayer::Normal,
            parent: None,
            split: None,
            tag: None
        }
    }
    fn process_state(&mut self, state: Atom, atoms: &AtomCollection) {
        if state == atoms._NET_WM_STATE_FULLSCREEN {
            self.flags.fullscreen = true;
        } else if state == atoms._NET_WM_STATE_STICKY {
            self.flags.sticky = true;
        }
    }

    fn process_hints(&mut self, hints: WmHints) {
        self.flags.urgent = hints.urgent
    }

    fn prcoess_size_hints(&mut self, size_hints: WmSizeHints) {
        if let Some(max) = size_hints.max_size.map(|x| (x.0 as u16, x.1 as u16)) {
            self.max_size = max;
        }
        if let Some(min) = size_hints.min_size.map(|x| (x.0 as u16, x.1 as u16)) {
            self.min_size = min;
            if self.max_size == self.min_size {
                self.flags.floating = true;
            }
        }
        if let Some((_, w, h)) = size_hints.size {
            self.size = (w as u16, h as u16);
        }
    }

    fn process_class(&mut self, class: WmClass) {
        self.class.replace(String::from_utf8(class.class().to_vec()).unwrap());
    }

    fn process_name(&mut self, name: GetPropertyReply) {
        self.name.replace(String::from_utf8(name.value).unwrap());
    }

    fn process_transient(&mut self, transient: GetPropertyReply) {
        if let Some(mut transient) = transient.value32() {
            if transient.next().map_or(false, |transient| transient != NONE) {
                self.flags.floating = true;
            }
        }
    }
}

pub struct Client {
    pub name: Option<String>,
    class: Option<String>,
    instance: Option<String>,
    pub border_width: u16,
    pub layer: StackLayer,
    last_layer: StackLayer,
    pub node: usize,
    pub stack_pos: usize,
    pub layer_pos: (usize, usize),
    pub flags: ClientFlags,
    pub win: Window,
    pub frame: Window
}

impl Client {

}


impl Tag {
    pub fn show_client(&self, conn: &Connections, client: usize, atoms: &AtomCollection) -> CwmRes<()>  {
        let client = &self.clients[client];
        let mut bytes: Vec<u8> = Vec::with_capacity(8);
        1u32.serialize_into(&mut bytes);
        NONE.serialize_into(&mut bytes);
        change_property(&conn.dpy, PropMode::REPLACE, client.win, atoms.WM_STATE, atoms.WM_STATE, 32, 2, &bytes)?;
        map_window(&conn.dpy, client.frame)?;
        map_window(&conn.dpy, client.win)?;
        Ok(())
    }

    pub fn hide_client(&self, conn: &Connections, client: usize, atoms: &AtomCollection) -> CwmRes<()> {
        let client = &self.clients[client];
        let mut bytes: Vec<u8> = Vec::with_capacity(8);
        unmap_window(&conn.dpy, client.win)?;
        unmap_window(&conn.dpy, client.frame)?;
        3u32.serialize_into(&mut bytes);
        NONE.serialize_into(&mut bytes);
        change_property(&conn.dpy, PropMode::REPLACE, client.win, atoms.WM_STATE, atoms.WM_STATE, 32, 2, &bytes)?;
        Ok(())
    }

    pub fn focus_client(&mut self, conn: &Connections, _client: usize, theme: &Theme, hooks: &mut Hooks) -> CwmRes<()> {
        let client = &self.clients[_client];
        self.focus_stack.remove_node(client.stack_pos);
        if let Some(client) = self.focus_stack.front() {
            let client = &self.clients[*client];
            change_window_attributes(&conn.dpy, client.frame, &ChangeWindowAttributesAux::new().border_pixel(theme.border_color_unfocused))?;
        }
        let client = &mut self.clients[_client];
        client.stack_pos = self.focus_stack.push_front(_client);
        set_input_focus(&conn.dpy, InputFocus::PARENT, client.win, CURRENT_TIME)?;
        // focused window callback
        change_window_attributes(&conn.dpy, client.frame, &ChangeWindowAttributesAux::new().border_pixel(theme.border_color_focused))?;
        let name = client.name.clone();
        self.set_active_window(name, hooks);

        Ok(())
    }

    pub fn apply_pos_size(&self, conn: &Connections, client: usize, size: &Rect, border: bool) -> CwmRes<()> {
        let client = &self.clients[client];
        print!("{:?}", size);
        let (aux1, aux2) = size.aux_border(if border {client.border_width} else {0});
        configure_window(&conn.dpy, client.frame, &aux1)?;
        configure_window(&conn.dpy, client.win, &aux2)?;
        Ok(())
    }

    pub fn set_fullscreen(&mut self, conn: &Connections, client: usize, arg: &FlagArg) -> CwmRes<()> {
        if arg.apply(&mut self.clients[client].flags.fullscreen) {
            self.switch_layer(conn, client)?;
        }
        Ok(())
    }

    pub fn set_floating(&mut self, conn: &Connections, client: usize, arg: &FlagArg) -> CwmRes<()> {
        if arg.apply(&mut self.clients[client].flags.floating) {
            self.switch_layer(conn, client)?;
        }
        Ok(())
    }

    pub fn set_stack_layer(&mut self, conn: &Connections, client: usize, arg: &SetArg<StackLayer>) -> CwmRes<()> {
        let (layer, last) = (self.clients[client].layer, self.clients[client].last_layer);
        if arg.apply_arg(&mut self.clients[client].layer, last) {
            self.clients[client].last_layer = layer;
            self.switch_layer(conn, client)?;
        }
        Ok(())
    }

    pub fn set_sticky(&mut self, client: usize, arg: &FlagArg) {
        arg.apply(&mut self.clients[client].flags.sticky);
    }
}

impl WindowManager {
    pub fn unmanage_client(&mut self, tag: usize, client: usize) -> CwmRes<()> {
        let tag = &mut self.tags[tag];
        tag.free_clients.push(client);
        {
            let client = &mut tag.clients[client];
            let (layer, layer_pos) = client.layer_pos;
            tag.layers[layer].remove(layer_pos);
            tag.focus_stack.remove_node(client.stack_pos);
            client.flags.hidden = true;
        }
        if let Some(client) = tag.focus_stack.front() {
            tag.focus_client(&self.conn, *client, &self.theme, &mut self.hooks);
        } else {
            set_input_focus(&self.conn.dpy, InputFocus::POINTER_ROOT, self.root, CURRENT_TIME)?; 
        }
        let client = &tag.clients[client];
        destroy_window(&self.conn.dpy, client.frame)?;
        reparent_window(&self.conn.dpy, client.win, self.root, 0, 0)?;
        tag.remove_node(&self.conn, client.node);
        tag.print_node(0, 0);
        Ok(())
    }

    pub fn process_args(&mut self, win: Window, args: &mut ClientArgs) -> CwmRes<()> {
        let state_cookie = get_property(&self.conn.dpy, false, win, self.atoms._NET_WM_STATE, AtomEnum::ATOM, 0, 2048)?;
        let hints_cookie = WmHints::get(&self.conn.dpy, win)?;
        let size_hints_cookie = WmSizeHints::get_normal_hints(&self.conn.dpy, win)?;
        let class_cookie = WmClass::get(&self.conn.dpy, win)?;
        let name_cookie = get_property(&self.conn.dpy, false, win, AtomEnum::WM_NAME, self.atoms.UTF8_STRING, 0, 2048)?;
        let wm_name_cookie = get_property(&self.conn.dpy, false, win, self.atoms._NET_WM_NAME, self.atoms.UTF8_STRING, 0, 2048)?;
        let transient_cookie = get_property(&self.conn.dpy, false, win, AtomEnum::WM_TRANSIENT_FOR, AtomEnum::WINDOW, 0, 1)?;

        if let Ok(states) = state_cookie.reply() {
            if let Some(states) = states.value32() {
                for state in states {
                    args.process_state(state, &self.atoms);
                }
            }
        }
        hints_cookie.reply().map(|hints| args.process_hints(hints))?;
        size_hints_cookie.reply().map(|size_hints| args.prcoess_size_hints(size_hints))?;
        class_cookie.reply().map(|class| args.process_class(class))?;
        name_cookie.reply().map(|name| args.process_name(name))?;
        wm_name_cookie.reply().map(|name| args.process_name(name))?;
        transient_cookie.reply().map(|transient| args.process_transient(transient))?;
        Ok(())
    }

    pub fn manage_client(&mut self, win: Window, args: ClientArgs) -> CwmRes<()> {
        let ClientArgs { focus, flags, centered, managed: _, min_size, max_size, size, layer, class, instance, name, tag, pos, parent, split } = args;
        let tag_idx = tag.unwrap_or_else(|| self.focused_tag());
        let tag = &mut self.tags[tag_idx];
        println!("{:?} {:?}", tag.tiling_size, tag.size);

        let floating_rect = if centered || pos.is_none() {
            Rect::new(
                tag.tiling_size.x + (tag.tiling_size.width as i16 - size.0 as i16) / 2, 
                tag.tiling_size.y + (tag.tiling_size.height as i16 - size.1 as i16) / 2, 
                size.0, size.1
            )
        } else {
            let pos = pos.unwrap();
            Rect::new(pos.0, pos.0, size.0, size.1)
        };

        let absent = flags.absent();
        let hidden = flags.hidden;
        let frame = self.conn.dpy.generate_id().unwrap();
        let client = Client {
            name,
            node: 0,
            class,
            instance,
            border_width: self.theme.border_width,
            layer,
            last_layer: layer,
            stack_pos: 0,
            layer_pos: (0, 0),
            flags,
            win, 
            frame
        };

        let client = if let Some(idx) = tag.free_clients.pop() {
            tag.clients[idx] = client;
            idx
        } else {
            tag.clients.push(client);
            tag.clients.len() - 1
        };

        let info = NodeContents::leaf(client, min_size, max_size, floating_rect);

        match tag.nodes[0].info {
            NodeContents::Empty => {
                tag.nodes[0].info = info;
                tag.nodes[0].absent = absent;
            },
            NodeContents::Leaf(..) => {
                tag.split_leaf(&self.conn, 0, split, absent, client, info)?;
            },
            NodeContents::Node(..) => {
                let leaf = parent.unwrap_or_else(|| *tag.focus_stack.front().unwrap());
                let leaf = tag.clients[leaf].node;
                tag.split_leaf(&self.conn, leaf, split, absent, client, info)?;
            }
        }

        tag.clients[client].stack_pos = if focus {
            tag.focus_stack.push_front(client)
        } else {
            tag.focus_stack.push_back(client)
        };

        let size = tag.get_rect(client).unwrap();
        info!(" size {:?}", size);
        let border_width = if tag.clients[client].flags.floating {0} else {tag.clients[client].border_width};
        let aux = CreateWindowAux::new().event_mask(EventMask::ENTER_WINDOW | EventMask::FOCUS_CHANGE | EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY);
        create_window(&self.conn.dpy, COPY_DEPTH_FROM_PARENT, frame, self.root, size.x, size.y, size.width - border_width * 2, size.height - border_width * 2, border_width, WindowClass::COPY_FROM_PARENT, COPY_FROM_PARENT, &aux)?;
        reparent_window(&self.conn.dpy, win, frame, 0, 0)?;

        tag.set_layer(&self.conn, client, focus)?;
        if hidden {
            tag.hide_client(&self.conn, client, &self.atoms)?
        } else {
            tag.show_client(&self.conn, client, &self.atoms)?
        }
        if !hidden && focus {
            tag.focus_client(&self.conn, client, &self.theme, &mut self.hooks)?
        } else {
            change_window_attributes(&self.conn.dpy, frame, &ChangeWindowAttributesAux::new().border_pixel(self.theme.border_color_unfocused))?;
        }
        self.conn.dpy.flush()?;
        self.windows.insert(frame, WindowLocation::Client(tag.id, client));
        tag.print_node(0, 0);
        Ok(())
    }
}