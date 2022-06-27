use anyhow::{bail, Error, Result};
use cwm::connections::{
    ClientRequest, CwmResponse, HiddenSelection, Rule as Rule_, SetArg, Side as Side_, StackLayer,
    Stream, TagSelection,
};
use nix::poll::{poll, PollFd, PollFlags};
use simplelog::*;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::UnixStream;

use struct_args::{parse_u32, Arg};

struct Monitor(Option<u32>);
impl Arg for Monitor {
    fn parse_args(args: &mut Vec<String>) -> Result<Self> {
        Ok(Self(
            match args
                .pop()
                .ok_or_else(|| Error::msg("mon: No argument provided"))?
                .as_str()
            {
                "-f" => None,
                item => Some(parse_u32(item)?),
            },
        ))
    }
}

struct Tag(TagSelection, bool);
impl Arg for Tag {
    fn parse_args(args: &mut Vec<String>) -> Result<Self> {
        let item = args
            .pop()
            .ok_or_else(|| Error::msg("side: No argument provided"))?;
        let mut s = item.as_str().trim_start_matches('-');
        let mut toggle = false;
        if let Some(s_) = s.strip_prefix('~') {
            toggle = true;
            s = s_;
        }
        let tag = match s {
            "index" => TagSelection::Index(
                args.pop()
                    .ok_or_else(|| Error::msg("tag: No argument provided"))?
                    .parse()?,
            ),
            "name" => {
                let name = args.pop().unwrap_or_default();
                if name.is_empty() {
                    bail!("tag: No argument provided")
                }
                TagSelection::Name(name)
            }
            "focused" | "f" => TagSelection::Focused(None),
            "next" => TagSelection::Next(None),
            "prev" => TagSelection::Prev(None),
            "last" => TagSelection::Last(None),
            s => bail!("unknown argument '{}'", s),
        };
        Ok(Self(tag, toggle))
    }
}

struct Side(Side_);
impl Arg for Side {
    fn parse_args(args: &mut Vec<String>) -> Result<Self> {
        use Side_::*;
        Ok(Self(
            match args
                .pop()
                .ok_or_else(|| Error::msg("side: No argument provided"))?
                .as_str()
            {
                "left" => Left,
                "right" => Right,
                "top" => Top,
                "bottom" => Bottom,
                s => bail!("invalid side: {}", s),
            },
        ))
    }
}

struct Node(Option<u32>);
impl Arg for Node {
    fn parse_args(args: &mut Vec<String>) -> Result<Self> {
        Ok(Self(
            match args
                .pop()
                .ok_or_else(|| Error::msg("mon: No argument provided"))?
                .as_str()
            {
                "-f" => None,
                item => Some(parse_u32(item)?),
            },
        ))
    }
}

mod node {
    use super::*;
    #[derive(Arg)]
    pub(super) enum Args {
        Set(Node, NodeFlags),
        #[struct_args_match(ND, "set-layer")]
        SetLayer(Node, Layer),
        Kill(Node),
        Close(Node),
        #[struct_args_match(ND, "move-tag")]
        MoveTag(Node, Tag),
        Cycle,
        #[struct_args_match(ND, "!cycle")]
        CycleRev,
        Select(Node, Side),
        Move(Node, Side, u16),
        Resize(Node, Side, i16),
        IgnoreSizeHints(Node),
    }

