use anyhow::{bail, Context, Result};
use cwm::connections::{ClientRequest, CwmResponse, SetArg, Stream, TagSelection, SOCKET};
use nix::poll::{poll, PollFd, PollFlags};
use simplelog::*;
use std::env::{args, Args};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;

fn get_mon(args: &mut Args) -> Result<Option<u32>> {
    match args.next() {
        Some(item) => match item.as_str() {
            "-f" => Ok(None), // change to query focused monitor
            item => {
                if let Ok(node) = item.parse() {
                    Ok(Some(node))
                } else {
                    bail!("invalid node id '{}'", item)
                }
            }
        },
        _ => bail!("node id or -f must be specified"),
    }
}

fn get_tag_(args: &mut Args, allow_toggle: bool) -> Result<(bool, String)> {
    if let Some(mut item) = args.next() {
        let mut start = 0;
        let mut toggle = false;
        if allow_toggle && &item[0..1] == "~" {
            toggle = true;
            start += 1;
        }
        Ok((toggle, item.split_off(start)))
    } else {
        bail!("mon: No argument provided")
    }
}

pub fn get_tag(args: &mut Args, allow_toggle: bool) -> Result<(TagSelection, bool)> {
    match args.next() {
        Some(item) => match (item.as_str(), allow_toggle) {
            ("index", _) => {
                let (toggle, idx) = get_tag_(args, allow_toggle)?;
                Ok((TagSelection::Index(idx.parse()?), toggle))
            }
            ("name", _) => {
                let (toggle, name) = get_tag_(args, allow_toggle)?;
                Ok((TagSelection::Name(name), toggle))
            }
            ("~focused", true) => Ok((TagSelection::Focused(None), true)),
            ("focused", _) => Ok((TagSelection::Focused(None), false)),
            _ => bail!("mon: unknown argument '{}'", item),
        },
        _ => bail!("mon: No arguments provided"),
    }
}

mod node {
    use super::*;
    #[derive(Default)]
    struct NodeFlags {
        hidden: Option<SetArg<bool>>,
        floating: Option<SetArg<bool>>,
        fullscreen: Option<SetArg<bool>>,
        sticky: Option<SetArg<bool>>,
    }

    pub(super) fn process(mut args: Args, stream: ClientStream) -> Result<()> {
        match args.next() {
            Some(item) => match item.as_str() {
                "set" => set(args, stream),
                "kill" => kill(args, stream),
                "close" => close(args, stream),
                "move" => move_(args, stream),
                _ => bail!("subscribe: unknown argument '{}'", item),
            },
            _ => bail!("subscribe: No arguments provided"),
        }
    }

    fn get_node(args: &mut Args) -> Result<Option<u32>> {
        match args.next() {
            Some(item) => match item.as_str() {
                "-f" => Ok(None), // change to query focused monitor
                item => {
                    if let Ok(node) = item.parse() {
                        Ok(Some(node))
                    } else {
                        bail!("invalid node id '{}'", item)
                    }
                }
            },
            _ => bail!("node id or -f must be specified"),
        }
    }

    fn set(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let node = get_node(&mut args)?;
        let mut flags = NodeFlags::default();
        if let Some(item) = args.next() {
            for item in item.split('.') {
                let mut start = 0;
                let mut toggle = false;
                let mut set = true;
                if &item[0..1] == "~" {
                    toggle = true;
                    start += 1;
                } else if &item[0..1] == "!" {
                    set = false;
                    start += 1;
                }
                match &item[start..] {
                    "hidden" => flags.hidden = flags.hidden.or(Some(SetArg(set, toggle))),
                    "floating" => flags.floating = flags.floating.or(Some(SetArg(set, toggle))),
                    "fullscreen" => {
                        flags.fullscreen = flags.fullscreen.or(Some(SetArg(set, toggle)))
                    }
                    "sticky" => flags.sticky = flags.sticky.or(Some(SetArg(set, toggle))),
                    arg => bail!("node set: unknown arg '{}'", arg),
                }
            }
        } else {
            bail!("node set: missing arguments")
        }
        if let Some(args) = flags.hidden {
            stream.send_value(&ClientRequest::SetHidden(node, args))?
        }
        if let Some(args) = flags.floating {
            stream.send_value(&ClientRequest::SetFloating(node, args))?
        }
        if let Some(args) = flags.fullscreen {
            stream.send_value(&ClientRequest::SetFullscreen(node, args))?
        }
        if let Some(args) = flags.sticky {
            stream.send_value(&ClientRequest::SetSticky(node, args))?
        }
        Ok(())
    }

