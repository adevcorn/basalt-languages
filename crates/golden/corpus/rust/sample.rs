use std::collections::HashMap;

/// A tiny inventory used by the golden corpus.
pub struct Inventory {
    items: HashMap<String, u32>,
}

impl Inventory {
    pub fn new() -> Self {
        Self { items: HashMap::new() }
    }

    pub fn add(&mut self, name: &str, count: u32) -> u32 {
        let entry = self.items.entry(name.to_string()).or_insert(0);
        *entry += count;
        println!("{name}: {entry}");
        *entry
    }
}

fn main() {
    let mut inventory = Inventory::new();
    inventory.add("apples", 3);
}