    impl Args {
        pub(super) fn process(self, mut stream: ClientStream) -> Result<()> {
            match self {
                Self::Set(Node(node), flags) => {
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
                Self::SetLayer(Node(node), Layer(layer, toggle)) => {
                    stream.send_value(&ClientRequest::SetLayer(node, SetArg(layer, toggle)))
                }
                Self::Kill(Node(node)) => {
                    stream.send_value(&ClientRequest::CloseClient(node, true))
                }
                Self::Close(Node(node)) => {
                    stream.send_value(&ClientRequest::CloseClient(node, false))
                }
                Self::MoveTag(Node(node), Tag(tag, toggle)) => {
                    stream.send_value(&ClientRequest::SetWindowTag(node, tag, toggle))
                }
                Self::Cycle => stream.send_value(&ClientRequest::CycleWindow(false)),
                Self::CycleRev => stream.send_value(&ClientRequest::CycleWindow(true)),
                Self::Select(Node(node), Side(side)) => {
                    stream.send_value(&ClientRequest::SelectNeighbour(node, side))
                }
                Self::Move(Node(node), Side(side), amt) => {
                    stream.send_value(&ClientRequest::MoveWindow(node, side, amt))
                }
                Self::Resize(Node(node), Side(side), amt) => {
                    stream.send_value(&ClientRequest::ResizeWindow(node, side, amt))
                }
                Self::IgnoreSizeHints(Node(node)) => {
                    stream.send_value(&ClientRequest::IgnoreSizeHints(node))
                }
            }
        }
    }

    #[derive(Default)]
    pub struct NodeFlags {
        hidden: Option<SetArg<bool>>,
        floating: Option<SetArg<bool>>,
        fullscreen: Option<SetArg<bool>>,
        sticky: Option<SetArg<bool>>,
    }
    impl Arg for NodeFlags {
        fn parse_args(args: &mut Vec<String>) -> Result<Self> {
            let mut flags = NodeFlags::default();
            for mut item in args
                .pop()
                .ok_or_else(|| Error::msg("flags: No argument provided"))?
                .as_str()
                .split('.')
            {
                let mut toggle = false;
                let mut set = true;
                if let Some(item_) = item.strip_prefix('~') {
                    toggle = true;
                    item = item_;
                }
                if let Some(item_) = item.strip_prefix('!') {
                    set = false;
                    item = item_;
                }
                match item {
                    "hidden" => flags.hidden = flags.hidden.or(Some(SetArg(set, toggle))),
                    "floating" => flags.floating = flags.floating.or(Some(SetArg(set, toggle))),
                    "fullscreen" => {
                        flags.fullscreen = flags.fullscreen.or(Some(SetArg(set, toggle)))
                    }
                    "sticky" => flags.sticky = flags.sticky.or(Some(SetArg(set, toggle))),
                    arg => bail!("node set: unknown arg '{}'", arg),
                }
            }
            Ok(flags)
        }
    }

    pub struct Layer(StackLayer, bool);
    impl Arg for Layer {
        fn parse_args(args: &mut Vec<String>) -> Result<Self> {
            let item = args
                .pop()
                .ok_or_else(|| Error::msg("side: No argument provided"))?;
            let mut s = item.as_str();
            let mut toggle = false;
            if let Some(s_) = s.strip_prefix('~') {
                toggle = true;
                s = s_;
            }
            let layer = match s {
                "above" => StackLayer::Above,
                "normal" => StackLayer::Normal,
                "below" => StackLayer::Below,
                arg => bail!("node set: unknown arg '{}'", arg),
            };
            Ok(Self(layer, toggle))
        }
    }
}

mod tag {
    use super::*;
    #[derive(Arg)]
    pub(super) enum Args {
        Show(Tag, Show),
        Set(Tag, TagFlags),
        Add(String),
        Remove(Tag),
    }

    impl Args {
        pub(super) fn process(self, mut stream: ClientStream) -> Result<()> {
            match self {
                Self::Show(Tag(tag, _), Show(selection)) => {
                    stream.send_value(&ClientRequest::Show(tag, selection))
                }
                Self::Set(Tag(tag, _), TagFlags(arg)) => {
                    stream.send_value(&ClientRequest::SetMonocle(tag, arg))
                }
                Self::Add(name) => stream.send_value(&ClientRequest::AddTag(name)),
                Self::Remove(Tag(tag, _)) => stream.send_value(&ClientRequest::RemoveTag(tag)),
            }
        }
    }

