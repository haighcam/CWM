use x11rb::connection::Connection;
use std::os::unix::net::{UnixStream, UnixListener};
use std::io::prelude::*;
use std::cell::RefCell;
use crate::config::WMCommand;
use super::WindowManager;

pub struct IPC {
    listener: UnixListener,
    streams: Vec<RefCell<UnixStream>>
}

impl Drop for IPC {
    fn drop(&mut self) {
        std::fs::remove_file("/tmp/cwm.sock").unwrap();
    }
}

impl IPC {
    pub fn new() -> Self {
        let _ = std::fs::remove_file("/tmp/cwm.sock"); // possibly use this to check if it is already running??
        let listener = UnixListener::bind("/tmp/cwm.sock").unwrap();
        listener.set_nonblocking(true).expect("Couldn't set non blocking");
        Self {
            listener,
            streams: Vec::new()
        }
    }

    pub fn update(&mut self, wm: &mut WindowManager<impl Connection>) -> Vec<WMCommand> {
        let mut commands = vec![];
        if let Ok((stream, addr)) = self.listener.accept() {
            self.streams.push(RefCell::new(stream));
        }
        self.streams.retain(|stream| handle_stream(&mut stream.borrow_mut()).is_ok());
        
        commands
    }
}

fn handle_stream(stream: &mut UnixStream) -> std::io::Result<()> {
    stream.write_all(b"hello world\n")?;
    Ok(())
}

impl<X: Connection> WindowManager<X> {
    pub fn parse_tags(&self) -> String {
        String::new()
    }

    pub fn tags_updated(&self) {

    }
}