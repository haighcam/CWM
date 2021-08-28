use anyhow::Result;
use log::info;
use x11rb::{
    connection::Connection, properties::*, protocol::xproto::*, wrapper::ConnectionExt as _,
    CURRENT_TIME, NONE,
};

use super::{node::NodeContents, Layer, StackLayer, Tag};
use crate::connections::{Aux, SetArg};
use crate::rules::Rule;
use crate::utils::Rect;
use crate::{WindowLocation, WindowManager};

#[derive(Debug, Clone)]
pub struct ClientFlags {
    pub urgent: bool,
    pub hidden: bool,
    pub floating: bool,
    pub fullscreen: bool,
    pub sticky: bool,
    pub psuedo_urgent: bool,
}

impl ClientFlags {
    pub fn get_layer(&self) -> usize {
        match self {
            Self {
                fullscreen: true, ..
            } => Layer::FULLSCREEN,
            Self { floating: true, .. } => Layer::FLOATING,
            Self { .. } => Layer::TILING,
        }
    }

    pub fn absent(&self) -> bool {
        self.floating | self.fullscreen | self.hidden
    }
}

// maybe replace with a bit field?
#[derive(Default, Debug, Clone)]
pub struct ClientProtocols {
    delete: bool,
}

#[derive(Debug)]
pub struct ClientArgs {
    pub focus: bool,
    pub flags: ClientFlags,
    pub centered: bool,
    pub managed: bool,
    min_size: (u16, u16),
    max_size: (u16, u16),
    pub size: Option<(u16, u16)>,
    pub pos: Option<(i16, i16)>,
    layer: StackLayer,
    class: Option<String>,
    instance: Option<String>,
    name: Option<String>,
    net_name: bool,
    tag: Option<u32>,
    parent: Option<usize>, // a leaf
    protocols: ClientProtocols,
}

impl PartialEq<Rule> for ClientArgs {
    fn eq(&self, other: &Rule) -> bool {
        self.name
            .as_ref()
            .map(|x| other.name.as_ref().map(|y| x == y).unwrap_or(true))
            .unwrap_or_else(|| other.name.is_none())
            && self
                .instance
                .as_ref()
                .map(|x| other.instance.as_ref().map(|y| x == y).unwrap_or(true))
                .unwrap_or_else(|| other.instance.is_none())
            && self
                .class
                .as_ref()
                .map(|x| other.class.as_ref().map(|y| x == y).unwrap_or(true))
                .unwrap_or_else(|| other.class.is_none())
    }
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
                psuedo_urgent: false,
            },
            centered: false,
            managed: true,
            min_size: (aux.theme.window_min_width, aux.theme.window_min_height),
            size: None,
            max_size: (std::u16::MAX, std::u16::MAX),
            pos: None,
            class: None,
            name: None,
            net_name: false,
            instance: None,
            layer: StackLayer::Normal,
            parent: None,
            tag: None,
            protocols: ClientProtocols::default(),
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
                self.size = Some(min);
                self.flags.floating = true;
            }
        }
    }

    fn process_class(&mut self, class: WmClass) {
        self.class
            .replace(String::from_utf8(class.class().to_vec()).unwrap());
        self.instance
            .replace(String::from_utf8(class.instance().to_vec()).unwrap());
    }

    fn process_name(&mut self, name: GetPropertyReply, net: bool) {
        if name.length > 0 {
            self.name.replace(String::from_utf8(name.value).unwrap());
            self.net_name = net;
        }
    }

    fn process_transient(&mut self, transient: GetPropertyReply) {
        if let Some(mut transient) = transient.value32() {
            if transient
                .next()
                .map_or(false, |transient| transient != NONE)
            {
                self.flags.floating = true;
            }
        }
    }

    fn process_protocol(&mut self, aux: &Aux, protocol: Atom) {
        if protocol == aux.atoms.WM_DELETE_WINDOW {
            self.protocols.delete = true;
        }
    }
}

#[derive(Debug, Clone)]
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
    pub frame: Window,
    protocols: ClientProtocols,
    pub ignore_unmaps: usize,
}

