use anyhow::Result;
use log::info;
use nix::poll::{poll, PollFd, PollFlags};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use std::io::prelude::*;
use std::net::Shutdown;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::render::*;
use x11rb::protocol::shape::{ConnectionExt, *};
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT};

use crate::hooks::Hooks;
use crate::tag::{NodeContents, Split, Tag};
use crate::utils::{mul_alpha, Rect};
use crate::{AtomCollection, WindowLocation, WindowManager};

pub use crate::config::Theme;
pub use crate::rules::Rule;
pub use crate::tag::{Side, StackLayer};

pub enum SelectionContent {
    Presel(Atom, usize, Presel),
    Node(Atom, usize),
    None,
}

pub struct Selection {
    pub win: Window,
    pub sel: SelectionContent,
}

impl Selection {
    fn new(dpy: &RustConnection, root: Window, vis: &VisualConfig) -> Result<Self> {
        let win = dpy.generate_id()?;
        create_window(
            dpy,
            vis.depth(),
            win,
            root,
            0,
            0,
            100,
            100,
            0,
            WindowClass::INPUT_OUTPUT,
            vis.visualid(),
            &CreateWindowAux::new()
                .colormap(vis.colormap())
                .background_pixel(mul_alpha(0x6600FF00))
                .border_pixel(0)
                .event_mask(EventMask::ENTER_WINDOW),
        )?;
        dpy.shape_rectangles(
            SO::SET,
            SK::INPUT,
            ClipOrdering::UNSORTED,
            win,
            0,
            0,
            &[Rectangle {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            }],
        )?;
        Ok(Self {
            win,
            sel: SelectionContent::None,
        })
    }

    pub fn presel(
        &mut self,
        dpy: &RustConnection,
        tag: Atom,
        node: usize,
    ) -> Result<Option<Presel>> {
        Ok(match &self.sel {
            SelectionContent::Presel(t, n, presel) if *t == tag && *n == node => {
                let presel = presel.clone();
                self.sel = SelectionContent::None;
                unmap_window(dpy, self.win)?;
                Some(presel)
            }
            _ => None,
        })
    }

    pub fn sel(&mut self) -> Option<(Atom, usize)> {
        if let SelectionContent::Node(tag, node) = &self.sel {
            Some((*tag, *node))
        } else {
            None
        }
    }

    pub fn show(&self, dpy: &RustConnection) -> Result<()> {
        map_window(dpy, self.win)?;
        Ok(())
    }

