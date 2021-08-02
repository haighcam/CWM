use x11rb::{
    connection::Connection,
    protocol::xproto::*,
    properties::*,
    x11_utils::Serialize,
    COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME, NONE
};
use std::{
    collections::HashMap,
    cell::{RefCell, Cell, Ref, RefMut},
    rc::Rc
};
use log::info;
use crate::utils::{StackElem, Rect, stack::Node};
use crate::config::Theme;
use crate::window_manager::AtomCollection;
use super::{WindowManager, Layout, Layers, Tag, Screen, layout::LayoutInfo, WindowLocation};


pub struct Client {
    win: Window,
    frame: Window,
    pub flags: ClientFlags,
    stack_pos: StackElem<Window>,
    layer_pos: StackElem<Window>,
    layout: RefCell<Rc<RefCell<Layout>>>,
    layer: Cell<usize>
}

#[derive(Default, Debug)]
pub struct ClientFlags {
    pub fullscreen: bool,
    pub floating: bool,
    pub sticky: bool,
    pub aot: bool,
    pub hidden: bool
}

impl Client {
    pub fn layout(&self) -> Ref<Rc<RefCell<Layout>>> {
        self.layout.borrow()
    }

    pub fn layout_mut(&self) -> RefMut<Rc<RefCell<Layout>>> {
        self.layout.borrow_mut()
    }

    pub fn win(&self) -> Window {
        self.win
    }

    pub fn frame(&self) -> Window {
        self.frame
    }

    pub fn stack_pos(&self) -> StackElem<Window> {
        self.stack_pos
    }

    pub fn layer_pos(&self) -> StackElem<Window> {
        self.layer_pos
    }

    pub fn layer(&self) -> usize {
        self.layer.get()
    }

    pub fn set_layer(&self, layer: usize) {
        self.layer.set(layer)
    }

    // maps and sets WM_STATE (NORMAL: 1, ICONIC: 3, WITHDRAWN: 0)
    pub fn show(&self, wm: &WindowManager<impl Connection>) {
        let mut bytes: Vec<u8> = Vec::with_capacity(8);
        1u32.serialize_into(&mut bytes);
        NONE.serialize_into(&mut bytes);
        change_property(&wm.dpy, PropMode::REPLACE, self.win, wm.atoms.WM_STATE, wm.atoms.WM_STATE, 32, 2, &bytes);
        map_window(&wm.dpy, self.frame);
        map_window(&wm.dpy, self.win);
    }

    pub fn hide(&self, wm: &WindowManager<impl Connection>) {
        let mut bytes: Vec<u8> = Vec::with_capacity(8);
        unmap_window(&wm.dpy, self.win);
        unmap_window(&wm.dpy, self.frame);
        3u32.serialize_into(&mut bytes);
        NONE.serialize_into(&mut bytes);
        change_property(&wm.dpy, PropMode::REPLACE, self.win, wm.atoms.WM_STATE, wm.atoms.WM_STATE, 32, 2, &bytes);
    }

    fn split(&self, wm: &WindowManager<impl Connection>, split: f32, mode: SplitMode, absent: bool) -> Rc<RefCell<Layout>> {
        let _layout = self.layout().clone();
        let (child2, layout_absent) = {
            let mut layout = _layout.borrow_mut();
            let vert = match mode {
                SplitMode::Horizontal => false,
                SplitMode::Vertical => true,
                SplitMode::Max => layout.rect.height < layout.rect.width
            };
            let mut child1 = Layout::new(Rect::default(), Some((Rc::downgrade(&_layout), true)), layout.absent);
            let child2 = Rc::new(RefCell::new(Layout::new(Rect::default(), Some((Rc::downgrade(&_layout), false)), absent)));
            child1.info = layout.info.clone();
            let child1 = Rc::new(RefCell::new(child1));
            layout.info = LayoutInfo::node(split, vert, (child1.clone(), child2.clone()));
            *self.layout.borrow_mut() = child1;
            (child2, layout.absent)
        };
        if layout_absent && !absent {
            Layout::propagate_absent(wm, _layout.clone());
        } else if !(layout_absent && absent) {
            let layout = _layout.borrow();
            match &layout.info {
                LayoutInfo::Node(node) => node.resize_tiled(&layout.rect, &mut vec![]),
                _ => ()
            }
        }
        if !self.flags.fullscreen && !self.flags.floating {
            self.apply_pos_size(wm, &self.layout().borrow().rect);
        }
        child2
    }

