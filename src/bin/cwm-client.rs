use x11rb::connection::Connection;
use x11rb::protocol::randr::*;
use std::os::unix::net::UnixStream;
use std::io::{self, prelude::*, BufReader, Error, ErrorKind};
use std::env::args;
use cwm::connections::{Stream, CWMResponse};

fn main() -> std::io::Result<()> {
    let mut args = args();
    args.next(); // skip file path
    
    match args.next() {
        None => return Err(Error::new(ErrorKind::InvalidInput, "No arguments given")),
        Some(arg) => return Err(Error::new(ErrorKind::InvalidInput, format!("Unknown argument or command: '{}'", arg)))
    }

    let (dpy, screen) = x11rb::connect(None).unwrap();
    let root = dpy.setup().roots[screen].root;
    let monitors = get_monitors(&dpy, root, true).unwrap().reply().unwrap();
    //println!("{} {:?}", screen, monitors);
    
    //return Ok(());
    let mut stream = Stream::new(UnixStream::connect("/tmp/cwm.sock")?);
    let mut responses: Vec<CWMResponse> = Vec::new();
    let mut done = false;
    let blank = String::new();
    while !done {
        done = stream.recieve(&mut responses);
        for response in responses.drain(..){
            if let CWMResponse::FocusedClient(client) = response {
                if let Some(client) = client {
                    println!("{}", client);
                } else {
                    println!();
                }
            }
        }
    }
    Ok(())
}
