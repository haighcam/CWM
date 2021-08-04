use crate::connections::{Stream, CWMResponse};
use std::cell::RefCell;

#[derive(Default)]
pub(crate) struct Hooks {
    pub monitor_focused: Vec<(Vec<RefCell<Stream>>, Option<String>)>
}

impl Hooks {
    pub fn monitor_focus(&mut self, id: usize, focused: Option<String>) {
        if let Some((hooks, curr)) = self.monitor_focused.get_mut(id) {
            if *curr != focused {
                *curr = focused;
                let message = CWMResponse::FocusedClient(curr.clone());
                hooks.retain(|hook| hook.borrow_mut().send(&message));
            }
        }
    }

    pub fn add_monitor_focus(&mut self, id: usize, stream: RefCell<Stream>) {
        if let Some((hooks, curr)) = self.monitor_focused.get_mut(id) {
            let message = CWMResponse::FocusedClient(curr.clone());
            if stream.borrow_mut().send(&message) {
                hooks.push(stream);
            }
        }
    }
}

