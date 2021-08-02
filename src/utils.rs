use x11rb::{
    connection::Connection,
    protocol::xproto::*,
    COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME
};

use std::{
    collections::HashMap,
    io::Read,
    process::{Command, Stdio}
};

pub use stack::{Stack, StackElem};

pub fn keymap_xmodmap() -> HashMap<String, u8> {
    let output = Command::new("xmodmap").arg("-pke").output().unwrap();
    let string = String::from_utf8(output.stdout).unwrap();
    string.lines().filter_map(|line| {
        let mut items = line.split_whitespace();
        items.next();
        let keycode: u8 = items.next().unwrap().parse().unwrap();
        items.next();
        items.next().map(|n| (n.to_string(), keycode))
    }).collect::<HashMap<_, _>>()
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16
}

impl Rect {
    pub fn new(x: i16, y: i16, width: u16, height: u16) -> Self {
        Self { x, y, width, height }
    }

    #[inline]
    pub fn aux(&self) -> ConfigureWindowAux {
        ConfigureWindowAux::new().x(self.x as i32).y(self.y as i32).width(self.width as u32).height(self.height as u32)
    }

    pub fn copy(&mut self, other: &Rect) {
        self.x = other.x;
        self.y = other.y;
        self.width = other.width;
        self.height = other.height;
    }

    pub fn contains(&self, point: &(i16, i16)) -> bool {
        point.0 > self.x && point.0 < self.x + self.width as i16 && point.1 > self.y && point.1 < self.y + self.height as i16 
    }

    pub fn split(&self, split: f32, vert: bool, rect1: &mut Rect, rect2: &mut Rect) {
        rect1.x = self.x;
        rect1.y = self.y;
        if vert {
            rect1.width = (self.width as f32 * split).round() as _;
            rect1.height = self.height;
            rect2.x = self.x + rect1.width as i16;
            rect2.y = self.y;
            rect2.width = self.width - rect1.width;
            rect2.height = self.height;
        } else {
            rect1.width = self.width;
            rect1.height = (self.height as f32 * split).round() as _;
            rect2.x = self.x;
            rect2.y = self.y + rect1.height as i16;
            rect2.width = self.width;
            rect2.height = self.height - rect1.height;
        };
    }
}

impl From<GetGeometryReply> for Rect {
    fn from(other: GetGeometryReply) -> Self {
        Self::new(other.x, other.y, other.width, other.height)
    }
}

pub mod stack {
    use std::{
        marker::PhantomData,
        ptr::NonNull
    };
    
    #[derive(Default)]
    pub struct Stack<T> {
        head: Option<StackElem<T>>,
        tail: Option<StackElem<T>>,
        len: usize,
        marker: PhantomData<Box<Node<T>>>,
    }

    pub type StackElem<T> = NonNull<Node<T>>;
    
    pub struct Node<T> {
        next: Option<StackElem<T>>,
        prev: Option<StackElem<T>>,
        element: T,
    }
    
    impl<T> Node<T> {
        pub fn new(element: T) -> Self {
            Node { next: None, prev: None, element }
        }
    
        pub fn into_element(self: Box<Self>) -> T {
            self.element
        }
    
        pub fn next(&self) -> Option<StackElem<T>> {
            self.next
        }
    
        pub fn prev(&self) -> Option<StackElem<T>> {
            self.prev
        }
    
        pub fn element(&self) -> &T {
            &self.element
        }
    }
    
    impl<T> Stack<T> {
        pub const fn new() -> Self {
            Stack { head: None, tail: None, len: 0, marker: PhantomData }
        }
    
        pub fn push_front(&mut self, elt: T) {
            self.push_front_node(Box::new(Node::new(elt)));
        }
    
        pub fn push_back(&mut self, elt: T) {
            self.push_back_node(Box::new(Node::new(elt)));
        }
    
        pub fn unlink_node(&mut self, mut node: StackElem<T>) {
            let node = unsafe { node.as_mut() }; // this one is ours now, we can create an &mut.
    
            // Not creating new mutable (unique!) references overlapping `element`.
            match node.prev {
                Some(prev) => unsafe { (*prev.as_ptr()).next = node.next },
                // this node is the head node
                None => self.head = node.next,
            };
    
            match node.next {
                Some(next) => unsafe { (*next.as_ptr()).prev = node.prev },
                // this node is the tail node
                None => self.tail = node.prev,
            };
    
            self.len -= 1;
        }
    
        pub fn first(&mut self) -> Option<StackElem<T>> {
            self.head
        }
    
        pub fn back(&mut self) -> Option<StackElem<T>> {
            self.tail
        }
    
        pub fn front(&self) -> Option<&T> {
            unsafe { self.head.as_ref().map(|node| &node.as_ref().element) }
        }
    
        #[inline]
        fn push_front_node(&mut self, mut node: Box<Node<T>>) {
            // This method takes care not to create mutable references to whole nodes,
            // to maintain validity of aliasing pointers into `element`.
            unsafe {
                node.next = self.head;
                node.prev = None;
                let node = Some(Box::leak(node).into());
    
                match self.head {
                    None => self.tail = node,
                    // Not creating new mutable (unique!) references overlapping `element`.
                    Some(head) => (*head.as_ptr()).prev = node,
                }
    
                self.head = node;
                self.len += 1;
            }
        }
    
        #[inline]
        fn push_back_node(&mut self, mut node: Box<Node<T>>) {
            // This method takes care not to create mutable references to whole nodes,
            // to maintain validity of aliasing pointers into `element`.
            unsafe {
                node.next = None;
                node.prev = self.tail;
                let node = Some(Box::leak(node).into());
    
                match self.tail {
                    None => self.head = node,
                    // Not creating new mutable (unique!) references overlapping `element`.
                    Some(tail) => (*tail.as_ptr()).next = node,
                }
    
                self.tail = node;
                self.len += 1;
            }
        }
    
        pub fn link_node_front(&mut self, mut node: StackElem<T>) {
            unsafe {
                {
                    let node = node.as_mut();
                    node.next = self.head;
                    node.prev = None;
                }
                let node = Some(node);
    
                match self.head {
                    None => self.tail = node,
                    // Not creating new mutable (unique!) references overlapping `element`.
                    Some(head) => (*head.as_ptr()).prev = node,
                }
    
                self.head = node;
                self.len += 1;
            }
        }

        pub fn link_node_back(&mut self, mut node: StackElem<T>) {
            // This method takes care not to create mutable references to whole nodes,
            // to maintain validity of aliasing pointers into `element`.
            unsafe {
                {
                    let node = node.as_mut();
                    node.next = None;
                    node.prev = self.tail;
                }
                let node = Some(node);
    
                match self.tail {
                    None => self.head = node,
                    // Not creating new mutable (unique!) references overlapping `element`.
                    Some(tail) => (*tail.as_ptr()).next = node,
                }
    
                self.tail = node;
                self.len += 1;
            }
        }
    }
}