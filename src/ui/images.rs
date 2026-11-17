use image::DynamicImage;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

pub struct ImageCache {
    protocols: HashMap<String, StatefulProtocol>,
    lru_order: VecDeque<String>,
    pending_fetches: HashSet<String>,
    max_capacity: usize,
    picker: Arc<Picker>,
}

impl ImageCache {
    pub fn new(picker: Arc<Picker>) -> Self {
        Self {
            protocols: HashMap::new(),
            lru_order: VecDeque::new(),
            pending_fetches: HashSet::new(),
            max_capacity: 128,
            picker,
        }
    }

    pub fn set_picker(&mut self, picker: Arc<Picker>) {
        self.picker = picker;
        // Re-encode cached protocols cleanly only if protocol changed; future inserts use the new picker
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut StatefulProtocol> {
        if self.protocols.contains_key(key) {
            // Touch key in LRU order
            if let Some(pos) = self.lru_order.iter().position(|k| k == key) {
                let item = self.lru_order.remove(pos).unwrap();
                self.lru_order.push_back(item);
            }
            self.protocols.get_mut(key)
        } else {
            None
        }
    }

    pub fn contains(&self, key: &str) -> bool {
        self.protocols.contains_key(key)
    }

    pub fn is_pending(&self, key: &str) -> bool {
        self.pending_fetches.contains(key)
    }

    pub fn mark_pending(&mut self, key: String) -> bool {
        self.pending_fetches.insert(key)
    }

    pub fn insert(&mut self, key: String, dyn_img: DynamicImage) {
        self.pending_fetches.remove(&key);
        if let Some(pos) = self.lru_order.iter().position(|k| k == &key) {
            self.lru_order.remove(pos);
        }

        let stateful = self.picker.new_resize_protocol(dyn_img);

        // Evict LRU if capacity exceeded
        while self.protocols.len() >= self.max_capacity && !self.protocols.contains_key(&key) {
            if let Some(oldest) = self.lru_order.pop_front() {
                self.protocols.remove(&oldest);
            } else {
                break;
            }
        }

        self.lru_order.push_back(key.clone());
        self.protocols.insert(key, stateful);
    }

    pub fn remove(&mut self, key: &str) {
        self.pending_fetches.remove(key);
        self.protocols.remove(key);
        if let Some(pos) = self.lru_order.iter().position(|k| k == key) {
            self.lru_order.remove(pos);
        }
    }

    pub fn remove_matching(&mut self, needle: &str) {
        self.pending_fetches.retain(|k| !k.contains(needle));
        self.protocols.retain(|k, _| !k.contains(needle));
        self.lru_order.retain(|k| !k.contains(needle));
    }
}