impl Client {
    pub fn send_message(&self, aux: &Aux, msg: Atom, val: Atom) -> Result<()> {
        let event = ClientMessageEvent {
            response_type: CLIENT_MESSAGE_EVENT,
            format: 32,
            sequence: 0,
            window: self.win,
            type_: msg,
            data: [val, CURRENT_TIME, 0, 0, 0].into(),
        };
        send_event(&aux.dpy, false, self.win, EventMask::NO_EVENT, event)?;
        Ok(())
    }

    pub fn close(&self, aux: &Aux, kill: bool) -> Result<()> {
        if self.protocols.delete && !kill {
            self.send_message(aux, aux.atoms.WM_PROTOCOLS, aux.atoms.WM_DELETE_WINDOW)?;
        } else {
            kill_client(&aux.dpy, self.win)?;
        }
        Ok(())
    }

    pub fn show(&self, aux: &Aux) -> Result<()> {
        aux.dpy.change_property32(
            PropMode::REPLACE,
            self.win,
            aux.atoms.WM_STATE,
            aux.atoms.WM_STATE,
            &[1, NONE],
        )?;
        map_window(&aux.dpy, self.frame)?;
        map_window(&aux.dpy, self.win)?;
        Ok(())
    }

    pub fn hide(&mut self, aux: &mut Aux, tag: Atom) -> Result<()> {
        info!("hiding window {} {}", self.win, self.frame);
        unmap_window(&aux.dpy, self.win)?;
        unmap_window(&aux.dpy, self.frame)?;
        aux.dpy.change_property32(
            PropMode::REPLACE,
            self.win,
            aux.atoms.WM_STATE,
            aux.atoms.WM_STATE,
            &[3, NONE],
        )?;
        self.ignore_unmaps += 2;
        aux.selection.hide(&aux.dpy, Some(tag), Some(self.node))?;
        Ok(())
    }
}

impl Tag {
    pub fn focus_client(&mut self, aux: &mut Aux, _client: usize) -> Result<()> {
        if self.focused == Some(_client) {
            return Ok(());
        }
        let client = &self.clients[_client];
        if client.flags.hidden {
            return Ok(());
        }
        info!("tag {} set focus {}", self.name, _client);
        if let Some(client) = self.focused {
            let client = &self.clients[client];
            change_window_attributes(
                &aux.dpy,
                client.frame,
                &ChangeWindowAttributesAux::new().border_pixel(aux.theme.border_color_unfocused),
            )?;
        }
        let client = &mut self.clients[_client];
        self.focus_stack.remove_node(client.stack_pos);
        self.focused.replace(_client);
        client.stack_pos = self.focus_stack.push_front(_client);
        set_input_focus(&aux.dpy, InputFocus::PARENT, client.win, CURRENT_TIME)?;
        // focused window callback
        change_window_attributes(
            &aux.dpy,
            client.frame,
            &ChangeWindowAttributesAux::new().border_pixel(aux.theme.border_color_focused),
        )?;
        client.flags.psuedo_urgent = false;
        let name = client.name.clone();
        if self.psuedo_urgent.remove(&_client) {
            aux.hooks.update_tag(self);
        }
        self.set_active_window(name, &mut aux.hooks);
        Ok(())
    }

