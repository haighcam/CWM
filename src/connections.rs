use super::tag::{Side, StackLayer};
use anyhow::Result;
use log::info;
use nix::poll::{poll, PollFd, PollFlags};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::io::prelude::*;
use std::net::Shutdown;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::time::Duration;
use x11rb::rust_connection::RustConnection;

use super::{AtomCollection, WindowLocation, WindowManager};
pub use crate::config::Theme;
use crate::hooks::Hooks;

pub const SOCKET: &str = "/tmp/cwm.sock";

pub struct Aux {
    pub dpy: RustConnection,
    listener: UnixListener,
    streams: Vec<Stream>,
    poll_fds: Vec<PollFd>,
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
    Client(u32),
    All,
    First,
    Last,
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ClientRequest {
    MonitorFocus(u32),
    TagState,
    FocusedWindow,
    FocusedTag,
    FocusedMonitor,
    Quit,
    Reload,
    KillClient(Option<u32>),
    SetLayout(Option<u32>, SetArg<Layout>),
    SetLayer(Option<u32>, SetArg<StackLayer>),
    SetFullscreen(Option<u32>, SetArg<bool>),
    SetFloating(Option<u32>, SetArg<bool>),
    SetSticky(Option<u32>, SetArg<bool>),
    SetHidden(Option<u32>, SetArg<bool>),
    Show(HiddenSelection),
    ResizeWindow(Option<u32>, Side, i16), // +grow, -shrink
    MoveWindow(Option<u32>, Side, u16),   // floating move amnt, tiling swap neighbour
    CycleWindow(bool),
    CycleTag(bool),
    CycleMonitor(bool), //warp the pointer??
    FocusTag(Option<u32>, SetArg<u32>),
    FocusMonitor(SetArg<u32>),
    SetWindowTag(Option<u32>, SetArg<u32>),
    FocusWindow(u32), // Somekind of visual preselection?
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
    pub(crate) fn new(dpy: RustConnection) -> Result<Self> {
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
            Ok(0) => {
                info!("read zero");
                true
            }
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

    fn get_monitor(&self, mon: Option<u32>) -> u32 {
        mon.unwrap_or(self.focused_monitor)
    }

    fn get_tag(&self, mon: Option<u32>, tag: Option<u32>) -> Option<u32> {
        tag.or_else(|| {
            self.monitors
                .get(&mon.unwrap_or(self.focused_monitor))
                .map(|mon| mon.focused_tag)
        })
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
            KillClient(client) => {
                info!("Killing Client");
                if let Some((tag, client)) = self.get_client(client) {
                    self.unmanage_window(self.tags.get(&tag).unwrap().client(client).frame)?
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
                    self.tags.get_mut(&tag).unwrap().set_sticky(client, &arg)
                }
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
            }
            FocusedMonitor => {
                stream.send(&CwmResponse::FocusedMonitor(self.focused_monitor));
                self.aux.streams.push(stream);
                self.aux.poll_fds.push(poll_fd);
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
                x => info!("{:?}", x),
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
