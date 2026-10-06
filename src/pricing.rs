use crate::model::Record;

#[derive(Clone, Copy)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

pub const DEFAULT: Price = Price {
    input: 1.0,
    output: 4.0,
    cache_read: 0.25,
    cache_write: 1.0,
};

const TABLE: &[(&str, f64, f64, f64, f64)] = &[
    ("gemini-2.5-flash", 0.30, 2.50, 0.075, 0.30),
    ("gemini-2.5-pro", 1.25, 10.0, 0.31, 1.25),
    ("gemini-3-flash", 0.30, 2.50, 0.075, 0.30),
    ("gemini-3-pro", 2.0, 12.0, 0.50, 2.0),
    ("gemini", 1.25, 10.0, 0.31, 1.25),
    ("claude-opus", 15.0, 75.0, 1.5, 18.75),
    ("opus", 15.0, 75.0, 1.5, 18.75),
    ("claude-sonnet", 3.0, 15.0, 0.30, 3.75),
    ("sonnet", 3.0, 15.0, 0.30, 3.75),
    ("claude-haiku", 1.0, 5.0, 0.10, 1.25),
    ("haiku", 1.0, 5.0, 0.10, 1.25),
    ("gpt-5", 1.25, 10.0, 0.125, 1.25),
    ("codex", 1.25, 10.0, 0.125, 1.25),
    ("gpt-4.1", 2.0, 8.0, 0.50, 2.0),
    ("gpt-4o", 2.5, 10.0, 1.25, 2.5),
    ("o4-mini", 1.1, 4.4, 0.275, 1.1),
    ("o3", 2.0, 8.0, 0.50, 2.0),
    ("grok", 3.0, 15.0, 0.75, 3.0),
    ("deepseek", 0.27, 1.10, 0.07, 0.27),
    ("glm", 0.60, 2.20, 0.11, 0.60),
    ("kimi", 0.60, 2.50, 0.15, 0.60),
    ("qwen", 0.40, 1.20, 0.10, 0.40),
    ("mimo", 0.30, 1.20, 0.06, 0.30),
    ("minimax", 0.30, 1.20, 0.06, 0.30),
    ("composer", 1.25, 10.0, 0.125, 1.25),
];

pub fn lookup(model: &str) -> (Price, bool) {
    let m = model.to_ascii_lowercase();
    for (pat, i, o, cr, cw) in TABLE {
        if m.contains(pat) {
            return (
                Price {
                    input: *i,
                    output: *o,
                    cache_read: *cr,
                    cache_write: *cw,
                },
                true,
            );
        }
    }
    (DEFAULT, false)
}

pub fn cost(r: &Record, p: &Price) -> (f64, f64) {
    const M: f64 = 1_000_000.0;
    let with = (r.input as f64 * p.input
        + r.output as f64 * p.output
        + r.cache_read as f64 * p.cache_read
        + r.cache_write as f64 * p.cache_write)
        / M;
    let without = (r.total_input() as f64 * p.input + r.output as f64 * p.output) / M;
    (with, without)
}