    pub fn apply_pos_size(
        &self,
        aux: &Aux,
        client: usize,
        size: &Rect,
        border: bool,
    ) -> Result<()> {
        let client = &self.clients[client];
        let conf_aux = size.aux(if border { client.border_width } else { 0 });
        configure_window(&aux.dpy, client.frame, &conf_aux)?;
        configure_window(&aux.dpy, client.win, &conf_aux.x(0).y(0).border_width(None))?;
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

    pub fn set_hidden(&mut self, aux: &mut Aux, client_: usize, arg: &SetArg<bool>) -> Result<()> {
        let client = &mut self.clients[client_];
        if arg.apply(&mut client.flags.hidden) {
            if client.flags.hidden {
                client.hide(aux, self.id)?;
                self.focus_stack.remove_node(client.stack_pos);
                self.set_focus(aux)?;
                self.hidden.push_back(client_);
                self.set_absent(aux, client_, true)?;
            } else {
                client.show(aux)?;
                client.stack_pos = self.focus_stack.push_back(client_);
                let absent = client.flags.absent();
                self.hidden.retain(|x| *x != client_);
                self.set_absent(aux, client_, absent)?;
            }
        }
        Ok(())
    }

    pub fn set_stack_layer(
        &mut self,
        aux: &Aux,
        client: usize,
        arg: &SetArg<StackLayer>,
    ) -> Result<()> {
        let (layer, last) = (self.clients[client].layer, self.clients[client].last_layer);
        if arg.apply_arg(&mut self.clients[client].layer, last) {
            self.clients[client].last_layer = layer;
            self.switch_layer(aux, client)?;
        }
        Ok(())
    }

    pub fn set_focus(&mut self, aux: &mut Aux) -> Result<()> {
        info!("tag {} setting focus", self.name);
        if let Some(client) = self.focus_stack.front().copied() {
            self.focus_client(aux, client)?;
        } else {
            set_input_focus(&aux.dpy, InputFocus::POINTER_ROOT, aux.root, CURRENT_TIME)?;
            self.set_active_window(None, &mut aux.hooks);
            self.focused.take();
        }
        Ok(())
    }

    pub fn unset_focus(&mut self, aux: &Aux) -> Result<()> {
        if let Some(client) = self.focused.take() {
            let client = &self.clients[client];
            change_window_attributes(
                &aux.dpy,
                client.frame,
                &ChangeWindowAttributesAux::new().border_pixel(aux.theme.border_color_unfocused),
            )?;
        }
        Ok(())
    }

    pub fn cycle(&mut self, aux: &mut Aux, rev: bool) -> Result<()> {
        if self.focus_stack.len() >= 2 {
            let client_ = if rev {
                *self.focus_stack.back().unwrap()
            } else {
                *self.focus_stack.front().unwrap()
            };
            let client = &mut self.clients[client_];
            self.focus_stack.remove_node(client.stack_pos);
            client.stack_pos = if rev {
                self.focus_stack.push_front(client_)
            } else {
                self.focus_stack.push_back(client_)
            };
            self.set_layer(aux, *self.focus_stack.front().unwrap(), true)?;
            self.set_focus(aux)?;
        }
        Ok(())
    }
}

impl WindowManager {
    pub fn remove_client(&mut self, tag: Atom, client: usize) -> Result<(Window, Window)> {
        let tag = self.tags.get_mut(&tag).unwrap();
        tag.urgent.remove(&client);
        tag.free_clients.insert(client);
        let (win, frame, node) = {
            let client = &mut tag.clients[client];
            let (layer, layer_pos) = client.layer_pos;
            tag.layers[layer].remove(layer_pos);
            if !client.flags.hidden {
                tag.focus_stack.remove_node(client.stack_pos);
            }
            client.flags.hidden = true;
            (client.win, client.frame, client.node)
        };
        if tag.id
            == self
                .monitors
                .get(&self.focused_monitor)
                .unwrap()
                .focused_tag
        {
            tag.set_focus(&mut self.aux)?;
        }
        self.windows.remove(&win);
        self.windows.remove(&frame);
        if node != 0 {
            tag.remove_node(&self.aux, node)?;
        } else {
            tag.nodes[0].info = NodeContents::Empty;
        }
        self.aux
            .selection
            .hide(&self.aux.dpy, Some(tag.id), Some(node))?;
        Ok((win, frame))
    }

    pub fn unmanage_client(&mut self, tag: Atom, client: usize) -> Result<()> {
        let (win, frame) = self.remove_client(tag, client)?;
        reparent_window(&self.aux.dpy, win, self.aux.root, 0, 0)?;
        destroy_window(&self.aux.dpy, frame)?;
        delete_property(&self.aux.dpy, win, self.aux.atoms.WM_STATE)?;
        delete_property(&self.aux.dpy, win, self.aux.atoms._NET_WM_STATE)?;
        self.aux
            .hooks
            .tag_update(&self.tags, &self.tag_order, self.focused_monitor);
        Ok(())
    }

