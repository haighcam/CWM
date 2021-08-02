use std::os::unix::net::UnixStream;
use x11rb::connection::Connection;
use x11rb::protocol::randr::*;
use std::io::{self, prelude::*, BufReader};

fn main() -> std::io::Result<()> {
    let (dpy, screen) = x11rb::connect(None).unwrap();
    let root = dpy.setup().roots[screen].root;
    let monitors = get_monitors(&dpy, root, true).unwrap().reply().unwrap();
    //println!("{} {:?}", screen, monitors);
    
    //return Ok(());
    let mut stream = UnixStream::connect("/tmp/cwm.sock")?;
    let mut reader = BufReader::new(stream);
    let mut msg = String::new();
    loop {
        reader.read_to_line(&mut msg);
        print!("{}", msg);
        if msg == "\n" {
            break
        }
        msg.clear();
    }
    Ok(())
}
