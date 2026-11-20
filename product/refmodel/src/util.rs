//! Small helpers: addresses, sliding-window rate limits, big-endian access.

use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, SocketAddrV4};

pub type Host = SocketAddrV4;

pub fn be16(b: &[u8], i: usize) -> u16 {
    u16::from_be_bytes([b[i], b[i + 1]])
}
pub fn be32(b: &[u8], i: usize) -> u32 {
    u32::from_be_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

pub fn in_subnet(ip: Ipv4Addr, own: Ipv4Addr, mask: Ipv4Addr) -> bool {
    u32::from(ip) & u32::from(mask) == u32::from(own) & u32::from(mask)
}
pub fn directed_broadcast(own: Ipv4Addr, mask: Ipv4Addr) -> Ipv4Addr {
    Ipv4Addr::from(u32::from(own) | !u32::from(mask))
}

/// At most `max` events per `window_us`, per key and (optionally) in total.
#[derive(Debug)]
pub struct Limiter<K: std::hash::Hash + Eq + Clone> {
    window_us: u64,
    per_key: usize,
    total: usize,
    keys: HashMap<K, VecDeque<u64>>,
    all: VecDeque<u64>,
}

impl<K: std::hash::Hash + Eq + Clone> Limiter<K> {
    pub fn new(window_us: u64, per_key: usize, total: usize) -> Self {
        Limiter { window_us, per_key, total, keys: HashMap::new(), all: VecDeque::new() }
    }
    fn prune(q: &mut VecDeque<u64>, now: u64, w: u64) {
        while let Some(&t) = q.front() {
            if now.saturating_sub(t) >= w {
                q.pop_front();
            } else {
                break;
            }
        }
    }
    /// Would an event pass now? Does not record it.
    pub fn check(&mut self, key: &K, now: u64) -> bool {
        Self::prune(&mut self.all, now, self.window_us);
        let q = self.keys.entry(key.clone()).or_default();
        Self::prune(q, now, self.window_us);
        q.len() < self.per_key && self.all.len() < self.total
    }
    pub fn record(&mut self, key: &K, now: u64) {
        self.keys.entry(key.clone()).or_default().push_back(now);
        self.all.push_back(now);
        if self.keys.len() > 4096 {
            let w = self.window_us;
            self.keys.retain(|_, q| q.back().map_or(false, |&t| now.saturating_sub(t) < w));
        }
    }
    /// check + record when it passes.
    pub fn allow(&mut self, key: &K, now: u64) -> bool {
        if self.check(key, now) {
            self.record(key, now);
            true
        } else {
            false
        }
    }
}

/// Least-recently-used map with a fixed capacity (the 4-entry per-host caches of A1.4, A1.10,
/// B5.1a).
#[derive(Debug, Clone)]
pub struct Lru<K: PartialEq + Clone, V> {
    cap: usize,
    items: Vec<(K, V)>, // most recent last
}

impl<K: PartialEq + Clone, V> Lru<K, V> {
    pub fn new(cap: usize) -> Self {
        Lru { cap, items: Vec::new() }
    }
    pub fn get(&mut self, k: &K) -> Option<&mut V> {
        let i = self.items.iter().position(|(x, _)| x == k)?;
        let e = self.items.remove(i);
        self.items.push(e);
        self.items.last_mut().map(|(_, v)| v)
    }
    pub fn peek(&self, k: &K) -> Option<&V> {
        self.items.iter().find(|(x, _)| x == k).map(|(_, v)| v)
    }
    pub fn put(&mut self, k: K, v: V) {
        if let Some(i) = self.items.iter().position(|(x, _)| *x == k) {
            self.items.remove(i);
        } else if self.items.len() == self.cap {
            self.items.remove(0);
        }
        self.items.push((k, v));
    }
    pub fn remove(&mut self, k: &K) -> Option<V> {
        let i = self.items.iter().position(|(x, _)| x == k)?;
        Some(self.items.remove(i).1)
    }
    pub fn clear(&mut self) {
        self.items.clear();
    }
    pub fn take_all(&mut self) -> Vec<(K, V)> {
        std::mem::take(&mut self.items)
    }
}
