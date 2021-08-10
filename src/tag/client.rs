use x11rb::{
    connection::Connection,
    protocol::xproto::*,
    properties::*,
    x11_utils::Serialize,
    COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE
};
use crate::utils::Rect;
use crate::connections::{Aux, SetArg};
use super::{Layer, StackLayer, Split, Tag, super::{WindowManager, WindowLocation}, node::NodeContents};
use anyhow::{Context, Result};
use log::info;

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
    net_name: bool,
    tag: Option<u32>,
    parent: Option<usize>, // a leaf
    split: Option<Split>
}

impl ClientArgs {
    pub fn new(aux: &Aux) -> Self {
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
            min_size: (aux.theme.window_min_width, aux.theme.window_min_height),
            size: (aux.theme.window_width, aux.theme.window_height),
            max_size: (std::u16::MAX, std::u16::MAX),
            pos: None,
            class: None,
            name: None,
            net_name: false,
            instance: None,
            layer: StackLayer::Normal,
            parent: None,
            split: None,
            tag: None
        }
    }
    fn process_state(&mut self, aux: &Aux, state: Atom) {
        if state == aux.atoms._NET_WM_STATE_FULLSCREEN {
            self.flags.fullscreen = true;
        } else if state == aux.atoms._NET_WM_STATE_STICKY {
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
        self.instance.replace(String::from_utf8(class.instance().to_vec()).unwrap());
    }

    fn process_name(&mut self, name: GetPropertyReply, net: bool) {
        if name.length > 0 {
            self.name.replace(String::from_utf8(name.value).unwrap());
            self.net_name = net;
        }
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
    net_name: bool,
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
    pub fn show_client(&self, aux: &Aux, client: usize) -> Result<()>  {
        let client = &self.clients[client];
        let mut bytes: Vec<u8> = Vec::with_capacity(8);
        1u32.serialize_into(&mut bytes);
        NONE.serialize_into(&mut bytes);
        change_property(&aux.dpy, PropMode::REPLACE, client.win, aux.atoms.WM_STATE, aux.atoms.WM_STATE, 32, 2, &bytes).context(crate::code_loc!())?;
        map_window(&aux.dpy, client.frame).context(crate::code_loc!())?;
        map_window(&aux.dpy, client.win).context(crate::code_loc!())?;
        Ok(())
    }

    pub fn hide_client(&self, aux: &Aux, client: usize) -> Result<()> {
        let client = &self.clients[client];
        let mut bytes: Vec<u8> = Vec::with_capacity(8);
        unmap_window(&aux.dpy, client.win).context(crate::code_loc!())?;
        unmap_window(&aux.dpy, client.frame).context(crate::code_loc!())?;
        3u32.serialize_into(&mut bytes);
        NONE.serialize_into(&mut bytes);
        change_property(&aux.dpy, PropMode::REPLACE, client.win, aux.atoms.WM_STATE, aux.atoms.WM_STATE, 32, 2, &bytes).context(crate::code_loc!())?;
        Ok(())
    }

    pub fn focus_client(&mut self, aux: &mut Aux, _client: usize) -> Result<()> {
        let client = &self.clients[_client];
        self.focus_stack.remove_node(client.stack_pos);
        if let Some(client) = self.focus_stack.front() {
            let client = &self.clients[*client];
            change_window_attributes(&aux.dpy, client.frame, &ChangeWindowAttributesAux::new().border_pixel(aux.theme.border_color_unfocused)).context(crate::code_loc!())?;
        }
        let client = &mut self.clients[_client];
        client.stack_pos = self.focus_stack.push_front(_client);
        set_input_focus(&aux.dpy, InputFocus::PARENT, client.win, CURRENT_TIME).context(crate::code_loc!())?;
        // focused window callback
        change_window_attributes(&aux.dpy, client.frame, &ChangeWindowAttributesAux::new().border_pixel(aux.theme.border_color_focused)).context(crate::code_loc!())?;
        let name = client.name.clone();
        self.set_active_window(name, &mut aux.hooks);
        Ok(())
    }

    pub fn apply_pos_size(&self, aux: &Aux, client: usize, size: &Rect, border: bool) -> Result<()> {
        let client = &self.clients[client];
        let (aux1, aux2) = size.aux(if border {client.border_width} else {0});
        configure_window(&aux.dpy, client.frame, &aux1).context(crate::code_loc!())?;
        configure_window(&aux.dpy, client.win, &aux2).context(crate::code_loc!())?;
        Ok(())
    }

    pub fn set_fullscreen(&mut self, aux: &Aux, client: usize, arg: &SetArg<bool>) -> Result<()> {
        if arg.apply(&mut self.clients[client].flags.fullscreen) {
            self.switch_layer(aux, client)?;
        }
        Ok(())
    }

    pub fn set_floating(&mut self, aux: &Aux, client: usize, arg: &SetArg<bool>) -> Result<()> {
        if arg.apply(&mut self.clients[client].flags.floating) {
            self.switch_layer(aux, client)?;
        }
        Ok(())
    }

    pub fn set_stack_layer(&mut self, aux: &Aux, client: usize, arg: &SetArg<StackLayer>) -> Result<()> {
        let (layer, last) = (self.clients[client].layer, self.clients[client].last_layer);
        if arg.apply_arg(&mut self.clients[client].layer, last) {
            self.clients[client].last_layer = layer;
            self.switch_layer(aux, client)?;
        }
        Ok(())
    }

    pub fn set_sticky(&mut self, client: usize, arg: &SetArg<bool>) {
        arg.apply(&mut self.clients[client].flags.sticky);
    }
}

impl WindowManager {
    pub fn unmanage_client(&mut self, tag: Atom, client: usize) -> Result<()> {
        if let Some(tag) = self.tags.get_mut(&tag){
            tag.free_clients.push(client);
            {
                let client = &mut tag.clients[client];
                let (layer, layer_pos) = client.layer_pos;
                tag.layers[layer].remove(layer_pos);
                tag.focus_stack.remove_node(client.stack_pos);
                client.flags.hidden = true;
            }
            if let Some(client) = tag.focus_stack.front() {
                tag.focus_client(&mut self.aux, *client)?;
            } else {
                set_input_focus(&self.aux.dpy, InputFocus::POINTER_ROOT, self.root, CURRENT_TIME)?; 
                tag.set_active_window(None, &mut self.aux.hooks);
            }
            let client = &tag.clients[client];
            self.windows.remove(&client.win);
            self.windows.remove(&client.frame);
            destroy_window(&self.aux.dpy, client.frame).context(crate::code_loc!())?;
            reparent_window(&self.aux.dpy, client.win, self.root, 0, 0).context(crate::code_loc!())?;
            if client.node != 0 {
                tag.remove_node(&self.aux, client.node)?;
            } else {
                tag.nodes[0].info = NodeContents::Empty;
            }
            self.aux.hooks.tag_update(&self.tags, &self.tag_order, self.focused_monitor);
        }
        Ok(())
    }

    pub fn process_args(&mut self, win: Window, args: &mut ClientArgs) -> Result<()> {
        let state_cookie = get_property(&self.aux.dpy, false, win, self.aux.atoms._NET_WM_STATE, AtomEnum::ATOM, 0, 2048).context(crate::code_loc!())?;
        let hints_cookie = WmHints::get(&self.aux.dpy, win).context(crate::code_loc!())?;
        let size_hints_cookie = WmSizeHints::get_normal_hints(&self.aux.dpy, win).context(crate::code_loc!())?;
        let class_cookie = WmClass::get(&self.aux.dpy, win).context(crate::code_loc!())?;
        let name_cookie = get_property(&self.aux.dpy, false, win, AtomEnum::WM_NAME, self.aux.atoms.UTF8_STRING, 0, 2048).context(crate::code_loc!())?;
        let wm_name_cookie = get_property(&self.aux.dpy, false, win, self.aux.atoms._NET_WM_NAME, self.aux.atoms.UTF8_STRING, 0, 2048).context(crate::code_loc!())?;
        let transient_cookie = get_property(&self.aux.dpy, false, win, AtomEnum::WM_TRANSIENT_FOR, AtomEnum::WINDOW, 0, 1).context(crate::code_loc!())?;

        if let Ok(states) = state_cookie.reply() {
            if let Some(states) = states.value32() {
                for state in states {
                    args.process_state(&self.aux, state);
                }
            }
        }
        let _ = hints_cookie.reply().map(|hints| args.process_hints(hints));
        let _ = size_hints_cookie.reply().map(|size_hints| args.prcoess_size_hints(size_hints));
        let _ = class_cookie.reply().map(|class| args.process_class(class));
        let _ = name_cookie.reply().map(|name| args.process_name(name, false));
        let _ = wm_name_cookie.reply().map(|name| args.process_name(name, true));
        let _ = transient_cookie.reply().map(|transient| args.process_transient(transient));
        Ok(())
    }

    pub fn manage_client(&mut self, win: Window, args: ClientArgs) -> Result<()> {
        info!("adding client {:?}", args);
        let ClientArgs { focus, flags, centered, managed: _, min_size, max_size, size, layer, class, instance, name, net_name, tag, pos, parent, split } = args;
        let tag_idx = tag.unwrap_or_else(|| self.focused_tag());
        if let Some(tag) = self.tags.get_mut(&tag_idx) {
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
            let frame = self.aux.dpy.generate_id().unwrap();
            let client = Client {
                name,
                net_name,
                node: 0,
                class,
                instance,
                border_width: self.aux.theme.border_width,
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
                    tag.split_leaf(&self.aux, 0, split, absent, client, info)?;
                },
                NodeContents::Node(..) => {
                    let leaf = parent.unwrap_or_else(|| *tag.focus_stack.front().unwrap());
                    let leaf = tag.clients[leaf].node;
                    tag.split_leaf(&self.aux, leaf, split, absent, client, info)?;
                }
            }

            tag.clients[client].stack_pos = if focus {
                tag.focus_stack.push_front(client)
            } else {
                tag.focus_stack.push_back(client)
            };

            let size = tag.get_rect(client).unwrap();
            let border_width = if tag.clients[client].flags.floating {0} else {tag.clients[client].border_width};
            let aux = CreateWindowAux::new().event_mask(EventMask::ENTER_WINDOW | EventMask::FOCUS_CHANGE | EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY).background_pixel(0);
            let attrs = get_window_attributes(&self.aux.dpy, win)?.reply()?;
            create_window(&self.aux.dpy, COPY_DEPTH_FROM_PARENT, frame, self.root, size.x, size.y, size.width - border_width * 2, size.height - border_width * 2, border_width, WindowClass::COPY_FROM_PARENT, attrs.visual, &aux).context(crate::code_loc!())?;
            reparent_window(&self.aux.dpy, win, frame, 0, 0).context(crate::code_loc!())?;
            change_window_attributes(&self.aux.dpy, win, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE)).context(crate::code_loc!())?;

            tag.set_layer(&self.aux, client, focus)?;
            if hidden {
                tag.hide_client(&self.aux, client)?
            } else {
                tag.show_client(&self.aux, client)?
            }
            if !hidden && focus {
                tag.focus_client(&mut self.aux, client)?
            } else {
                change_window_attributes(&self.aux.dpy, frame, &ChangeWindowAttributesAux::new().border_pixel(self.aux.theme.border_color_unfocused)).context(crate::code_loc!())?;
            }
            self.aux.dpy.flush().context(crate::code_loc!())?;
            self.windows.insert(frame, WindowLocation::Client(tag.id, client));
            self.windows.insert(win, WindowLocation::Client(tag.id, client));
        }
        self.aux.hooks.tag_update(&self.tags, &self.tag_order, self.focused_monitor);
        Ok(())
    }

    pub fn client_property(&mut self, tag: Atom, client_: usize, atom: Atom) {
        let tag = self.tags.get_mut(&tag).unwrap();
        let client = &mut tag.clients[client_];
        if !client.net_name && atom == AtomEnum::WM_NAME.into() {
            if let Some(name) = get_property(&self.aux.dpy, false, client.win, AtomEnum::WM_NAME, self.aux.atoms.UTF8_STRING, 0, 2048).ok().and_then(|cookie| cookie.reply().ok()) {
                if name.length > 0 {
                    let name = String::from_utf8(name.value).unwrap();
                    client.name.replace(name.clone());
                    if tag.focus_stack.front() == Some(&client_) {
                        tag.set_active_window(Some(name), &mut self.aux.hooks)
                    }
                }
            }
        } else if atom == self.aux.atoms._NET_WM_NAME {
            if let Some(name) = get_property(&self.aux.dpy, false, client.win, self.aux.atoms._NET_WM_NAME, self.aux.atoms.UTF8_STRING, 0, 2048).ok().and_then(|cookie| cookie.reply().ok()) {
                if name.length > 0 {
                    client.net_name = true;
                    let name = String::from_utf8(name.value).unwrap();
                    client.name.replace(name.clone());
                    if tag.focus_stack.front() == Some(&client_) {
                        tag.set_active_window(Some(name), &mut self.aux.hooks)
                    }
                }
            }
        } else if atom == AtomEnum::WM_HINTS.into() {
            if let Some(hints) = WmHints::get(&self.aux.dpy, client.win).ok().and_then(|cookie| cookie.reply().ok()) {
                let changed = if hints.urgent {
                    tag.urgent.insert(client_)
                } else {
                    tag.urgent.remove(&client_)
                };
                if changed {
                    self.aux.hooks.tag_update(&self.tags, &self.tag_order, self.focused_monitor)
                }
            }
        }
    }
}