    fn kill(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let node = get_node(&mut args)?;
        stream.send_value(&ClientRequest::CloseClient(node, true))
    }

    fn close(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let node = get_node(&mut args)?;
        stream.send_value(&ClientRequest::CloseClient(node, false))
    }

    fn move_(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let node = get_node(&mut args)?;
        let (tag, toggle) = monitor::get_tag(&mut args, true)?;
        stream.send_value(&ClientRequest::SetWindowTag(node, tag, toggle))
    }
}

mod tag {
    use super::*;
    pub(super) fn process(_args: Args, _stream: ClientStream) -> Result<()> {
        Ok(())
    }
}

mod monitor {
    use super::*;
    pub(super) fn process(mut args: Args, stream: ClientStream) -> Result<()> {
        match args.next() {
            Some(item) => match item.as_str() {
                "set-tag" => set_tag(args, stream),
                _ => bail!("mon: unknown argument '{}'", item),
            },
            _ => bail!("mon: No arguments provided"),
        }
    }

    pub fn get_tag(args: &mut Args, allow_toggle: bool) -> Result<(TagSelection, bool)> {
        match args.next() {
            Some(item) => match item.as_str() {
                "index" => {
                    let (toggle, idx) = get_tag_(args, allow_toggle)?;
                    Ok((TagSelection::Index(idx.parse()?), toggle))
                }
                "name" => {
                    let (toggle, name) = get_tag_(args, allow_toggle)?;
                    Ok((TagSelection::Name(name), toggle))
                }
                "~focused" => Ok((TagSelection::Focused(None), true)),
                "focused" => Ok((TagSelection::Focused(None), false)),
                _ => bail!("mon: unknown argument '{}'", item),
            },
            _ => bail!("mon: No arguments provided"),
        }
    }

    fn set_tag(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let mon = get_mon(&mut args)?;
        let (tag, toggle) = get_tag(&mut args, true)?;
        stream.send_value(&ClientRequest::FocusTag(mon, tag, toggle))
    }
}

mod subscribe {
    use super::*;
    pub(super) fn process(mut args: Args, stream: ClientStream) -> Result<()> {
        match args.next() {
            Some(item) => match item.as_str() {
                "tags" => tags(args, stream),
                "focused" => focused(args, stream),
                _ => bail!("subscribe: unknown argument '{}'", item),
            },
            _ => bail!("subscribe: No arguments provided"),
        }
    }

    fn get_monitor(args: &mut Args, stream: &mut ClientStream) -> Result<u32> {
        match args.next() {
            Some(item) => match item.as_str() {
                "-f" => {
                    stream.send_value(&ClientRequest::FocusedMonitor)?;
                    let (done, response) = stream.get_value()?;
                    if done {
                        bail!("server hung up")
                    } else if let CwmResponse::FocusedMonitor(mon) = response {
                        Ok(mon)
                    } else {
                        bail!("invalid response from server")
                    }
                } // change to query focused monitor
                item => Ok(item.parse()?),
            },
            _ => bail!("monitor id or -f must be specified"),
        }
    }

    fn tags(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let mon = get_monitor(&mut args, &mut stream)?;
        stream.send_value(&ClientRequest::TagState)?;
        loop {
            let (done, response) = stream.get_value()?;
            if let CwmResponse::TagState(tags, focused_mon) = response {
                println!(
                    "{}",
                    tags.iter()
                        .map(|tag| tag.format(mon, focused_mon))
                        .reduce(|info, tag| info + "\t" + tag.as_str())
                        .unwrap()
                );
            }
            if done {
                return Ok(());
            }
        }
    }

    fn focused(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let mon = get_monitor(&mut args, &mut stream)?;
        stream.send_value(&ClientRequest::MonitorFocus(mon))?;
        loop {
            let (done, response) = stream.get_value()?;
            if let CwmResponse::MonitorFocusedClient(client) = response {
                client
                    .map(|x| println!("{}", x))
                    .unwrap_or_else(|| println!());
            }
            if done {
                return Ok(());
            }
        }
    }
}