    pub fn hide(
        &mut self,
        dpy: &RustConnection,
        tag_: Option<Atom>,
        node_: Option<usize>,
    ) -> Result<()> {
        match &mut self.sel {
            SelectionContent::Presel(tag, node, ..) | SelectionContent::Node(tag, node) => {
                if tag_.map(|x| *tag == x).unwrap_or(true)
                    && node_.map(|x| *node == x).unwrap_or(true)
                {
                    self.sel = SelectionContent::None;
                    unmap_window(dpy, self.win)?;
                }
            }
            _ => (),
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct Presel {
    pub side: Side,
    pub amt: f32,
}

impl Default for Presel {
    fn default() -> Self {
        Self {
            side: Side::Right,
            amt: 0.5,
        }
    }
}

pub struct VisualConfig(Option<(u8, Visualid, Colormap, Pictformat)>);

impl VisualConfig {
    pub fn new(dpy: &RustConnection, root: Window, screen: usize) -> Result<Self> {
        let info = query_pict_formats(dpy)?.reply()?;
        let formats: HashMap<_, _> = info.formats.iter().map(|x| (x.id, x)).collect();
        for Pictdepth { depth, visuals } in &info.screens[screen].depths {
            for visual in visuals {
                if let Some(format) = formats.get(&visual.format) {
                    if format.type_ == PictType::DIRECT && format.direct.alpha_mask == 0xFF {
                        let colormap = dpy.generate_id()?;
                        create_colormap(dpy, ColormapAlloc::NONE, colormap, root, visual.visual)?;
                        info!("depth found {}, {:?}, {:X}", depth, format, visual.visual);
                        return Ok(Self(Some((*depth, visual.visual, colormap, visual.format))));
                    }
                }
            }
        }
        Ok(Self(None))
    }

    pub fn depth(&self) -> u8 {
        self.0.map(|x| x.0).unwrap_or(COPY_DEPTH_FROM_PARENT)
    }

    pub fn visualid(&self) -> Visualid {
        self.0.map(|x| x.1).unwrap_or(COPY_FROM_PARENT)
    }

    pub fn colormap(&self) -> Option<Colormap> {
        self.0.map(|x| x.2)
    }
}

pub struct Aux {
    pub dpy: RustConnection,
    listener: UnixListener,
    streams: Vec<Stream>,
    poll_fds: Vec<PollFd>,
    socket: String,
    pub root: u32,
    pub theme: Theme,
    pub hooks: Hooks,
    pub atoms: AtomCollection,
    pub rules: Vec<Rule>,
    pub vis: VisualConfig,
    pub selection: Selection,
}

pub struct Stream {
    stream: UnixStream,
    length: usize,
    reading: bool,
    data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum HiddenSelection {
    All,
    First,
    Last,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum TagSelection {
    Name(String),
    Index(usize),
    Focused(Option<u32>),
    Next(Option<u32>),
    Prev(Option<u32>),
    Last(Option<u32>),
    Id(u32),
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ClientRequest {
    MonitorFocus(Option<u32>),
    TagState,
    FocusedWindow(TagSelection),
    FocusedTag(Option<u32>),
    FocusedMonitor,
    Quit,
    Reload,
    CloseClient(Option<u32>, bool),
    SetLayer(Option<u32>, SetArg<StackLayer>),
    SetFullscreen(Option<u32>, SetArg<bool>),
    SetFloating(Option<u32>, SetArg<bool>),
    SetSticky(Option<u32>, SetArg<bool>),
    SetHidden(Option<u32>, SetArg<bool>),
    SetMonocle(TagSelection, SetArg<bool>),
    Show(TagSelection, HiddenSelection),
    ResizeWindow(Option<u32>, Side, i16), // +grow, -shrink
    MoveWindow(Option<u32>, Side, u16),   // floating move amnt, tiling swap neighbour
    SelectNeighbour(Option<u32>, Side),   // select tiling neighbour
    CycleWindow(bool),
    FocusTag(Option<u32>, TagSelection, bool),
    SetWindowTag(Option<u32>, TagSelection, bool),
    TagName(TagSelection),
    MonitorName(Option<u32>),
    ConfigBorderFocused(u32),
    ConfigBorderUnfocused(u32),
    ConfigBorderWidth(u16),
    ConfigGap(u16),
    ConfigMargin(Side, i16),
    AddRule(Rule),
    AddTag(String),
    RemoveTag(TagSelection),
    Select(Option<u32>),
    SelectDir(Side),
    SelectParent,
    PreselAmt(f32),
    SelectionCancel,
    Rotate(bool),
    ViewLayers(TagSelection),
    ViewStack(TagSelection),
    ViewClients(TagSelection),
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct TagState {
    pub name: String,
    pub focused: Option<u32>,
    pub urgent: bool,
    pub empty: bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum CwmResponse {
    MonitorFocusedClient(Option<String>),
    TagState(Vec<TagState>, u32),
    FocusedMonitor(u32),
    FocusedTag(u32),
    FocusedWindow(Option<u32>),
    Name(String),
    ViewLayers(Vec<Vec<usize>>),
    ViewStack(Vec<usize>),
    ViewClients(Vec<(usize, u32, Option<String>)>),
}

impl Drop for Aux {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

impl AsRawFd for Stream {
    fn as_raw_fd(&self) -> i32 {
        self.stream.as_raw_fd()
    }
}

impl Aux {
    pub(crate) fn new(dpy: RustConnection, root: u32, screen: usize) -> Result<Self> {
        let socket = format!("/tmp/cwm-{}.sock", whoami::username());
        let _ = std::fs::remove_file(&socket); // possibly use this to check if it is already running.
        let listener = UnixListener::bind(&socket).unwrap();
        listener
            .set_nonblocking(true)
            .expect("Couldn't set non blocking");

        let poll_fds = vec![
            PollFd::new(dpy.stream().as_raw_fd(), PollFlags::POLLIN),
            PollFd::new(listener.as_raw_fd(), PollFlags::POLLIN),
        ];

        let atoms = AtomCollection::new(&dpy)?.reply()?;
        let vis = VisualConfig::new(&dpy, root, screen)?;
        let selection = Selection::new(&dpy, root, &vis)?;

        dpy.change_property32(
            PropMode::APPEND,
            root,
            atoms._NET_SUPPORTED,
            AtomEnum::ATOM,
            &[
                atoms._NET_WM_STATE,
                atoms._NET_WM_STATE_FULLSCREEN,
                atoms._NET_WM_STATE_DEMANDS_ATTENTION,
                atoms._NET_ACTIVE_WINDOW,
            ],
        )?;

        Ok(Self {
            dpy,
            listener,
            streams: Vec::new(),
            poll_fds,
            socket,
            root,
            theme: Theme::default(),
            hooks: Hooks::new(),
            atoms,
            rules: Vec::new(),
            vis,
            selection,
        })
    }

    pub(crate) fn wait_for_updates(&mut self) {
        poll(&mut self.poll_fds, -1).ok();
    }

    pub fn resize_selection(&mut self, tag: &Tag) -> Result<()> {
        let (mut r1, mut r2) = (Rect::default(), Rect::default());
        if let Some((rect, attr)) = match &self.selection.sel {
            SelectionContent::Presel(tag_, node, Presel { side, amt }) => {
                (*tag_ == tag.id).then(|| {
                    let size = tag.get_node_rect(*node);
                    let (split, first) = side.get_split();
                    size.split(&split, *amt, &mut r1, &mut r2, 0);
                    (
                        if first { &r1 } else { &r2 },
                        ChangeWindowAttributesAux::new().background_pixel(self.theme.presel_color),
                    )
                })
            }
            SelectionContent::Node(tag_, node) => (*tag_ == tag.id).then(|| {
                (
                    tag.get_node_rect(*node),
                    ChangeWindowAttributesAux::new().background_pixel(self.theme.sel_color),
                )
            }),
            _ => None,
        } {
            configure_window(
                &self.dpy,
                self.selection.win,
                &rect
                    .aux(self.theme.selection_gap)
                    .stack_mode(StackMode::ABOVE),
            )?;
            change_window_attributes(&self.dpy, self.selection.win, &attr)?;
            clear_area(
                &self.dpy,
                false,
                self.selection.win,
                0,
                0,
                rect.width,
                rect.height,
            )?;
        }
        Ok(())
    }
}

impl Stream {
    pub fn new(stream: UnixStream) -> Self {
        Self {
            stream,
            length: 0,
            reading: false,
            data: Vec::new(),
        }
    }

    pub fn send<T: Serialize>(&mut self, item: &T) -> bool {
        let data = bincode::serialize(item).unwrap();
        match self
            .stream
            .write_all(bincode::serialize(&(data.len() as u32)).unwrap().as_slice())
            .and(self.stream.write_all(data.as_slice()))
        {
            Ok(_) => true,
            Err(e) => {
                info!("{:?}", e);
                false
            }
        }
    }

    pub fn get_bytes(&mut self) -> bool {
        let mut bytes = [0u8; 256];
        match self.stream.read(&mut bytes) {
            Ok(0) => true,
            Ok(len) => {
                self.data.extend(&bytes[..len]);
                false
            }
            Err(e) => {
                info!("{:?}", e);
                e.kind() != std::io::ErrorKind::WouldBlock
            }
        }
    }

    pub fn recieve<T: DeserializeOwned>(&mut self) -> (bool, Option<T>) {
        let done = self.get_bytes();
        if !self.reading && self.data.len() >= 4 {
            self.length =
                bincode::deserialize::<u32>(self.data.drain(..4).as_ref()).unwrap() as usize;
            self.reading = true;
        }
        if self.reading && self.data.len() >= self.length {
            self.reading = false;
            (
                done,
                Some(bincode::deserialize(self.data.drain(..self.length).as_ref()).unwrap()),
            )
        } else {
            (done, None)
        }
    }
}

impl WindowManager {
    fn get_client(&self, client: Option<u32>) -> Option<(u32, usize)> {
        if let Some(client) = client {
            if let Some(WindowLocation::Client(tag, id)) = self.windows.get(&client) {
                Some((*tag, *id))
            } else {
                None
            }
        } else {
            self.tags
                .get(&self.focused_tag())
                .unwrap()
                .focused_client()
                .map(|client| (self.focused_tag(), client))
        }
    }

    fn get_monitor(&self, mon: Option<u32>) -> Option<u32> {
        if let Some(mon) = mon {
            if self.monitors.contains_key(&mon) {
                Some(mon)
            } else {
                None
            }
        } else {
            Some(self.focused_monitor)
        }
    }

    fn get_tag(&self, tag: TagSelection) -> Result<Option<u32>> {
        match tag {
            TagSelection::Index(idx) => Ok(self.tag_order.get(idx).copied()),
            TagSelection::Name(name) => {
                let id = intern_atom(&self.aux.dpy, false, name.as_ref())?
                    .reply()?
                    .atom;
                if self.tags.contains_key(&id) {
                    Ok(Some(id))
                } else {
                    Ok(None)
                }
            }
            TagSelection::Focused(mon) => Ok(self
                .get_monitor(mon)
                .map(|x| self.monitors.get(&x).unwrap().focused_tag)),
            TagSelection::Next(mon) => {
                if let Some(mon) = self.get_monitor(mon) {
                    let mon = self.monitors.get(&mon).unwrap();
                    for (idx, id) in self.tag_order.iter().enumerate() {
                        if *id == mon.focused_tag {
                            return self
                                .get_tag(TagSelection::Index((idx + 1) % self.tag_order.len()));
                        }
                    }
                }
                Ok(None)
            }
            TagSelection::Prev(mon) => {
                if let Some(mon) = self.get_monitor(mon) {
                    let mon = self.monitors.get(&mon).unwrap();
                    for (idx, id) in self.tag_order.iter().enumerate() {
                        if *id == mon.focused_tag {
                            return self.get_tag(TagSelection::Index(
                                (idx + self.tag_order.len() - 1) % self.tag_order.len(),
                            ));
                        }
                    }
                }
                Ok(None)
            }
            TagSelection::Last(mon) => Ok(self
                .get_monitor(mon)
                .map(|x| self.monitors.get(&x).unwrap().prev_tag)),
            TagSelection::Id(tag) => Ok(if self.tags.contains_key(&tag) {
                Some(tag)
            } else {
                None
            }),
        }
    }

    fn handle_request(
        &mut self,
        mut stream: Stream,
        poll_fd: PollFd,
        request: ClientRequest,
    ) -> Result<()> {
        info!("Request {:?}", request);
        match request {
            ClientRequest::MonitorFocus(mon) => {
                if let Some(mon) = self.get_monitor(mon) {
                    self.aux.hooks.add_monitor_focus(mon, stream)
                }
            }
            ClientRequest::TagState => self.aux.hooks.add_monitor_tag(stream),
            ClientRequest::CloseClient(client, kill) => {
                info!("Killing Client");
                if let Some((tag, client)) = self.get_client(client) {
                    self.tags
                        .get(&tag)
                        .unwrap()
                        .client(client)
                        .close(&self.aux, kill)?
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::Quit => {
                self.running = false;
                info!("Exiting");
            }
            ClientRequest::Reload => {
                for mon in self.monitors.values() {
                    self.aux.hooks.mon_close(mon.id, mon.name.as_str());
                }
                self.aux.hooks.config();
                for mon in self.monitors.values() {
                    self.aux.hooks.mon_open(mon.id, mon.name.as_str(), mon.bg);
                }
            }
            ClientRequest::SetFullscreen(client, arg) => {
                info!("Fullscreen {:?}", arg);
                if let Some((tag, client)) = self.get_client(client) {
                    self.tags
                        .get_mut(&tag)
                        .unwrap()
                        .set_fullscreen(&self.aux, client, &arg)?
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::SetLayer(client, arg) => {
                info!("SetLayer {:?}", arg);
                if let Some((tag, client)) = self.get_client(client) {
                    self.tags
                        .get_mut(&tag)
                        .unwrap()
                        .set_stack_layer(&self.aux, client, &arg)?
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::SetFloating(client, arg) => {
                info!("Floating {:?}", arg);
                if let Some((tag, client)) = self.get_client(client) {
                    self.tags
                        .get_mut(&tag)
                        .unwrap()
                        .set_floating(&self.aux, client, &arg)?
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::SetSticky(client, arg) => {
                info!("Sticky {:?}", arg);
                if let Some((tag, client)) = self.get_client(client) {
                    self.set_sticky(tag, client, &arg);
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::SetHidden(client, arg) => {
                info!("Hidden {:?}", arg);
                if let Some((tag, client)) = self.get_client(client) {
                    self.tags
                        .get_mut(&tag)
                        .unwrap()
                        .set_hidden(&mut self.aux, client, &arg)?
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::SetMonocle(tag, arg) => {
                info!("Monocle {:?}", arg);
                if let Some(tag) = self.get_tag(tag)? {
                    self.tags
                        .get_mut(&tag)
                        .unwrap()
                        .set_monocle(&self.aux, &arg)?;
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::Show(tag, selection) => {
                info!("Show {:?}, {:?}", tag, selection);
                if let Some(tag) = self.get_tag(tag)? {
                    self.tags
                        .get_mut(&tag)
                        .unwrap()
                        .show_clients(&mut self.aux, selection)?;
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::FocusedMonitor => {
                stream.send(&CwmResponse::FocusedMonitor(self.focused_monitor));
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::FocusedTag(mon) => {
                if let Some(mon) = self.get_monitor(mon) {
                    stream.send(&CwmResponse::FocusedTag(
                        self.monitors.get(&mon).unwrap().focused_tag,
                    ));
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::FocusedWindow(tag) => {
                if let Some(tag) = self.get_tag(tag)? {
                    let tag = self.tags.get(&tag).unwrap();
                    stream.send(&CwmResponse::FocusedWindow(
                        tag.focused_client().map(|x| tag.client(x).win),
                    ));
                    self.aux.streams.push(stream);
                    self.aux.poll_fds.push(poll_fd);
                }
            }
            ClientRequest::FocusTag(mon, tag, toggle) => {
                if let (Some(mon), Some(tag)) = (self.get_monitor(mon), self.get_tag(tag)?) {
                    self.switch_monitor_tag(mon, SetArg(tag, toggle))?;
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::SetWindowTag(client, tag, toggle) => {
                if let Some(dest) = self.get_tag(tag)? {
                    if let Some((tag, client)) = self.get_client(client) {
                        self.move_client(tag, client, SetArg(dest, toggle))?;
                    }
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::CycleWindow(rev) => {
                let tag = self.focused_tag();
                let tag = self.tags.get_mut(&tag).unwrap();
                tag.cycle(&mut self.aux, rev)?;
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::SelectNeighbour(client, side) => {
                if let Some((tag, client)) = self.get_client(client) {
                    let tag = self.tags.get_mut(&tag).unwrap();
                    if let Some(neighbour) = tag.get_neighbour(client, side) {
                        tag.focus_client(&mut self.aux, neighbour)?;
                    }
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::MoveWindow(client, side, amt) => {
                if let Some((tag, client)) = self.get_client(client) {
                    let tag = self.tags.get_mut(&tag).unwrap();
                    tag.move_side(&self.aux, client, side, amt)?;
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::ResizeWindow(client, side, amt) => {
                if let Some((tag, client)) = self.get_client(client) {
                    let tag = self.tags.get_mut(&tag).unwrap();
                    let delta = side.parse_amt(amt);
                    tag.resize_client(
                        &mut self.aux,
                        client,
                        delta,
                        side == Side::Left,
                        side == Side::Top,
                    )?;
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::MonitorName(mon) => {
                if let Some(mon) = self.get_monitor(mon) {
                    stream.send(&CwmResponse::Name(
                        self.monitors.get(&mon).unwrap().name.clone(),
                    ));
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::TagName(tag) => {
                if let Some(tag) = self.get_tag(tag)? {
                    stream.send(&CwmResponse::Name(
                        self.tags.get(&tag).unwrap().name.clone(),
                    ));
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::ConfigBorderFocused(color) => {
                self.aux.theme.border_color_focused = mul_alpha(color);
                for mon in self.monitors.values() {
                    let tag = self.tags.get(&mon.focused_tag).unwrap();
                    if let Some(client) = tag.focused_client() {
                        let client = tag.client(client);
                        change_window_attributes(
                            &self.aux.dpy,
                            client.win,
                            &ChangeWindowAttributesAux::new()
                                .border_pixel(self.aux.theme.border_color_focused),
                        )?;
                    }
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::ConfigBorderUnfocused(color) => {
                self.aux.theme.border_color_unfocused = mul_alpha(color);
                for tag in self.tags.values() {
                    let focused = tag.focused_client();
                    for (id, client) in tag.clients().iter().enumerate() {
                        if Some(id) != focused {
                            change_window_attributes(
                                &self.aux.dpy,
                                client.win,
                                &ChangeWindowAttributesAux::new()
                                    .border_pixel(self.aux.theme.border_color_unfocused),
                            )?;
                        }
                    }
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::ConfigBorderWidth(width) => {
                for tag in self.tags.values_mut() {
                    for client in tag.clients_mut() {
                        if client.border_width == self.aux.theme.border_width {
                            client.border_width = width;
                        }
                    }
                }
                self.aux.theme.border_width = width;
                for mon in self.monitors.values() {
                    let tag = self.tags.get_mut(&mon.focused_tag).unwrap();
                    tag.resize_all(&self.aux, &mon.free_rect(), &mon.size)?;
                }

                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::ConfigGap(gap) => {
                self.aux.theme.gap = gap;
                for mon in self.monitors.values() {
                    let tag = self.tags.get_mut(&mon.focused_tag).unwrap();
                    tag.set_tiling_size(&self.aux, mon.free_rect())?;
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::ConfigMargin(side, marg) => {
                match side {
                    Side::Left => self.aux.theme.left_margin = marg,
                    Side::Right => self.aux.theme.right_margin = marg,
                    Side::Top => self.aux.theme.top_margin = marg,
                    Side::Bottom => self.aux.theme.bottom_margin = marg,
                }
                for mon in self.monitors.values() {
                    let tag = self.tags.get_mut(&mon.focused_tag).unwrap();
                    tag.set_tiling_size(&self.aux, mon.free_rect())?;
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::AddRule(rule) => {
                self.aux.rules.push(rule);
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::AddTag(tag) => {
                self.add_tag(tag)?;
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::RemoveTag(tag) => {
                if let Some(tag) = self.get_tag(tag)? {
                    self.remove_tag(tag)?;
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::Select(client) => {
                if let Some((tag, client)) = self.get_client(client) {
                    let tag = self.tags.get(&tag).unwrap();
                    self.aux.selection.sel =
                        SelectionContent::Node(tag.id, tag.client(client).node);
                    self.aux.resize_selection(tag)?;
                    self.aux.selection.show(&self.aux.dpy)?;
                }
            }
            ClientRequest::SelectParent => {
                if let Some((tag, node)) = match &mut self.aux.selection.sel {
                    SelectionContent::Node(tag, node) | SelectionContent::Presel(tag, node, ..) => {
                        Some((*tag, *node))
                    }
                    SelectionContent::None => self.get_client(None).map(|(tag, client)| {
                        (tag, self.tags.get(&tag).unwrap().client(client).node)
                    }),
                } {
                    let node_ = &self.tags.get(&tag).unwrap().node(node);
                    if let Some((node, _)) = node_.parent {
                        self.aux.selection.sel = SelectionContent::Node(tag, node);
                        let tag = self.tags.get(&tag).unwrap();
                        self.aux.resize_selection(tag)?;
                        self.aux.selection.show(&self.aux.dpy)?;
                    }
                }
            }
            ClientRequest::SelectDir(side) => match &mut self.aux.selection.sel {
                SelectionContent::Node(tag, node) => {
                    let tag = *tag;
                    let node_ = &self.tags.get(&tag).unwrap().node(*node);
                    match &node_.info {
                        NodeContents::Node(node_) => {
                            if let Some(node_) = match (&node_.split, side) {
                                (Split::Vertical, Side::Left) => Some(node_.first_child),
                                (Split::Vertical, Side::Right) => Some(node_.second_child),
                                (Split::Horizontal, Side::Top) => Some(node_.first_child),
                                (Split::Horizontal, Side::Bottom) => Some(node_.second_child),
                                _ => None,
                            } {
                                *node = node_;
                            }
                        }
                        NodeContents::Leaf(..) => {
                            let node = *node;
                            self.aux.selection.sel =
                                SelectionContent::Presel(tag, node, Presel { side, amt: 0.5 })
                        }
                        _ => (),
                    }
                    let tag = self.tags.get(&tag).unwrap();
                    self.aux.resize_selection(tag)?;
                    self.aux.selection.show(&self.aux.dpy)?;
                }
                SelectionContent::Presel(tag, _, presel) => {
                    presel.side = side;
                    let tag = self.tags.get(tag).unwrap();
                    self.aux.resize_selection(tag)?;
                    self.aux.selection.show(&self.aux.dpy)?;
                }
                _ => {
                    if let Some((tag, client)) = self.get_client(None) {
                        let tag = self.tags.get(&tag).unwrap();
                        self.aux.selection.sel = SelectionContent::Presel(
                            tag.id,
                            tag.client(client).node,
                            Presel { side, amt: 0.5 },
                        );
                        self.aux.resize_selection(tag)?;
                        self.aux.selection.show(&self.aux.dpy)?;
                    }
                }
            },
            ClientRequest::PreselAmt(amt_) => {
                if let Some(tag) = if let SelectionContent::Presel(tag, _, Presel { side, amt }) =
                    &mut self.aux.selection.sel
                {
                    *amt = (if side.get_split().1 {
                        *amt + amt_
                    } else {
                        *amt - amt_
                    })
                    .min(Side::MAX)
                    .max(Side::MIN);
                    Some(self.tags.get(tag).unwrap())
                } else {
                    None
                } {
                    self.aux.resize_selection(tag)?;
                }
            }
            ClientRequest::SelectionCancel => {
                self.aux.selection.hide(&self.aux.dpy, None, None)?;
            }
            ClientRequest::Rotate(rev) => {
                if let SelectionContent::Node(tag, node) = &self.aux.selection.sel {
                    self.tags
                        .get_mut(tag)
                        .unwrap()
                        .rotate(&self.aux, *node, rev)?;
                } else if let Some(tag) = self.get_tag(TagSelection::Focused(None))? {
                    self.tags.get_mut(&tag).unwrap().rotate(&self.aux, 0, rev)?;
                }
            }
            ClientRequest::ViewLayers(tag) => {
                if let Some(tag) = self.get_tag(tag)? {
                    stream.send(&CwmResponse::ViewLayers(
                        self.tags.get_mut(&tag).unwrap().get_layers(),
                    ));
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::ViewStack(tag) => {
                if let Some(tag) = self.get_tag(tag)? {
                    stream.send(&CwmResponse::ViewStack(
                        self.tags.get_mut(&tag).unwrap().get_stack(),
                    ));
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            ClientRequest::ViewClients(tag) => {
                if let Some(tag) = self.get_tag(tag)? {
                    stream.send(&CwmResponse::ViewClients(
                        self.tags.get_mut(&tag).unwrap().get_clients(),
                    ));
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
        }
        Ok(())
    }

    pub(crate) fn handle_connections(&mut self) -> Result<()> {
        if let Ok((stream, _)) = self.aux.listener.accept() {
            stream
                .set_read_timeout(Some(Duration::from_nanos(100)))
                .unwrap();
            stream
                .set_nonblocking(true)
                .expect("Couldn't set non blocking");
            self.aux
                .poll_fds
                .push(PollFd::new(stream.as_raw_fd(), PollFlags::POLLIN));
            self.aux.streams.push(Stream::new(stream));
        }
        for (mut stream, poll_fd) in self
            .aux
            .streams
            .drain(..)
            .zip(self.aux.poll_fds.drain(2..))
            .collect::<Vec<_>>()
        {
            match stream.recieve() {
                (false, None) => {
                    self.aux.streams.push(stream);
                    self.aux.poll_fds.push(poll_fd);
                }
                (false, Some(request)) => self.handle_request(stream, poll_fd, request)?,
                _ => (),
            }
        }
        Ok(())
    }
}

impl TagState {
    pub fn format(&self, curr_mon: u32, focused_mon: u32) -> String {
        let prefix = match self {
            Self { urgent: true, .. } => "!",
            Self {
                focused: Some(mon), ..
            } if *mon == curr_mon && *mon == focused_mon => "#",
            Self {
                focused: Some(mon), ..
            } if *mon == curr_mon => "+",
            Self {
                focused: Some(mon), ..
            } if *mon == focused_mon => "%",
            Self {
                focused: Some(_), ..
            } => "-",
            Self { empty: false, .. } => ":",
            _ => ".",
        };
        prefix.to_string() + self.name.as_str()
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SetArg<T: PartialEq + Clone>(pub T, pub bool);

impl<T: PartialEq + Clone> SetArg<T> {
    pub fn apply_arg(&self, arg: &mut T, last: T) -> bool {
        if *arg != self.0 {
            *arg = self.0.clone();
            true
        } else if self.1 && last != self.0 {
            *arg = last;
            true
        } else {
            false
        }
    }
}

impl SetArg<bool> {
    pub fn apply(&self, arg: &mut bool) -> bool {
        if *arg != self.0 {
            *arg = self.0;
            true
        } else if self.1 {
            *arg ^= true;
            true
        } else {
            false
        }
    }
}
