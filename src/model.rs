use crate::pricing::{self, Price};

#[derive(Clone)]
pub struct Record {
    pub ts: i64,
    pub provider: &'static str,
    pub model: String,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

impl Record {
    pub fn total_input(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }
}

#[derive(Default, Clone, Copy)]
pub struct Totals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub cost_with: f64,
    pub cost_without: f64,
    pub calls: u64,
}

impl Totals {
    pub fn add(&mut self, r: &Record, p: &Price) {
        self.input += r.input;
        self.output += r.output;
        self.cache_read += r.cache_read;
        self.cache_write += r.cache_write;
        let (with, without) = pricing::cost(r, p);
        self.cost_with += with;
        self.cost_without += without;
        self.calls += 1;
    }

    pub fn merge(&mut self, o: &Totals) {
        self.input += o.input;
        self.output += o.output;
        self.cache_read += o.cache_read;
        self.cache_write += o.cache_write;
        self.cost_with += o.cost_with;
        self.cost_without += o.cost_without;
        self.calls += o.calls;
    }

    pub fn total_input(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }

    pub fn total_tokens(&self) -> u64 {
        self.total_input() + self.output
    }

    pub fn saved(&self) -> f64 {
        self.cost_without - self.cost_with
    }
}