mod qurery {
    use super::*;
    pub(super) fn process(mut args: Args, stream: ClientStream) -> Result<()> {
        match args.next() {
            Some(item) => match item.as_str() {
                "focused" => focused(args, stream),
                _ => bail!("subscribe: unknown argument '{}'", item),
            },
            _ => bail!("subscribe: No arguments provided"),
        }
    }

    fn focused(mut args: Args, stream: ClientStream) -> Result<()> {
        match args.next() {
            Some(item) => match item.as_str() {
                "mon" => focused_mon(args, stream),
                "tag" => focused_tag(args, stream),
                "node" => focused_node(args, stream),
                _ => bail!("subscribe: unknown argument '{}'", item),
            },
            _ => bail!("subscribe: No arguments provided"),
        }
    }

    fn focused_mon(_args: Args, mut stream: ClientStream) -> Result<()> {
        stream.send_value(&ClientRequest::FocusedMonitor)?;
        loop {
            let (mut done, response) = stream.get_value()?;
            if let CwmResponse::FocusedMonitor(mon) = response {
                println!("{}", mon);
                done = true;
            }
            if done {
                return Ok(());
            }
        }
    }

    fn focused_tag(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let mon = get_mon(&mut args)?;
        stream.send_value(&ClientRequest::FocusedTag(mon))?;
        loop {
            let (mut done, response) = stream.get_value()?;
            if let CwmResponse::FocusedTag(tag) = response {
                println!("{}", tag);
                done = true;
            }
            if done {
                return Ok(());
            }
        }
    }

    fn focused_node(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let tag = get_tag(&mut args, false)?.0;
        stream.send_value(&ClientRequest::FocusedWindow(tag))?;
        loop {
            let (mut done, response) = stream.get_value()?;
            if let CwmResponse::FocusedWindow(win) = response {
                println!("{}", win.unwrap_or(0));
                done = true;
            }
            if done {
                return Ok(());
            }
        }
    }
}

mod command {
    use super::*;
    pub(super) fn process(mut args: Args, stream: ClientStream) -> Result<()> {
        match args.next() {
            Some(item) => match item.as_str() {
                "quit" => quit(stream),
                _ => bail!("command: unknown argument '{}'", item),
            },
            _ => bail!("command: No arguments provided"),
        }
    }

    fn quit(mut stream: ClientStream) -> Result<()> {
        stream.send_value(&ClientRequest::Quit)
    }
}

struct ClientStream {
    stream: Stream,
    fd: [PollFd; 1],
}

impl ClientStream {
    fn new() -> Result<Self> {
        let stream = Stream::new(UnixStream::connect(SOCKET).context(cwm::code_loc!())?);
        let fd = [PollFd::new(stream.as_raw_fd(), PollFlags::POLLIN)];
        Ok(Self { stream, fd })
    }
    fn get_value(&mut self) -> Result<(bool, CwmResponse)> {
        loop {
            poll(&mut self.fd, -1).ok();
            let info = self.stream.recieve();
            if let (done, Some(val)) = info {
                return Ok((done, val));
            } else if info.0 {
                bail!("server disconnect while waiting for value")
            }
        }
    }
    fn send_value(&mut self, val: &ClientRequest) -> Result<()> {
        if self.stream.send(val) {
            Ok(())
        } else {
            bail!("Could not send request to server")
        }
    }
}

fn main() -> Result<()> {
    SimpleLogger::init(LevelFilter::Error, Config::default()).unwrap();
    let stream = ClientStream::new()?;
    let mut args = args();
    args.next();
    match args.next() {
        Some(item) => match item.as_str() {
            "node" => node::process(args, stream),
            "tag" => tag::process(args, stream),
            "monitor" | "mon" => monitor::process(args, stream),
            "subscribe" | "sub" => subscribe::process(args, stream),
            "command" | "cmd" => command::process(args, stream),
            "query" => qurery::process(args, stream),
            _ => bail!("unknown argument '{}'", item),
        },
        _ => bail!("uo arguments provided"),
    }
}
