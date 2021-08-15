use std::collections::HashSet;
use x11rb::protocol::xproto::*;

pub use stack::{Stack, StackElem};

pub fn pop_set<T: Clone + Eq + std::hash::Hash>(set: &mut HashSet<T>, order: &[T]) -> Option<T> {
    if let Some(item) = order.iter().find(|x| set.contains(x)).cloned() {
        set.remove(&item);
        Some(item)
    } else {
        None
    }
}

pub fn three_mut<T>(
    vec: &mut Vec<T>,
    idx: (usize, usize, usize),
) -> Option<(&mut T, &mut T, &mut T)> {
    match idx {
        (i1, i2, i3) if i1 > i2 && i2 > i3 => {
            let (b, a) = vec.split_at_mut(i1);
            let (c, b) = b.split_at_mut(i2);
            Some((&mut a[0], &mut b[0], &mut c[i3]))
        }
        (i1, i2, i3) if i1 > i3 && i3 > i2 => {
            let (b, a) = vec.split_at_mut(i1);
            let (c, b) = b.split_at_mut(i3);
            Some((&mut a[0], &mut c[i2], &mut b[0]))
        }
        (i1, i2, i3) if i2 > i1 && i1 > i3 => {
            let (b, a) = vec.split_at_mut(i2);
            let (c, b) = b.split_at_mut(i1);
            Some((&mut b[0], &mut a[0], &mut c[i3]))
        }
        (i1, i2, i3) if i2 > i3 && i3 > i1 => {
            let (b, a) = vec.split_at_mut(i2);
            let (c, b) = b.split_at_mut(i3);
            Some((&mut c[i1], &mut a[0], &mut b[0]))
        }
        (i1, i2, i3) if i3 > i2 && i2 > i1 => {
            let (b, a) = vec.split_at_mut(i3);
            let (c, b) = b.split_at_mut(i2);
            Some((&mut c[i1], &mut b[0], &mut a[0]))
        }
        (i1, i2, i3) if i3 > i1 && i1 > i2 => {
            let (b, a) = vec.split_at_mut(i3);
            let (c, b) = b.split_at_mut(i1);
            Some((&mut b[0], &mut c[i2], &mut a[0]))
        }
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: i16,
    pub y: i16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn new(x: i16, y: i16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn aux(&self, width: u16) -> ConfigureWindowAux {
        ConfigureWindowAux::new()
            .x(self.x as i32)
            .y(self.y as i32)
            .width((self.width - width * 2) as u32)
            .height((self.height - width * 2) as u32)
            .border_width(width as u32)
    }

    pub fn copy(&mut self, other: &Rect) {
        self.x = other.x;
        self.y = other.y;
        self.width = other.width;
        self.height = other.height;
    }

    pub fn contains(&self, point: &(i16, i16)) -> bool {
        point.0 >= self.x
            && point.0 < self.x + self.width as i16
            && point.1 >= self.y
            && point.1 < self.y + self.height as i16
    }

    pub fn contains_rect(&self, other: &Rect) -> bool {
        other.x >= self.x
            && other.y >= self.y
            && other.x + other.width as i16 <= self.x + self.width as i16
            && other.y + other.height as i16 <= self.y + self.height as i16
    }

    pub fn split(&self, split: f32, vert: bool, rect1: &mut Rect, rect2: &mut Rect, gap: u16) {
        rect1.x = self.x;
        rect1.y = self.y;
        if vert {
            rect1.width = (self.width as f32 * split).round() as u16 - gap / 2;
            rect1.height = self.height;
            rect2.x = self.x + (rect1.width + gap) as i16;
            rect2.y = self.y;
            rect2.width = self.width - (rect1.width + gap);
            rect2.height = self.height;
        } else {
            rect1.width = self.width;
            rect1.height = (self.height as f32 * split).round() as _;
            rect2.x = self.x;
            rect2.y = self.y + (rect1.height + gap) as i16;
            rect2.width = self.width;
            rect2.height = self.height - (rect1.height + gap);
        };
    }

    pub fn reposition(&mut self, old_size: &Rect, new_size: &Rect) {
        self.x = ((self.x - old_size.x) as f32
                / old_size.width as f32
                * new_size.width as f32)
                .round()
                as i16
                + new_size.x;
        self.y = ((self.y - old_size.y) as f32
            / old_size.height as f32
            * new_size.height as f32)
            .round()
            as i16
            + new_size.y;
    }
}

impl From<GetGeometryReply> for Rect {
    fn from(other: GetGeometryReply) -> Self {
        Self::new(other.x, other.y, other.width, other.height)
    }
}

pub mod stack {
    pub struct StackElem<T> {
        item: T,
        next: Option<usize>,
        prev: Option<usize>,
    }

    impl<T> StackElem<T> {
        fn new(item: T) -> Self {
            Self {
                item,
                next: None,
                prev: None,
            }
        }
    }

    #[derive(Default)]
    pub struct Stack<T> {
        items: Vec<StackElem<T>>,
        free: Vec<usize>,
        head: Option<usize>,
        tail: Option<usize>,
    }

    impl<T> Stack<T> {
        pub fn len(&self) -> usize {
            self.items.len() - self.free.len()
        }

        pub fn front(&self) -> Option<&T> {
            self.head.map(|x| &self.items[x].item)
        }

        pub fn back(&self) -> Option<&T> {
            self.tail.map(|x| &self.items[x].item)
        }

        pub fn push_front(&mut self, item: T) -> usize {
            let idx = if let Some(idx) = self.free.pop() {
                self.items[idx].item = item;
                idx
            } else {
                self.items.push(StackElem::new(item));
                self.items.len() - 1
            };
            self.items[idx].next = self.head;

            match self.head {
                None => self.tail = Some(idx),
                Some(head_idx) => self.items[head_idx].prev = Some(idx),
            }

            self.head = Some(idx);
            idx
        }

        pub fn push_back(&mut self, item: T) -> usize {
            let idx = if let Some(idx) = self.free.pop() {
                self.items[idx].item = item;
                idx
            } else {
                self.items.push(StackElem::new(item));
                self.items.len() - 1
            };
            self.items[idx].prev = self.tail;

            match self.tail {
                None => self.head = Some(idx),
                Some(tail_idx) => self.items[tail_idx].next = Some(idx),
            }

            self.tail = Some(idx);
            idx
        }

        pub fn remove_node(&mut self, idx: usize) {
            match self.items[idx].prev {
                Some(prev_idx) => self.items[prev_idx].next = self.items[idx].next,
                None => self.head = self.items[idx].next,
            }
            match self.items[idx].next {
                Some(next_idx) => self.items[next_idx].prev = self.items[idx].prev,
                None => self.tail = self.items[idx].prev,
            }
        }

        pub fn iter(&self) -> StackIter<'_, T> {
            StackIter {
                stack: self,
                curr: self.head
            }
        }
    }

    pub struct StackIter<'a, T> {
        stack: &'a Stack<T>,
        curr: Option<usize>
    }

    impl<'a, T> Iterator for StackIter<'a, T> {
        type Item = &'a T;
        fn next(&mut self) -> Option<&'a T> {
            if let Some(item) = self.curr.and_then(|x| self.stack.items.get(x)) {
                self.curr = item.next;
                Some(&item.item)
            } else {
                None
            }
        }
    }
}
