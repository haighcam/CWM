use anyhow::{bail, Context, Result};
use cwm::connections::{ClientRequest, CwmResponse, SetArg, Stream, SOCKET};
use nix::poll::{poll, PollFd, PollFlags};
use simplelog::*;
use std::env::{args, Args};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;

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
        stream.send_value(&ClientRequest::KillClient(node))
    }
}

mod tag {
    use super::*;
    pub(super) fn process(mut args: Args, mut stream: ClientStream) -> Result<()> {
        Ok(())
    }
}

mod monitor {
    use super::*;
    pub(super) fn process(mut args: Args, mut stream: ClientStream) -> Result<()> {
        Ok(())
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
                _ => bail!("subscribe: unknown argument '{}'", item),
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
            _ => bail!("unknown argument '{}'", item),
        },
        _ => bail!("uo arguments provided"),
    }
}