    pub fn remove(&self, wm: &WindowManager<impl Connection>) {
        let parent = {
            let layout = self.layout.borrow();
            let mut layout = layout.borrow_mut();
            layout.info = LayoutInfo::default();
            layout.parent()
        };
        if let Some((_parent, first)) = parent {
            {
                let mut parent = _parent.borrow_mut();
                match &parent.info {
                    LayoutInfo::Node(node) => {
                        let child = node.child(!first);
                        let child = child.borrow();
                        parent.info = child.info.clone();
                        parent.absent = child.absent;
                    },
                    _ => parent.info = LayoutInfo::Empty
                };
                match &parent.info {
                    LayoutInfo::Leaf(leaf) => *leaf.client().borrow().layout.borrow_mut() = _parent.clone(),
                    _ => ()
                }
            }
            Layout::resize_tiled(_parent.clone(), wm, None);
            Layout::propagate_absent(wm, _parent);
        }
    }

    pub fn get_rect(&self) -> Option<Rect> {
        if self.flags.fullscreen {
            None
        } else {
            let layout = self.layout();
            let layout = layout.borrow();
            if self.flags.floating {
                Some(layout.floating().unwrap().clone())
            } else {
                Some(layout.tiled().clone())
            }
        }
    }

    #[inline]
    pub fn apply_pos_size(&self, wm: &WindowManager<impl Connection>, size: &Rect) {
        let aux = size.aux();
        configure_window(&wm.dpy, self.frame, &aux);
        configure_window(&wm.dpy, self.win, &aux.x(None).y(None));
    }

    // don't call with the client borrowed_mut since it could (but shouldn't) probably call client.borrow()
    pub fn set_absent(&self, wm: &WindowManager<impl Connection>) {
        // delas with all of the stuff that happens when absent is set to true;
        if let Some(parent) = {
            let layout = self.layout();
            let mut layout = layout.borrow_mut();
            if !layout.absent {
                layout.absent = true;
                layout.parent().map(|x| x.0)
            } else {
                None
            }
        } {
            Layout::propagate_absent(wm, parent);
        }
    }

    // don't call with the client borrowed_mut since it will probably call client.borrow()
    pub fn set_present(&self, wm: &WindowManager<impl Connection>) {
        // delas with all of the stuff that happens when absent is set to true;
        if let Some(parent) = {
            let layout = self.layout();
            let mut layout = layout.borrow_mut();
            if layout.absent {
                layout.absent = false;
                layout.parent().map(|x| x.0)
            } else {
                None
            }
        } {
            Layout::propagate_absent(wm, parent);
        }
    }
}

enum SplitMode {
    Horizontal,
    Vertical,
    Max
}

impl ClientFlags {
    pub fn get_layer(&self) -> usize {
        match self {
            Self { fullscreen: true, aot: false, .. } => Layers::FULLSCREEN,
            Self { floating: true, aot: false, .. } => Layers::FLOATING,
            Self { aot: false, .. } => Layers::TILING,
            Self { fullscreen: true, aot: true, .. } => Layers::FULLSCREEN + Layers::AOT,
            Self { floating: true, aot: true, .. } => Layers::FLOATING + Layers::AOT,
            Self { aot: true, .. } => Layers::TILING + Layers::AOT,
        }
    }
}


