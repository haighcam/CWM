use anyhow::Result;
use log::info;
use nix::poll::{poll, PollFd, PollFlags};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::prelude::*;
use std::net::Shutdown;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::Duration;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

use super::tag::{StackLayer};
use super::{AtomCollection, WindowLocation, WindowManager};
use crate::hooks::Hooks;

pub use crate::tag::Side;
pub use crate::config::Theme;

pub const SOCKET: &str = "/tmp/cwm.sock";

pub struct Aux {
    pub dpy: RustConnection,
    listener: UnixListener,
    streams: Vec<Stream>,
    poll_fds: Vec<PollFd>,
    pub root: u32,
    pub theme: Theme,
    pub hooks: Hooks,
    pub atoms: AtomCollection,
}

pub struct Stream {
    stream: UnixStream,
    length: usize,
    reading: bool,
    data: Vec<u8>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub enum Layout {
    Tiled,
    Monocle,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum HiddenSelection {
    All,
    First,
    Last
}

#[derive(Serialize, Deserialize, Debug)]
pub enum TagSelection {
    Name(String),
    Index(usize),
    Focused(Option<u32>),
    Id(u32),
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ClientRequest {
    MonitorFocus(u32),
    TagState,
    FocusedWindow(TagSelection),
    FocusedTag(Option<u32>),
    FocusedMonitor,
    Quit,
    Reload,
    CloseClient(Option<u32>, bool),
    SetLayout(Option<u32>, SetArg<Layout>),
    SetLayer(Option<u32>, SetArg<StackLayer>),
    SetFullscreen(Option<u32>, SetArg<bool>),
    SetFloating(Option<u32>, SetArg<bool>),
    SetSticky(Option<u32>, SetArg<bool>),
    SetHidden(Option<u32>, SetArg<bool>),
    Show(TagSelection, HiddenSelection),
    ResizeWindow(Option<u32>, Side, i16), // +grow, -shrink
    MoveWindow(Option<u32>, Side, u16),   // floating move amnt, tiling swap neighbour
    SelectNeighbour(Option<u32>, Side),   // select tiling neighbour
    CycleWindow(bool),
    CycleTag(bool),
    CycleMonitor(bool), //warp the pointer??
    FocusTag(Option<u32>, TagSelection, bool),
    FocusMonitor(SetArg<u32>),
    SetWindowTag(Option<u32>, TagSelection, bool),
    FocusWindow(u32), // Somekind of visual preselection?
    TagName(TagSelection),
    MonitorName(Option<u32>)
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
    Name(String)
}

impl Drop for Aux {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(SOCKET);
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
    pub(crate) fn new(dpy: RustConnection, root: u32) -> Result<Self> {
        let _ = std::fs::remove_file(SOCKET); // possibly use this to check if it is already running.
        let listener = UnixListener::bind(SOCKET).unwrap();
        listener
            .set_nonblocking(true)
            .expect("Couldn't set non blocking");

        let poll_fds = vec![
            PollFd::new(dpy.stream().as_raw_fd(), PollFlags::POLLIN),
            PollFd::new(listener.as_raw_fd(), PollFlags::POLLIN),
        ];

        let atoms = AtomCollection::new(&dpy)?.reply()?;

        Ok(Self {
            dpy,
            listener,
            streams: Vec::new(),
            poll_fds,
            root,
            theme: Theme::default(),
            hooks: Hooks::new(),
            atoms,
        })
    }

    pub(crate) fn wait_for_updates(&mut self) {
        poll(&mut self.poll_fds, -1).ok();
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
            TagSelection::Focused(mon) => {
                Ok(self.get_monitor(mon).map(|x| self.monitors.get(&x).unwrap().focused_tag))
            }
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
        use ClientRequest::*;
        info!("Request {:?}", request);
        match request {
            MonitorFocus(mon) => self.aux.hooks.add_monitor_focus(mon, stream),
            TagState => self.aux.hooks.add_monitor_tag(stream),
            CloseClient(client, kill) => {
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
            Quit => {
                self.running = false;
                info!("Exiting");
            }
            SetFullscreen(client, arg) => {
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
            SetLayer(client, arg) => {
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
            SetFloating(client, arg) => {
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
            SetSticky(client, arg) => {
                info!("Sticky {:?}", arg);
                if let Some((tag, client)) = self.get_client(client) {
                    self.set_sticky(tag, client, &arg);
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            SetHidden(client, arg) => {
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
            Show(tag, selection) => {
                info!("Show {:?}, {:?}", tag, selection);
                if let Some(tag) = self.get_tag(tag)? {
                    self.tags.get_mut(&tag).unwrap().show_clients(&mut self.aux, selection)?;
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            FocusedMonitor => {
                stream.send(&CwmResponse::FocusedMonitor(self.focused_monitor));
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            FocusedTag(mon) => {
                if let Some(mon) = self.get_monitor(mon) {
                    stream.send(&CwmResponse::FocusedTag(
                        self.monitors.get(&mon).unwrap().focused_tag,
                    ));
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            FocusedWindow(tag) => {
                if let Some(tag) = self.get_tag(tag)? {
                    let tag = self.tags.get(&tag).unwrap();
                    stream.send(&CwmResponse::FocusedWindow(
                        tag.focused_client().map(|x| tag.client(x).frame),
                    ));
                    self.aux.streams.push(stream);
                    self.aux.poll_fds.push(poll_fd);
                }
            }
            FocusTag(mon, tag, toggle) => {
                if let (Some(mon), Some(tag)) = (self.get_monitor(mon), self.get_tag(tag)?) {
                    self.switch_monitor_tag(mon, SetArg(tag, toggle))?;
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            SetWindowTag(client, tag, toggle) => {
                if let Some(dest) = self.get_tag(tag)? {
                    if let Some((tag, client)) = self.get_client(client) {
                        self.move_client(tag, client, SetArg(dest, toggle))?;
                    }
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            },
            CycleWindow(rev) => {
                let tag = self.focused_tag();
                let tag = self.tags.get_mut(&tag).unwrap();
                tag.cycle(&mut self.aux, rev)?;
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            },
            SelectNeighbour(client, side) => {
                if let Some((tag, client)) = self.get_client(client) {
                    let tag = self.tags.get_mut(&tag).unwrap();
                    if let Some(neighbour) = tag.get_neighbour(client, side) {
                        tag.focus_client(&mut self.aux, neighbour)?;
                    }
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            },
            MoveWindow(client, side, amt) => {
                if let Some((tag, client)) = self.get_client(client) {
                    let tag = self.tags.get_mut(&tag).unwrap();
                    tag.move_side(&self.aux, client, side, amt)?;
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            },
            ResizeWindow(client, side, amt) => {
                if let Some((tag, client)) = self.get_client(client) {
                    let tag = self.tags.get_mut(&tag).unwrap();
                    let delta = side.parse_amt(amt);
                    tag.resize_client(&self.aux, client, delta, side==Side::Left, side==Side::Top)?;
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            },
            MonitorName(mon) => {
                if let Some(mon) = self.get_monitor(mon) {
                    stream.send(&CwmResponse::Name(
                        self.monitors.get(&mon).unwrap().name.clone(),
                    ));
                }
            },
            TagName(tag) => {
                if let Some(tag) = self.get_tag(tag)? {
                    stream.send(&CwmResponse::Name(
                        self.tags.get(&tag).unwrap().name.clone(),
                    ));
                }
            }

            _ => (),
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