    pub fn process_args(&mut self, win: Window, args: &mut ClientArgs) -> Result<()> {
        let state_cookie = get_property(
            &self.aux.dpy,
            false,
            win,
            self.aux.atoms._NET_WM_STATE,
            AtomEnum::ATOM,
            0,
            2048,
        )?;
        let hints_cookie = WmHints::get(&self.aux.dpy, win)?;
        let size_hints_cookie = WmSizeHints::get_normal_hints(&self.aux.dpy, win)?;
        let class_cookie = WmClass::get(&self.aux.dpy, win)?;
        let name_cookie = get_property(
            &self.aux.dpy,
            false,
            win,
            AtomEnum::WM_NAME,
            self.aux.atoms.UTF8_STRING,
            0,
            2048,
        )?;
        let wm_name_cookie = get_property(
            &self.aux.dpy,
            false,
            win,
            self.aux.atoms._NET_WM_NAME,
            self.aux.atoms.UTF8_STRING,
            0,
            2048,
        )?;
        let transient_cookie = get_property(
            &self.aux.dpy,
            false,
            win,
            AtomEnum::WM_TRANSIENT_FOR,
            AtomEnum::WINDOW,
            0,
            1,
        )?;
        let protocols_cookie = get_property(
            &self.aux.dpy,
            false,
            win,
            self.aux.atoms.WM_PROTOCOLS,
            AtomEnum::ATOM,
            0,
            32,
        )?;

        if let Ok(states) = state_cookie.reply() {
            if let Some(states) = states.value32() {
                for state in states {
                    args.process_state(&self.aux, state);
                }
            }
        }
        let _ = hints_cookie.reply().map(|hints| args.process_hints(hints));
        let _ = size_hints_cookie
            .reply()
            .map(|size_hints| args.prcoess_size_hints(size_hints));
        let _ = class_cookie.reply().map(|class| args.process_class(class));
        let _ = name_cookie
            .reply()
            .map(|name| args.process_name(name, false));
        let _ = wm_name_cookie
            .reply()
            .map(|name| args.process_name(name, true));
        let _ = transient_cookie
            .reply()
            .map(|transient| args.process_transient(transient));
        if let Ok(protocols) = protocols_cookie.reply() {
            if let Some(protocols) = protocols.value32() {
                for protocol in protocols {
                    args.process_protocol(&self.aux, protocol);
                }
            }
        }

        self.aux
            .rules
            .retain(|r| if args == r { !r.apply(args) } else { true });
        Ok(())
    }

