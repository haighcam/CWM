use serde::{Deserialize, Serialize};

use crate::tag::ClientArgs;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Rule {
    pub class: Option<String>,
    pub instance: Option<String>,
    pub name: Option<String>,
    floating: Option<bool>,
    size: Option<(u16, u16)>,
    pos: Option<(i16, i16)>,
    temp: bool,
}

impl Rule {
    pub fn new() -> Self {
        Rule::default()
    }
    pub fn class(&mut self, class: String) {
        self.class.replace(class);
    }
    pub fn instance(&mut self, instance: String) {
        self.instance.replace(instance);
    }
    pub fn name(&mut self, name: String) {
        self.name.replace(name);
    }
    pub fn floating(&mut self, floating: bool) {
        self.floating.replace(floating);
    }
    pub fn size(&mut self, size: (u16, u16)) {
        self.size.replace(size);
    }
    pub fn pos(&mut self, pos: (i16, i16)) {
        self.pos.replace(pos);
    }
    pub fn temp(&mut self) {
        self.temp = true;
    }

    pub fn apply(&self, args: &mut ClientArgs) -> bool {
        if let Some(floating) = self.floating {
            args.flags.floating = floating;
        }
        if let Some(size) = self.size {
            args.size = size;
        }
        if let Some(pos) = self.pos {
            args.pos.replace(pos);
        }
        self.temp
    }
}