    pub struct TagFlags(SetArg<bool>);
    impl Arg for TagFlags {
        fn parse_args(args: &mut Vec<String>) -> Result<Self> {
            let item = args
                .pop()
                .ok_or_else(|| Error::msg("flags: No argument provided"))?;
            let mut s = item.as_str();
            let mut toggle = false;
            let mut set = true;
            if let Some(s_) = s.strip_prefix('~') {
                toggle = true;
                s = s_;
            }
            if let Some(s_) = s.strip_prefix('!') {
                set = false;
                s = s_;
            }
            if s != "monocle" {
                bail!("tag set: unknown arg '{}'", s);
            }
            Ok(Self(SetArg(set, toggle)))
        }
    }

    pub struct Show(HiddenSelection);
    impl Arg for Show {
        fn parse_args(args: &mut Vec<String>) -> Result<Self> {
            let selection = match args
                .pop()
                .ok_or_else(|| Error::msg("show: No argument provided"))?
                .as_str()
            {
                "first" => HiddenSelection::First,
                "last" => HiddenSelection::Last,
                "all" => HiddenSelection::All,
                arg => bail!("hidden selction: unknown arg '{}'", arg),
            };
            Ok(Self(selection))
        }
    }
}

mod monitor {
    use super::*;
    #[derive(Arg)]
    pub(super) enum Args {
        #[struct_args_match(ND, "set-tag")]
        SetTag(Monitor, Tag),
    }

    impl Args {
        pub(super) fn process(self, mut stream: ClientStream) -> Result<()> {
            match self {
                Self::SetTag(Monitor(mon), Tag(tag, toggle)) => {
                    stream.send_value(&ClientRequest::FocusTag(mon, tag, toggle))
                }
            }
        }
    }
}

mod subscribe {
    use super::*;
    #[derive(Arg)]
    pub(super) enum Args {
        Tags(Monitor),
        Focused(Monitor),
    }

