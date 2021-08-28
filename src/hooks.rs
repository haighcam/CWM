use log::info;
use std::cell::RefCell;
use std::collections::HashMap;
use std::env::var;
use std::process::{Command, Stdio};

use super::Tag;
use crate::connections::{CwmResponse, Stream, TagState};

#[derive(Default)]
pub struct Hooks {
    monitor_focused: HashMap<u32, (Vec<RefCell<Stream>>, Option<String>)>,
    pub monitor_tags: (Vec<RefCell<Stream>>, Vec<(TagState, u32)>, u32),
    script_config: Option<String>,
    script_mon_open: Option<String>,
    script_mon_close: Option<String>,
}

impl Hooks {
    pub fn new() -> Self {
        let mut script_config = None;
        let mut script_mon_open = None;
        let mut script_mon_close = None;
        if let Ok(path) = var("HOME") {
            let config = path.clone() + "/.config/cwm/cwmrc";
            let mon_open = path.clone() + "/.config/cwm/mon_open";
            let mon_close = path + "/.config/cwm/mon_close";
            if std::path::Path::new(&config).exists() {
                script_config.replace(config);
            }
            if std::path::Path::new(&mon_open).exists() {
                script_mon_open.replace(mon_open);
            }
            if std::path::Path::new(&mon_close).exists() {
                script_mon_close.replace(mon_close);
            }
        }
        Self {
            script_config,
            script_mon_open,
            script_mon_close,
            ..Self::default()
        }
    }

    pub fn config(&self) {
        if let Some(script) = &self.script_config {
            Command::new(script)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .unwrap();
        }
    }

    pub fn mon_open(&mut self, mon: u32, name: &str, bg: u32) {
        self.monitor_focused.insert(mon, (Vec::new(), None));
        if let Some(script) = &self.script_mon_open {
            Command::new(script)
                .arg(mon.to_string())
                .arg(name)
                .arg(bg.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
        }
    }

    pub fn mon_close(&mut self, mon: u32, name: &str) {
        self.monitor_focused.remove(&mon);
        if let Some(script) = &self.script_mon_close {
            Command::new(script)
                .arg(mon.to_string())
                .arg(name)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
        }
    }

    pub fn monitor_focus(&mut self, id: u32, focused: Option<String>) {
        if let Some((hooks, curr)) = self.monitor_focused.get_mut(&id) {
            if *curr != focused {
                *curr = focused;
                let message = CwmResponse::MonitorFocusedClient(curr.clone());
                hooks.retain(|hook| hook.borrow_mut().send(&message));
            }
        }
    }

    pub fn add_monitor_focus(&mut self, id: u32, mut stream: Stream) {
        if let Some((hooks, curr)) = self.monitor_focused.get_mut(&id) {
            if stream.send(&CwmResponse::MonitorFocusedClient(curr.clone())) {
                hooks.push(RefCell::new(stream));
            } else {
                info!("dropped hook");
            }
        }
    }

    pub fn add_monitor_tag(&mut self, mut stream: Stream) {
        if stream.send(&CwmResponse::TagState(
            self.monitor_tags.1.iter().map(|x| x.0.clone()).collect(),
            self.monitor_tags.2,
        )) {
            self.monitor_tags.0.push(RefCell::new(stream))
        }
    }

    pub fn update_tag(&mut self, tag: &Tag) {
        #[inline]
        fn val_changed<T: PartialEq>(val: &mut T, new: T) -> bool {
            if *val != new {
                *val = new;
                true
            } else {
                false
            }
        }
        if let Some((state, _)) = self.monitor_tags.1.iter_mut().find(|x| x.1 == tag.id) {
            if 
            val_changed(&mut state.name, tag.name.clone())
            || val_changed(&mut state.focused, tag.monitor)
            || val_changed(&mut state.urgent, tag.urgent())
            || val_changed(&mut state.empty, tag.empty()) {
                let message = CwmResponse::TagState(self.monitor_tags.1.iter().map(|x| x.0.clone()).collect(), self.monitor_tags.2);
                self.monitor_tags
                    .0
                    .retain(|hook| hook.borrow_mut().send(&message));    
            }
        }
    }

    pub fn tag_update(&mut self, tags: &HashMap<u32, Tag>, order: &[u32], focused_mon: u32) {
        #[inline]
        fn val_changed<T: PartialEq>(val: &mut T, new: T) -> bool {
            if *val != new {
                *val = new;
                true
            } else {
                false
            }
        }
        let mut changed = val_changed(&mut self.monitor_tags.2, focused_mon);
        if self.monitor_tags.1.len() < tags.len() {
            self.monitor_tags.1.extend(vec![
                (TagState::default(), 0);
                tags.len() - self.monitor_tags.1.len()
            ]);
            changed = true;
        }
        if self.monitor_tags.1.len() > tags.len() {
            self.monitor_tags.1.drain(tags.len()..);
            changed = true;
        }
        for (tag, (state, _)) in order
            .iter()
            .map(|id| tags.get(id).unwrap())
            .zip(self.monitor_tags.1.iter_mut())
        {
            changed |= val_changed(&mut state.name, tag.name.clone());
            changed |= val_changed(&mut state.focused, tag.monitor);
            changed |= val_changed(&mut state.urgent, tag.urgent());
            changed |= val_changed(&mut state.empty, tag.empty());
        }
        if changed {
            let message = CwmResponse::TagState(self.monitor_tags.1.iter().map(|x| x.0.clone()).collect(), self.monitor_tags.2);
            self.monitor_tags
                .0
                .retain(|hook| hook.borrow_mut().send(&message));
        }
    }
}