#[derive(Debug)]
pub struct ClientArgs {
    pub focus: bool,
    pub fullscreen: bool,
    pub floating: bool, 
    pub centered: bool,
    pub managed: bool,
    pub urgent: bool,
    pub sticky: bool,
    pub hidden: bool,
    min_size: (u16, u16),
    max_size: (u16, u16),
    size: (u16, u16),
    pub class: String,
    pub name: String
}

impl ClientArgs {
    pub fn new(wm: &WindowManager<impl Connection>) -> Self {
        Self {
            focus: true,
            fullscreen: false,
            floating: false,
            centered: false,
            managed: true,
            urgent: false,
            sticky: false,
            hidden: false,
            min_size: (wm.theme.window_min_width, wm.theme.window_min_height),
            size: (wm.theme.window_width, wm.theme.window_height),
            max_size: (std::u16::MAX, std::u16::MAX),
            class: "".into(),
            name: "".into()
        }
    }

    pub fn build(self, wm: &WindowManager<impl Connection>, win: Window, tag: &mut Tag, screen: &Screen) -> Window {
        let Self {
            focus,
            fullscreen,
            floating,
            centered,
            managed,
            urgent,
            sticky,
            hidden,
            min_size,
            size,
            max_size,
            class,
            name
        } = self;
        let frame = wm.dpy.generate_id().unwrap();
        let layout = tag.focused_client().and_then(|x| tag.client(x)).map(|x| x.borrow().split(wm, 0.5, SplitMode::Max, floating || fullscreen || hidden)).unwrap_or(tag.layout.clone());
        let flags = ClientFlags {
            aot: false,
            floating: floating,
            fullscreen: fullscreen,
            hidden: hidden,
            sticky
        };
        let layer_pos: StackElem<Window> = Box::leak(Box::new(Node::new(frame))).into();
        tag.focus_stack.push_back(frame);
        let stack_pos = tag.focus_stack.back().unwrap();
        let client = Rc::new(RefCell::new(Client {
            win,
            frame,
            flags,
            stack_pos,
            layer_pos,
            layout: RefCell::new(layout.clone()),
            layer: Cell::new(0)
        })); // construct the client based on flags (Set the floating size based on the desired size? / centered / default)
        let centered_rect = Rect::new(
            tag.available.x + (tag.available.width as i16 - size.0 as i16) / 2, 
            tag.available.y + (tag.available.height as i16 - size.1 as i16) as i16 / 2,
            size.0,
            size.1
        );
        layout.borrow_mut().info = LayoutInfo::leaf(centered_rect, min_size, max_size, client.clone());
        tag.clients.insert(frame, client.clone());
        // reparent
        
        {
            let client = client.borrow();
            let size = client.get_rect().unwrap_or(tag.screen_size.clone());
            let border_width = if client.flags.fullscreen {0} else {wm.theme.border_width};
            let aux = CreateWindowAux::new().event_mask(EventMask::ENTER_WINDOW | EventMask::FOCUS_CHANGE | EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY);
            create_window(&wm.dpy, COPY_DEPTH_FROM_PARENT, frame, screen.root(), size.x, size.y, size.width, size.height, border_width, WindowClass::COPY_FROM_PARENT, COPY_FROM_PARENT, &aux);
            reparent_window(&wm.dpy, win, frame, 0, 0);
            configure_window(&wm.dpy, client.win, &size.aux().x(None).y(None));
            if self.hidden {
                client.hide(wm);
            } else {
                client.show(wm);
            }
        }
        tag.set_layer(wm, frame, focus);
        if !self.hidden && self.focus {
            tag.focus_client(wm, frame)
        } else {
            change_window_attributes(&wm.dpy, frame, &ChangeWindowAttributesAux::new().border_pixel(wm.theme.border_color_unfocused));
        }
        wm.dpy.flush();
        frame
    }