    pub fn manage_client(&mut self, win: Window, args: ClientArgs) -> Result<()> {
        let ClientArgs {
            focus,
            flags,
            centered,
            managed: _,
            min_size,
            max_size,
            size,
            layer,
            class,
            instance,
            name,
            net_name,
            tag,
            mut pos,
            parent,
            protocols,
        } = args;
        let tag_idx = tag
            .and_then(|tag| self.tags.contains_key(&tag).then(|| tag))
            .unwrap_or_else(|| self.focused_tag());
        let tag = self.tags.get_mut(&tag_idx).unwrap();
        let border_width = self.aux.theme.border_width;
        let mut size = if let Some(size) = size {
            size
        } else {
            let rect = get_geometry(&self.aux.dpy, win)?.reply()?;
            pos.get_or_insert((rect.x, rect.y));
            (rect.width, rect.height)
        };
        size.0 += border_width * 2;
        size.1 += border_width * 2;
        let floating_rect = if centered || pos.is_none() {
            Rect::new(
                tag.tiling_size.x + (tag.tiling_size.width as i16 - size.0 as i16) / 2,
                tag.tiling_size.y + (tag.tiling_size.height as i16 - size.1 as i16) / 2,
                size.0,
                size.1,
            )
        } else {
            let mut pos = pos.unwrap();
            pos.0 -= border_width as i16;
            pos.1 -= border_width as i16;
            Rect::new(pos.0, pos.1, size.0, size.1)
        };

        let hidden = flags.hidden;
        let frame = self.aux.dpy.generate_id().unwrap();
        let client = Client {
            name,
            net_name,
            node: 0,
            class,
            instance,
            border_width,
            layer,
            last_layer: layer,
            stack_pos: 0,
            layer_pos: (0, 0),
            flags,
            win,
            frame,
            protocols,
            ignore_unmaps: 0,
        };

        info!("adding client {:?}", client);
        let info = NodeContents::leaf(0, min_size, max_size, floating_rect);

        info!("currennt node state {:?}, {:?}", tag.free_nodes, tag.nodes);
        let client = tag.add_client(&mut self.aux, client, parent, info, focus)?;

        let aux = CreateWindowAux::new()
            .event_mask(
                EventMask::ENTER_WINDOW
                    | EventMask::FOCUS_CHANGE
                    // | EventMask::SUBSTRUCTURE_REDIRECT
                    | EventMask::SUBSTRUCTURE_NOTIFY,
            )
            .colormap(self.aux.vis.colormap())
            .border_pixel(self.aux.theme.border_color_unfocused)
            .background_pixel(0);
        create_window(
            &self.aux.dpy,
            self.aux.vis.depth(),
            frame,
            self.aux.root,
            tag.tiling_size.x,
            tag.tiling_size.y,
            tag.tiling_size.width,
            tag.tiling_size.height,
            0,
            WindowClass::INPUT_OUTPUT,
            self.aux.vis.visualid(),
            &aux,
        )?;
        reparent_window(&self.aux.dpy, win, frame, 0, 0)?;
        change_window_attributes(
            &self.aux.dpy,
            win,
            &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?;

        tag.set_layer(&self.aux, client, focus)?;
        if let Some(client) = tag.clients.get_mut(client) {
            if hidden {
                client.hide(&mut self.aux, tag.id)?
            } else {
                client.show(&self.aux)?
            }
        }
        if !hidden && focus {
            tag.focus_client(&mut self.aux, client)?
        } else {
            change_window_attributes(
                &self.aux.dpy,
                frame,
                &ChangeWindowAttributesAux::new()
                    .border_pixel(self.aux.theme.border_color_unfocused),
            )?;
        }
        let tag = tag.id;
        self.ewmh_set_client_tag(client, tag)?;

        self.aux.dpy.flush()?;
        self.windows
            .insert(frame, WindowLocation::Client(tag, client));
        self.windows
            .insert(win, WindowLocation::Client(tag, client));
        self.aux
            .hooks
            .tag_update(&self.tags, &self.tag_order, self.focused_monitor);
        Ok(())
    }

    pub fn move_client(
        &mut self,
        tag: Atom,
        client: usize,
        set_dest: SetArg<Atom>,
    ) -> Result<usize> {
        let mut dest = tag;
        if let Some(mon) = self.tags.get(&tag).and_then(|tag| tag.monitor) {
            let prev = self.monitors.get(&mon).unwrap().prev_tag;
            set_dest.apply_arg(&mut dest, prev);
        } else {
            dest = set_dest.0;
        }
        if tag == dest {
            return Ok(client);
        }
        info!(
            "Moving client, src {}, dst: {}, client: {}",
            tag, dest, client
        );
        let (client_, mut info, focus, old_size, show) = {
            let hide = self.tags.get(&dest).unwrap().monitor.is_none();
            let tag = self.tags.get_mut(&tag).unwrap();
            let focus = Some(client) == tag.focused_client();
            let client = &mut tag.clients[client];
            if hide {
                client.hide(&mut self.aux, tag.id)?;
            }
            let node = &tag.nodes[client.node];
            (
                client.clone(),
                node.info.clone(),
                focus,
                tag.size.clone(),
                tag.monitor.is_none() && !hide,
            )
        };
        self.remove_client(tag, client)?;
        let hidden = client_.flags.hidden;
        let frame = client_.frame;
        let win = client_.win;
        let tag = self.tags.get_mut(&dest).unwrap();
        if let NodeContents::Leaf(leaf) = &mut info {
            leaf.floating.reposition(&old_size, &tag.size);
        }
        let client = tag.add_client(&mut self.aux, client_, None, info, focus)?;
        tag.set_layer(&self.aux, client, focus)?;
        if show {
            tag.clients[client].show(&self.aux)?;
            if !hidden
                && focus
                && tag.id
                    == self
                        .monitors
                        .get(&self.focused_monitor)
                        .unwrap()
                        .focused_tag
            {
                tag.focus_client(&mut self.aux, client)?
            } else {
                change_window_attributes(
                    &self.aux.dpy,
                    frame,
                    &ChangeWindowAttributesAux::new()
                        .border_pixel(self.aux.theme.border_color_unfocused),
                )?;
            }
        }
        let tag = tag.id;
        self.ewmh_set_client_tag(client, tag)?;
        self.aux.dpy.flush()?;
        self.windows
            .insert(frame, WindowLocation::Client(tag, client));
        self.windows
            .insert(win, WindowLocation::Client(tag, client));
        self.aux
            .hooks
            .tag_update(&self.tags, &self.tag_order, self.focused_monitor);
        Ok(client)
    }

    pub fn client_state(&mut self, tag: Atom, client_: usize, state: Atom, action: Atom) {
        let name = get_atom_name(&self.aux.dpy, state)
            .unwrap()
            .reply()
            .unwrap();
        info!("Client state, {}", String::from_utf8(name.name).unwrap());
        let tag = self.tags.get_mut(&tag).unwrap();
        let client = &mut tag.clients[client_];
        let arg = match action {
            0 => SetArg(false, false),
            1 => SetArg(true, false),
            2 => SetArg(false, true),
            _ => return
        };
        if state == self.aux.atoms._NET_WM_STATE_DEMANDS_ATTENTION && tag.focused != Some(client_) && arg.apply(&mut client.flags.psuedo_urgent) {
            if client.flags.psuedo_urgent {
                tag.psuedo_urgent.insert(client_)
            } else {
                tag.psuedo_urgent.remove(&client_)
            };
            self.aux
                .hooks
                .tag_update(&self.tags, &self.tag_order, self.focused_monitor)
        }
    }

    pub fn client_property(&mut self, tag: Atom, client_: usize, atom: Atom) {
        let tag = self.tags.get_mut(&tag).unwrap();
        let client = &mut tag.clients[client_];
        if !client.net_name && atom == AtomEnum::WM_NAME.into() {
            if let Some(name) = get_property(
                &self.aux.dpy,
                false,
                client.win,
                AtomEnum::WM_NAME,
                self.aux.atoms.UTF8_STRING,
                0,
                2048,
            )
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            {
                if name.length > 0 {
                    let name = String::from_utf8(name.value).unwrap();
                    client.name.replace(name.clone());
                    if tag.focus_stack.front() == Some(&client_) {
                        tag.set_active_window(Some(name), &mut self.aux.hooks)
                    }
                }
            }
        } else if atom == self.aux.atoms._NET_WM_NAME {
            if let Some(name) = get_property(
                &self.aux.dpy,
                false,
                client.win,
                self.aux.atoms._NET_WM_NAME,
                self.aux.atoms.UTF8_STRING,
                0,
                2048,
            )
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            {
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
            if let Some(hints) = WmHints::get(&self.aux.dpy, client.win)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
            {
                info!("HINTS: {:?}", hints);
                let changed = if hints.urgent {
                    tag.urgent.insert(client_)
                } else {
                    tag.urgent.remove(&client_)
                };
                if changed {
                    tag.clients[client_].flags.urgent = hints.urgent;
                    self.aux
                        .hooks
                        .tag_update(&self.tags, &self.tag_order, self.focused_monitor)
                }
            }
        }
    }

    pub fn ewmh_set_client_tag(&self, client: usize, tag: Atom) -> Result<()> {
        let tag = self.tags.get(&tag).unwrap();
        let client = &tag.clients[client];
        let mut tag_id = None;
        for (i, id) in self.tag_order.iter().enumerate() {
            if *id == tag.id {
                tag_id = Some(i);
                break;
            }
        }
        if let Some(id) = tag_id {
            self.aux.dpy.change_property32(
                PropMode::REPLACE,
                client.win,
                self.aux.atoms._NET_WM_DESKTOP,
                AtomEnum::CARDINAL,
                &[id as u32],
            )?;
        }
        Ok(())
    }
}
