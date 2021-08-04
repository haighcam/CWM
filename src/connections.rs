use x11rb::connection::Connection;
use x11rb::rust_connection::RustConnection;
use std::os::unix::io::{AsRawFd, RawFd};

use serde::{Serialize, Deserialize, de::DeserializeOwned};
use std::os::unix::net::{UnixStream, UnixListener};
use std::net::Shutdown;
use std::io::prelude::*;
use std::cell::RefCell;
use std::time::Duration;
use nix::poll::{poll, PollFd, PollFlags};

use super::WindowManager;
pub use crate::config::FlagArg;

pub struct Connections {
    pub dpy: RustConnection,
    listener: UnixListener,
    streams: Vec<RefCell<Stream>>,
    poll_fds: Vec<PollFd>
}

pub struct Stream {
    stream: UnixStream,
    length: usize,
    reading: bool,
    data: Vec<u8>
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ClientSelection {
    Client(u32),
    Focused
}

#[derive(Serialize, Deserialize, Debug)]
pub enum ClientRequest {
    MonitorFocus(u32),
    TagState,
    CloseWM,
    KillClient(ClientSelection),
    Fullscreen(ClientSelection, FlagArg),
    AlwaysOnTop(ClientSelection, FlagArg),
    Floating(ClientSelection, FlagArg),
    Sticky(ClientSelection, FlagArg)
}

#[derive(Serialize, Deserialize, Debug)]
pub struct TagState {
    name: String,
    focused: Option<usize>,
    urgent: bool,
    empty: bool
}

#[derive(Serialize, Deserialize, Debug)]
pub enum CWMResponse {
    FocusedClient(Option<String>),
    TagState(Vec<TagState>)
}

impl Drop for Connections {
    fn drop(&mut self) {
        let _ = std::fs::remove_file("/tmp/cwm.sock");
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

impl Connections {
    pub(crate) fn new(dpy: RustConnection) -> Self {
        let _ = std::fs::remove_file("/tmp/cwm.sock"); // possibly use this to check if it is already running??
        let listener = UnixListener::bind("/tmp/cwm.sock").unwrap();
        listener.set_nonblocking(true).expect("Couldn't set non blocking");

        let poll_fds = vec![
            PollFd::new(dpy.stream().as_raw_fd(), PollFlags::POLLIN), 
            PollFd::new(listener.as_raw_fd(), PollFlags::POLLIN)
        ];

        Self {
            dpy,
            listener,
            streams: Vec::new(),
            poll_fds
        }
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
            data: Vec::new()
        }
    }

    pub fn send<T: Serialize>(&mut self, item: &T) -> bool {
        let data = bincode::serialize(item).unwrap();
        self.stream.write_all(bincode::serialize(&(data.len() as u32)).unwrap().as_slice()).and(
            self.stream.write_all(data.as_slice())
        ).is_ok()
    }

    pub fn get_bytes(&mut self) -> bool {
        let mut bytes = [0u8; 256];
        match self.stream.read(&mut bytes) {
            Ok(0) => true,
            Ok(len) => {
                self.data.extend(&bytes[..len]);
                false
            },
            Err(e) => {
                println!("{:?}", e);
                e.kind() != std::io::ErrorKind::WouldBlock
            },
        }
    }
    
    pub fn recieve<T: DeserializeOwned>(&mut self, items: &mut Vec<T>) -> bool {
        let done = self.get_bytes();
        if !self.reading && self.data.len() >= 4 {
            self.length = bincode::deserialize::<u32>(self.data.drain(..4).as_ref()).unwrap() as usize;
            self.reading = true;
        }
        if self.reading && self.data.len() >= self.length {
            items.push(bincode::deserialize(self.data.drain(..self.length).as_ref()).unwrap());
            self.reading = false;
        }
        done
    }
}

impl WindowManager {
    pub(crate) fn handle_connections(&mut self) {
        if let Ok((stream, addr)) = self.conn.listener.accept() {
            stream.set_read_timeout(Some(Duration::from_nanos(100))).unwrap();
            stream.set_nonblocking(true).expect("Couldn't set non blocking");
            self.hooks.borrow_mut().add_monitor_focus(0, RefCell::new(Stream::new(stream)));
            //self.poll_fds.push(PollFd(stream.as_raw_fd(), PollFlags::POLLIN))
            //self.streams.push(Stream::new(stream)); 
            
        }
        //self.streams.retain(|stream| stream.borrow_mut().send_message("hello world"));
    }

    pub(crate) fn parse_tags(&self) -> String {
        String::new()
    }

    pub(crate) fn tags_updated(&self) {

    }
}