    impl Args {
        pub(super) fn process(self, mut stream: ClientStream) -> Result<()> {
            match self {
                Self::Tags(Monitor(mon)) => {
                    let mon = if let Some(mon) = mon {
                        mon
                    } else {
                        stream.send_value(&ClientRequest::FocusedMonitor)?;
                        let (done, response) = stream.get_value()?;
                        if done {
                            bail!("server hung up")
                        } else if let CwmResponse::FocusedMonitor(mon) = response {
                            mon
                        } else {
                            bail!("invalid response from server")
                        }
                    };
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
                Self::Focused(Monitor(mon)) => {
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
        }
    }
}

mod query {
    use super::*;
    #[derive(Arg)]
    pub(super) enum Args {
        #[struct_args_match("-f")]
        Focused(FocusedArgs),
        Name(NameArgs),
        Layers(Tag),
        Stack(Tag),
        Clients(Tag),
    }

    #[derive(Arg)]
    pub(super) enum FocusedArgs {
        #[struct_args_match("mon")]
        Monitor,
        Tag(Monitor),
        Node(Tag),
    }

    #[derive(Arg)]
    pub(super) enum NameArgs {
        #[struct_args_match("mon")]
        Monitor(Monitor),
        Tag(Tag),
    }

    fn layers(mut stream: ClientStream, Tag(tag, _): Tag) -> Result<()> {
        stream.send_value(&ClientRequest::ViewLayers(tag))?;
        let (_, response) = stream.get_value()?;
        if let CwmResponse::ViewLayers(stack) = response {
            println!("{:?}", stack);
        } else {
            bail!("invalid response from server")
        }
        Ok(())
    }

    fn stack(mut stream: ClientStream, Tag(tag, _): Tag) -> Result<()> {
        stream.send_value(&ClientRequest::ViewStack(tag))?;
        let (_, response) = stream.get_value()?;
        if let CwmResponse::ViewStack(stack) = response {
            println!("{:?}", stack);
        } else {
            bail!("invalid response from server")
        }
        Ok(())
    }

    fn clients(mut stream: ClientStream, Tag(tag, _): Tag) -> Result<()> {
        stream.send_value(&ClientRequest::ViewClients(tag))?;
        let (_, response) = stream.get_value()?;
        if let CwmResponse::ViewClients(stack) = response {
            println!("{:?}", stack);
        } else {
            bail!("invalid response from server")
        }
        Ok(())
    }

    impl Args {
        pub(super) fn process(self, stream: ClientStream) -> Result<()> {
            match self {
                Self::Focused(args) => args.process(stream),
                Self::Name(args) => args.process(stream),
                Self::Layers(tag) => layers(stream, tag),
                Self::Stack(tag) => stack(stream, tag),
                Self::Clients(tag) => clients(stream, tag),
            }
        }
    }

    impl FocusedArgs {
        pub(super) fn process(self, mut stream: ClientStream) -> Result<()> {
            match self {
                Self::Monitor => {
                    stream.send_value(&ClientRequest::FocusedMonitor)?;
                    let (_, response) = stream.get_value()?;
                    if let CwmResponse::FocusedMonitor(mon) = response {
                        println!("{}", mon);
                    } else {
                        bail!("invalid response from server")
                    }
                }
                Self::Tag(Monitor(mon)) => {
                    stream.send_value(&ClientRequest::FocusedTag(mon))?;
                    let (_, response) = stream.get_value()?;
                    if let CwmResponse::FocusedTag(tag) = response {
                        println!("{}", tag);
                    } else {
                        bail!("invalid response from server")
                    }
                }
                Self::Node(Tag(tag, _)) => {
                    stream.send_value(&ClientRequest::FocusedWindow(tag))?;
                    let (_, response) = stream.get_value()?;
                    if let CwmResponse::FocusedWindow(win) = response {
                        if let Some(win) = win {
                            println!("{}", win);
                        }
                    } else {
                        bail!("invalid response from server")
                    }
                }
            }
            Ok(())
        }
    }

    impl NameArgs {
        fn process(self, mut stream: ClientStream) -> Result<()> {
            let request = match self {
                Self::Monitor(Monitor(mon)) => ClientRequest::MonitorName(mon),
                Self::Tag(Tag(tag, _)) => ClientRequest::TagName(tag),
            };
            stream.send_value(&request)?;
            let (_, response) = stream.get_value()?;
            if let CwmResponse::Name(name) = response {
                println!("{}", name);
            } else {
                bail!("invalid response from server")
            }
            Ok(())
        }
    }
}

mod command {
    use super::*;
    #[derive(Arg)]
    pub(super) enum Args {
        Quit,
        Reload,
        #[struct_args_match("sel")]
        Select(Node),
        #[struct_args_match(ND, "sel-dir")]
        SelectDir(Side),
        #[struct_args_match(ND, "sel-parent")]
        SelectParent,
        #[struct_args_match(ND, "presel-amt")]
        PreselAmt(f32),
        #[struct_args_match(ND, "sel-cancel")]
        SelectionCancel,
        Rotate,
        #[struct_args_match(ND, "!rotate")]
        RotateRev,
    }

    impl Args {
        pub(super) fn process(self, mut stream: ClientStream) -> Result<()> {
            match self {
                Self::Quit => stream.send_value(&ClientRequest::Quit),
                Self::Reload => stream.send_value(&ClientRequest::Reload),
                Self::Select(Node(node)) => stream.send_value(&ClientRequest::Select(node)),
                Self::SelectDir(Side(side)) => stream.send_value(&ClientRequest::SelectDir(side)),
                Self::SelectParent => stream.send_value(&ClientRequest::SelectParent),
                Self::PreselAmt(amt) => stream.send_value(&ClientRequest::PreselAmt(amt)),
                Self::SelectionCancel => stream.send_value(&ClientRequest::SelectionCancel),
                Self::Rotate => stream.send_value(&ClientRequest::Rotate(false)),
                Self::RotateRev => stream.send_value(&ClientRequest::Rotate(true)),
            }
        }
    }
}

mod config {
    use super::*;
    #[derive(Arg)]
    pub(super) enum Args {
        #[struct_args_match(ND, "color-focused")]
        BorderFocused(Color),
        #[struct_args_match(ND, "color-unfocused")]
        BorderUnfocused(Color),
        #[struct_args_match(ND, "border-width")]
        BorderWidth(u16),
        Gap(u16),
        Margin(Side, i16),
    }

    pub struct Color(u32);
    impl Arg for Color {
        fn parse_args(args: &mut Vec<String>) -> Result<Self> {
            Ok(Self(parse_u32(
                args.pop()
                    .ok_or_else(|| Error::msg("mon: No argument provided"))?
                    .as_str(),
            )?))
        }
    }

    impl Args {
        pub(super) fn process(self, mut stream: ClientStream) -> Result<()> {
            match self {
                Self::BorderFocused(Color(color)) => {
                    stream.send_value(&ClientRequest::ConfigBorderFocused(color))
                }
                Self::BorderUnfocused(Color(color)) => {
                    stream.send_value(&ClientRequest::ConfigBorderUnfocused(color))
                }
                Self::BorderWidth(width) => {
                    stream.send_value(&ClientRequest::ConfigBorderWidth(width))
                }
                Self::Gap(gap) => stream.send_value(&ClientRequest::ConfigGap(gap)),
                Self::Margin(Side(side), marg) => {
                    stream.send_value(&ClientRequest::ConfigMargin(side, marg))
                }
            }
        }
    }
}

struct Rule(Rule_);
impl Arg for Rule {
    fn parse_args(args: &mut Vec<String>) -> Result<Self> {
        let mut rule = Rule_::new();
        while let Some(item) = args.pop() {
            match item.as_str() {
                "name" => rule.name(
                    args.pop()
                        .ok_or_else(|| Error::msg("rule: No argument provided"))?,
                ),
                "class" => rule.class(
                    args.pop()
                        .ok_or_else(|| Error::msg("rule: No argument provided"))?,
                ),
                "instance" | "inst" => rule.instance(
                    args.pop()
                        .ok_or_else(|| Error::msg("rule: No argument provided"))?,
                ),
                "floating" => rule.floating(true),
                "!floating" => rule.floating(false),
                "pos" => rule.pos((
                    args.pop()
                        .ok_or_else(|| Error::msg("rule: No argument provided"))?
                        .parse()?,
                    args.pop()
                        .ok_or_else(|| Error::msg("rule: No argument provided"))?
                        .parse()?,
                )),
                "size" => rule.size((
                    args.pop()
                        .ok_or_else(|| Error::msg("rule: No argument provided"))?
                        .parse()?,
                    args.pop()
                        .ok_or_else(|| Error::msg("rule: No argument provided"))?
                        .parse()?,
                )),
                "temp" => rule.temp(),
                _ => {
                    args.push(item);
                    break;
                }
            }
        }
        Ok(Self(rule))
    }
}

mod rule {
    use super::*;
    #[derive(Arg)]
    pub(super) enum Args {
        Add(Rule),
    }

    impl Args {
        pub(super) fn process(self, mut stream: ClientStream) -> Result<()> {
            match self {
                Self::Add(Rule(rule)) => stream.send_value(&ClientRequest::AddRule(rule)),
            }
        }
    }
}

#[derive(Arg)]
enum Opts {
    Node(node::Args),
    Tag(tag::Args),
    #[struct_args_match("mon")]
    Monitor(monitor::Args),
    #[struct_args_match("sub")]
    Subscribe(subscribe::Args),
    Query(query::Args),
    #[struct_args_match("cmd")]
    Command(command::Args),
    Config(config::Args),
    Rule(rule::Args),
}

struct ClientStream {
    stream: Stream,
    fd: [PollFd; 1],
}

impl ClientStream {
    fn new() -> Result<Self> {
        let socket = format!("/tmp/cwm-{}.sock", whoami::username());
        let stream = Stream::new(UnixStream::connect(socket)?);
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
    let args = Opts::from_args()?;
    let stream = ClientStream::new()?;
    match args {
        Opts::Node(args) => args.process(stream),
        Opts::Tag(args) => args.process(stream),
        Opts::Monitor(args) => args.process(stream),
        Opts::Subscribe(args) => args.process(stream),
        Opts::Query(args) => args.process(stream),
        Opts::Command(args) => args.process(stream),
        Opts::Config(args) => args.process(stream),
        Opts::Rule(args) => args.process(stream),
    }
}
