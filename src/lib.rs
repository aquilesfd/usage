pub mod capture;
pub mod model;
pub mod paths;
pub mod pricing;
pub mod providers;
pub mod render;
pub mod store;

pub struct Opts {
    pub providers: Option<Vec<String>>,
    pub days: i64,
    pub top: usize,
    pub period: String,
    pub json: bool,
    pub sources: bool,
    pub color: bool,
}
