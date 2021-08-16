use anyhow::{bail, Context, Result};
use cwm::connections::{
    ClientRequest, CwmResponse, HiddenSelection, SetArg, Side, StackLayer, Stream, TagSelection,
    SOCKET,
};
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
                    bail!("invalid mon id '{}'", item)
                }
            }
        },
        _ => bail!("mon id or -f must be specified"),
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
        bail!("tag: No argument provided")
    }
}

fn get_tag(args: &mut Args, allow_toggle: bool) -> Result<(TagSelection, bool)> {
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
            ("~focused", true) | ("-~f", true) => Ok((TagSelection::Focused(None), true)),
            ("~next", true) => Ok((TagSelection::Next(None), true)),
            ("~prev", true) => Ok((TagSelection::Prev(None), true)),
            ("~last", true) => Ok((TagSelection::Last(None), true)),
            ("focused", _) | ("-f", _) => Ok((TagSelection::Focused(None), false)),
            ("next", _) => Ok((TagSelection::Next(None), false)),
            ("prev", _) => Ok((TagSelection::Prev(None), false)),
            ("last", _) => Ok((TagSelection::Last(None), false)),
            _ => bail!("tag: unknown argument '{}'", item),
        },
        _ => bail!("tag: No arguments provided"),
    }
}

fn get_side(args: &mut Args) -> Result<Side> {
    match args.next() {
        Some(item) => match item.as_str() {
            "left" => Ok(Side::Left),
            "right" => Ok(Side::Right),
            "top" => Ok(Side::Top),
            "bottom" => Ok(Side::Bottom),
            _ => bail!("invalid side: {}", item),
        },
        _ => bail!("side: No arguments provided"),
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
                "set-layer" => set_layer(args, stream),
                "kill" => kill(args, stream),
                "close" => close(args, stream),
                "move-tag" => move_tag(args, stream),
                "cycle" => cycle(args, stream, false),
                "!cycle" => cycle(args, stream, true),
                "select" => select(args, stream),
                "move" => move_(args, stream),
                "resize" => resize(args, stream),
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

    fn set_layer(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let node = get_node(&mut args)?;
        let args = if let Some(item) = args.next() {
            let mut start = 0;
            let mut toggle = false;
            if &item[0..1] == "~" {
                toggle = true;
                start += 1;
            }
            match &item[start..] {
                "above" => SetArg(StackLayer::Above, toggle),
                "normal" => SetArg(StackLayer::Normal, toggle),
                "below" => SetArg(StackLayer::Below, toggle),
                arg => bail!("node set: unknown arg '{}'", arg),
            }
        } else {
            bail!("node set-layer: missing arguments")
        };
        stream.send_value(&ClientRequest::SetLayer(node, args))
    }

    fn kill(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let node = get_node(&mut args)?;
        stream.send_value(&ClientRequest::CloseClient(node, true))
    }

    fn close(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let node = get_node(&mut args)?;
        stream.send_value(&ClientRequest::CloseClient(node, false))
    }

    fn move_tag(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let node = get_node(&mut args)?;
        let (tag, toggle) = get_tag(&mut args, true)?;
        stream.send_value(&ClientRequest::SetWindowTag(node, tag, toggle))
    }

    fn cycle(_args: Args, mut stream: ClientStream, rev: bool) -> Result<()> {
        stream.send_value(&ClientRequest::CycleWindow(rev))
    }

    fn select(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let node = get_node(&mut args)?;
        let side = get_side(&mut args)?;
        stream.send_value(&ClientRequest::SelectNeighbour(node, side))
    }

    fn move_(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let node = get_node(&mut args)?;
        let side = get_side(&mut args)?;
        let amt = match args.next() {
            Some(item) => item.parse()?,
            _ => bail!("node: No arguments provided"),
        };
        stream.send_value(&ClientRequest::MoveWindow(node, side, amt))
    }

    fn resize(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let node = get_node(&mut args)?;
        let side = get_side(&mut args)?;
        let amt = match args.next() {
            Some(item) => item.parse()?,
            _ => bail!("node: No arguments provided"),
        };
        stream.send_value(&ClientRequest::ResizeWindow(node, side, amt))
    }
}

mod tag {
    use super::*;
    pub(super) fn process(mut args: Args, stream: ClientStream) -> Result<()> {
        match args.next() {
            Some(item) => match item.as_str() {
                "show" => show(args, stream),
                "set" => set(args, stream),
                _ => bail!("tag: unknown argument '{}'", item),
            },
            _ => bail!("tag: No arguments provided"),
        }
    }

    fn show(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let tag = get_tag(&mut args, false)?.0;
        let selection;
        match args.next() {
            Some(item) => match item.as_str() {
                "first" => selection = HiddenSelection::First,
                "last" => selection = HiddenSelection::Last,
                "all" => selection = HiddenSelection::All,
                _ => bail!("tag: unknown argument '{}'", item),
            },
            _ => bail!("tag: No arguments provided"),
        }
        stream.send_value(&ClientRequest::Show(tag, selection))
    }

    fn set(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let tag = get_tag(&mut args, false)?.0;
        let arg = if let Some(item) = args.next() {
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
            if &item[start..] == "monocle" {
                SetArg(set, toggle)
            } else {
                bail!("tag set: unknown arg '{}'", &item[start..]);
            }
        } else {
            bail!("tag set: missing arguments")
        };
        stream.send_value(&ClientRequest::SetMonocle(tag, arg))
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

mod query {
    use super::*;
    pub(super) fn process(mut args: Args, stream: ClientStream) -> Result<()> {
        match args.next() {
            Some(item) => match item.as_str() {
                "focused" => focused(args, stream),
                "name" => name(args, stream),
                _ => bail!("query: unknown argument '{}'", item),
            },
            _ => bail!("query: No arguments provided"),
        }
    }

    fn focused(mut args: Args, stream: ClientStream) -> Result<()> {
        match args.next() {
            Some(item) => match item.as_str() {
                "mon" => focused_mon(args, stream),
                "tag" => focused_tag(args, stream),
                "node" => focused_node(args, stream),
                _ => bail!("query: unknown argument '{}'", item),
            },
            _ => bail!("query: No arguments provided"),
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

    fn name(mut args: Args, mut stream: ClientStream) -> Result<()> {
        let request = match args.next() {
            Some(item) => match item.as_str() {
                "mon" => ClientRequest::MonitorName(get_mon(&mut args)?),
                "tag" => ClientRequest::TagName(get_tag(&mut args, false)?.0),
                _ => bail!("query: unknown argument '{}'", item),
            },
            _ => bail!("query: No arguments provided"),
        };
        stream.send_value(&request)?;
        loop {
            let (mut done, response) = stream.get_value()?;
            if let CwmResponse::Name(name) = response {
                println!("{}", name);
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
            "query" => query::process(args, stream),
            _ => bail!("unknown argument '{}'", item),
        },
        _ => bail!("uo arguments provided"),
    }
}