    fn process_state(&mut self, state: Atom, atoms: &AtomCollection) {
        if state == atoms._NET_WM_STATE_FULLSCREEN {
            self.fullscreen = true;
        } else if state == atoms._NET_WM_STATE_STICKY {
            self.sticky = true;
        }
    }

    fn process_hints(&mut self, hints: WmHints) {
        self.urgent = hints.urgent
    }

    fn prcoess_size_hints(&mut self, size_hints: WmSizeHints) {
        if let Some(max) = size_hints.max_size.map(|x| (x.0 as u16, x.1 as u16)) {
            self.max_size = max;
        }
        if let Some(min) = size_hints.min_size.map(|x| (x.0 as u16, x.1 as u16)) {
            self.min_size = min;
            if self.max_size == self.min_size {
                self.floating = true;
            }
        }
        if let Some((_, w, h)) = size_hints.size {
            self.size = (w as u16, h as u16);
        }
    }

    fn process_class(&mut self, class: WmClass) {
        self.class = String::from_utf8(class.class().to_vec()).unwrap();
    }

    fn process_name(&mut self, name: GetPropertyReply) {
        self.class = String::from_utf8(name.value).unwrap();
    }

    fn process_transient(&mut self, transient: GetPropertyReply) {
        if let Some(mut transient) = transient.value32() {
            if transient.next().map_or(false, |transient| transient != NONE) {
                self.floating = true;
            }
        }
    }
}

impl Tag {
    pub fn unmanage(&mut self, wm: &WindowManager<impl Connection>, win: Window) {
        if let Some(client) = self.clients.remove(&win) {
            info!("Unmanaging {}", win);
            let client = client.borrow();
            let root = self.root.unwrap_or(NONE);
            let layer = &mut self.layers.0[client.layer()];
            layer.remove(client.layer_pos);
            self.focus_stack.unlink_node(client.stack_pos);
            if let Some(win) = self.focus_stack.front() {
                self.focus_client(wm, *win);
            } else {
                set_input_focus(&wm.dpy, InputFocus::POINTER_ROOT, root, CURRENT_TIME); 
            }
            destroy_window(&wm.dpy, client.frame);
            reparent_window(&wm.dpy, client.win, root, 0, 0);
            client.remove(wm);
            self.layout.borrow().print(0);
        }
    }

    pub fn manage(&mut self, wm: &WindowManager<impl Connection>, win: Window, mut args: ClientArgs, screen: &Screen) -> (WindowLocation, Window) {
        let state_cookie = get_property(&wm.dpy, false, win, wm.atoms._NET_WM_STATE, AtomEnum::ATOM, 0, 2048).unwrap();
        let hints_cookie = WmHints::get(&wm.dpy, win).unwrap();
        let size_hints_cookie = WmSizeHints::get_normal_hints(&wm.dpy, win).unwrap();
        let class_cookie = WmClass::get(&wm.dpy, win).unwrap();
        let name_cookie = get_property(&wm.dpy, false, win, AtomEnum::WM_NAME, AtomEnum::STRING, 0, 2048).unwrap();
        let transient_cookie = get_property(&wm.dpy, false, win, AtomEnum::WM_TRANSIENT_FOR, AtomEnum::WINDOW, 0, 1).unwrap();

        if let Ok(states) = state_cookie.reply() {
            if let Some(states) = states.value32() {
                for state in states {
                    args.process_state(state, &wm.atoms);
                }
            }
        }
        hints_cookie.reply().map(|hints| args.process_hints(hints));
        size_hints_cookie.reply().map(|size_hints| args.prcoess_size_hints(size_hints));
        class_cookie.reply().map(|class| args.process_class(class));
        name_cookie.reply().map(|name| args.process_name(name));
        transient_cookie.reply().map(|transient| args.process_transient(transient));

        let frame = args.build(wm, win, self, screen);
        self.layout.borrow().print(0);
        (WindowLocation::Client(self.id), frame)
    }
}