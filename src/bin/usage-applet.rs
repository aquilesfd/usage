//! COSMIC panel applet for the `usage` dashboard.
//!
//! Reads the snapshot history written by `usage capture` (normally driven by a
//! systemd user timer every 5 minutes) and shows plan quotas + token usage.

use std::cmp::Ordering;
use std::sync::LazyLock;
use std::time::Duration;

use chrono::Timelike;
use cosmic::Element;
use cosmic::app::{Core, Task};
use cosmic::applet::cosmic_panel_config::PanelAnchor;
use cosmic::iced::widget::{column, row};
use cosmic::iced::window::Id;
use cosmic::iced::{Alignment, Length, Rectangle};
use cosmic::surface::action::{app_popup, destroy_popup};
use cosmic::widget::{
    Id as WidgetId, autosize, button, container, determinate_linear, icon, progress_bar, text,
};

use usage::paths;
use usage::store::{self, PlanPoint, WindowPoint};

const APP_ID: &str = "io.github.aquilesfd.UsageApplet";
const REFRESH_SECS: u64 = 60;
/// Gap between the panel and the applet popup, in pixels.
const POPUP_GAP: i32 = 10;
/// Display order for rolling usage windows.
const WINDOW_ORDER: &[&str] = &["5h", "weekly", "monthly"];

static AUTOSIZE_ID: LazyLock<WidgetId> = LazyLock::new(|| WidgetId::new("usage-applet-autosize"));

pub fn main() -> cosmic::iced::Result {
    cosmic::applet::run::<Applet>(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Metric {
    Percent,
    Cost,
    Tokens,
}

impl Metric {
    fn as_str(self) -> &'static str {
        match self {
            Metric::Percent => "percent",
            Metric::Cost => "cost",
            Metric::Tokens => "tokens",
        }
    }
    fn from_str(s: &str) -> Self {
        match s {
            "cost" => Metric::Cost,
            "tokens" => Metric::Tokens,
            _ => Metric::Percent,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Config {
    metric: String,
    #[serde(default)]
    remaining: bool,
    #[serde(default)]
    show_unused: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            metric: "percent".to_string(),
            remaining: false,
            show_unused: false,
        }
    }
}

fn config_path() -> std::path::PathBuf {
    paths::usage_dir().join("applet.json")
}

fn load_config() -> Config {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_config(metric: Metric, remaining: bool, show_unused: bool) {
    let cfg = Config {
        metric: metric.as_str().to_string(),
        remaining,
        show_unused,
    };
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(&cfg) {
        let _ = std::fs::write(path, text);
    }
}

#[derive(Clone, Debug)]
enum Message {
    Refresh,
    SetMetric(Metric),
    SetRemaining(bool),
    ToggleUnused,
    PopupClosed(Id),
    Surface(cosmic::surface::Action<Message>),
}

#[derive(Default)]
struct Snapshot {
    captured_at: Option<i64>,
    subscriptions: Vec<Subscription>,
}

/// One provider subscription: quota windows plus cumulative and rolling usage.
struct Subscription {
    provider: String,
    plan: String,
    windows: Vec<PlanPoint>,
    usage: Vec<WindowPoint>,
    details: String,
    note: Option<String>,
    today_cost: f64,
    today_tokens: u64,
    today_calls: u64,
    total_cost: f64,
    total_tokens: u64,
}

impl Subscription {
    fn new(provider: String, plan: String) -> Self {
        Self {
            provider,
            plan,
            windows: Vec::new(),
            usage: Vec::new(),
            details: String::new(),
            note: None,
            today_cost: 0.0,
            today_tokens: 0,
            today_calls: 0,
            total_cost: 0.0,
            total_tokens: 0,
        }
    }

    fn is_used(&self) -> bool {
        self.total_cost > 0.0
            || self.total_tokens > 0
            || self.windows.iter().any(|w| w.used_pct > 0.0)
    }

    fn worst_used(&self) -> Option<f64> {
        self.windows
            .iter()
            .map(|w| w.used_pct)
            .fold(None, |acc, v| Some(acc.map_or(v, |a: f64| a.max(v))))
    }

    fn usage(&self, window: &str) -> Option<&WindowPoint> {
        self.usage.iter().find(|w| w.window == window)
    }
}

fn today_start() -> i64 {
    let now = chrono::Local::now();
    now.timestamp() - now.time().num_seconds_from_midnight() as i64
}

fn load() -> Snapshot {
    let db = paths::db_path();
    if !db.exists() {
        return Snapshot::default();
    }
    let conn = match store::open_readonly(&db) {
        Ok(c) => c,
        Err(_) => return Snapshot::default(),
    };
    let captured_at = store::latest_capture_at(&conn);
    let plans = store::current_plans(&conn);
    let latest = store::latest_providers(&conn);
    let today = store::usage_since(&conn, today_start());
    let rolling = store::latest_windows(&conn);

    let mut subscriptions: Vec<Subscription> = Vec::new();

    for p in &plans {
        if let Some(s) = subscriptions.iter_mut().find(|s| s.provider == p.provider) {
            if p.window != "plan" {
                s.windows.push(p.clone());
            }
        } else {
            let mut s = Subscription::new(p.provider.clone(), p.plan.clone());
            if p.window != "plan" {
                s.windows.push(p.clone());
            }
            s.details = p.details.clone();
            s.note = p.note.clone();
            subscriptions.push(s);
        }
    }

    for p in &latest {
        if !subscriptions.iter().any(|s| s.provider == p.provider) {
            subscriptions.push(Subscription::new(p.provider.clone(), "usage".to_string()));
        }
        if let Some(s) = subscriptions.iter_mut().find(|s| s.provider == p.provider) {
            s.total_cost = p.cost_with;
            s.total_tokens = p.total_tokens();
            if let Some(d) = today.get(&p.provider) {
                s.today_cost = d.cost;
                s.today_tokens = d.tokens;
                s.today_calls = d.calls;
            }
        }
    }

    for w in rolling {
        if let Some(s) = subscriptions.iter_mut().find(|s| s.provider == w.provider) {
            s.usage.push(w);
        }
    }

    Snapshot {
        captured_at,
        subscriptions,
    }
}

fn money(x: f64) -> String {
    if x >= 1000.0 {
        format!("${x:.0}")
    } else {
        format!("${x:.2}")
    }
}

fn mtok(tokens: u64) -> String {
    format!("{:.2} Mtok", tokens as f64 / 1e6)
}

fn mtok_short(tokens: u64) -> String {
    format!("{:.1}M", tokens as f64 / 1e6)
}

fn display_pct(used: f64, remaining: bool) -> f64 {
    if remaining {
        (100.0 - used).clamp(0.0, 100.0)
    } else {
        used.clamp(0.0, 100.0)
    }
}

fn severity_color(used: f64) -> cosmic::iced::Color {
    if used >= 85.0 {
        cosmic::iced::Color::from_rgb(0.88, 0.11, 0.14)
    } else if used >= 60.0 {
        cosmic::iced::Color::from_rgb(0.90, 0.65, 0.04)
    } else {
        cosmic::iced::Color::from_rgb(0.18, 0.76, 0.49)
    }
}

fn age(now: i64, ts: i64) -> String {
    let d = now - ts;
    if d <= 0 {
        "just now".to_string()
    } else if d >= 86400 {
        format!("{}d ago", d / 86400)
    } else if d >= 3600 {
        format!("{}h ago", d / 3600)
    } else {
        format!("{}m ago", (d / 60).max(1))
    }
}

fn reset_label(now: i64, ts: i64) -> String {
    let d = ts - now;
    if d <= 0 {
        "expired".to_string()
    } else if d >= 86400 {
        format!("in {}d {}h", d / 86400, (d % 86400) / 3600)
    } else {
        format!("in {}h {}m", d / 3600, (d % 3600) / 60)
    }
}

struct Applet {
    core: Core,
    popup: Option<Id>,
    data: Snapshot,
    metric: Metric,
    remaining: bool,
    show_unused: bool,
}

impl Applet {
    fn worst_used(&self) -> Option<f64> {
        self.data
            .subscriptions
            .iter()
            .filter(|s| s.is_used())
            .filter_map(|s| s.worst_used())
            .fold(None, |acc, v| Some(acc.map_or(v, |a: f64| a.max(v))))
    }

    fn today_cost(&self) -> f64 {
        self.data.subscriptions.iter().map(|s| s.today_cost).sum()
    }

    fn today_tokens(&self) -> u64 {
        self.data.subscriptions.iter().map(|s| s.today_tokens).sum()
    }

    fn panel_label(&self) -> String {
        match self.metric {
            Metric::Percent => match self.worst_used() {
                Some(used) => format!("{:.0}%", display_pct(used, self.remaining)),
                None => "usage".to_string(),
            },
            Metric::Cost => {
                let cost = self.today_cost();
                if cost > 0.0 {
                    format!("${cost:.0}")
                } else {
                    "usage".to_string()
                }
            }
            Metric::Tokens => {
                let tokens = self.today_tokens();
                if tokens > 0 {
                    mtok_short(tokens)
                } else {
                    "usage".to_string()
                }
            }
        }
    }

    fn metric_button(&self, label: &'static str, metric: Metric) -> Element<'_, Message> {
        let selected = self.metric == metric;
        button::custom(text(label).size(12))
            .padding([3, 12])
            .selected(selected)
            .class(if selected {
                cosmic::theme::Button::Suggested
            } else {
                cosmic::theme::Button::Text
            })
            .on_press(Message::SetMetric(metric))
            .into()
    }

    fn direction_button(&self, label: &'static str, remaining: bool) -> Element<'_, Message> {
        let selected = self.remaining == remaining;
        button::custom(text(label).size(12))
            .padding([3, 10])
            .selected(selected)
            .class(if selected {
                cosmic::theme::Button::Suggested
            } else {
                cosmic::theme::Button::Text
            })
            .on_press(Message::SetRemaining(remaining))
            .into()
    }

    fn bar(&self, fraction: f64, color: cosmic::iced::Color) -> Element<'_, Message> {
        determinate_linear(fraction.clamp(0.0, 1.0) as f32)
            .width(Length::Fill)
            .girth(Length::Fixed(7.0))
            .class(progress_bar::style::Class::default().bar_color(color))
            .into()
    }

    fn window_max(&self, used: &[&Subscription], window: &str, metric: Metric) -> f64 {
        used.iter()
            .filter_map(|s| s.usage(window))
            .map(|w| match metric {
                Metric::Tokens => w.tokens as f64,
                _ => w.cost,
            })
            .fold(0.0, f64::max)
    }

    fn subscription_view(
        &self,
        s: &Subscription,
        used_subs: &[&Subscription],
    ) -> Element<'_, Message> {
        let mut col = column![].spacing(4).width(Length::Fill);

        let headline = match self.metric {
            Metric::Percent => match s.worst_used() {
                Some(used) => {
                    let v = display_pct(used, self.remaining);
                    if self.remaining {
                        format!("{v:.0}% left")
                    } else {
                        format!("{v:.0}% used")
                    }
                }
                None => "no quota".to_string(),
            },
            Metric::Cost => format!("{} today", money(s.today_cost)),
            Metric::Tokens => format!("{} today", mtok(s.today_tokens)),
        };

        col = col.push(
            row![
                container(
                    text(s.provider.clone())
                        .size(13)
                        .class(cosmic::theme::Text::Accent)
                )
                .width(Length::Fixed(96.0)),
                container(text(s.plan.clone()).size(12)).width(Length::Fill),
                container(text(headline).size(12))
                    .width(Length::Fixed(130.0))
                    .align_x(Alignment::End),
            ]
            .align_y(Alignment::Center)
            .width(Length::Fill),
        );

        match self.metric {
            Metric::Percent => {
                if s.windows.is_empty() {
                    col = col.push(text("no local quota recorded").size(10));
                }
                for w in &s.windows {
                    let used = w.used_pct.clamp(0.0, 100.0);
                    let shown = display_pct(used, self.remaining);
                    let detail = w
                        .resets_at
                        .map(|ts| reset_label(chrono::Utc::now().timestamp(), ts))
                        .unwrap_or_default();
                    col = col.push(
                        row![
                            container(text(w.window.clone()).size(11)).width(Length::Fixed(52.0)),
                            container(self.bar(shown / 100.0, severity_color(used)))
                                .width(Length::Fill),
                            container(text(format!("{shown:.0}%")).size(11))
                                .width(Length::Fixed(40.0))
                                .align_x(Alignment::End),
                            container(text(detail).size(10)).width(Length::Fixed(88.0)),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    );
                }
                // For providers without quota, still surface rolling usage.
                if s.windows.is_empty() && !s.usage.is_empty() {
                    col = col.push(text(self.rolling_summary(s)).size(10));
                }
            }
            Metric::Cost | Metric::Tokens => {
                let color = if self.metric == Metric::Cost {
                    cosmic::iced::Color::from_rgb(0.20, 0.60, 0.95)
                } else {
                    cosmic::iced::Color::from_rgb(0.60, 0.45, 0.95)
                };
                for label in WINDOW_ORDER {
                    let Some(w) = s.usage(label) else { continue };
                    let value = match self.metric {
                        Metric::Tokens => w.tokens as f64,
                        _ => w.cost,
                    };
                    let max = self.window_max(used_subs, label, self.metric);
                    let frac = if max > 0.0 { value / max } else { 0.0 };
                    let value_text = match self.metric {
                        Metric::Tokens => mtok(w.tokens),
                        _ => money(w.cost),
                    };
                    col = col.push(
                        row![
                            container(text(*label).size(11)).width(Length::Fixed(52.0)),
                            container(self.bar(frac, color)).width(Length::Fill),
                            container(text(value_text).size(11))
                                .width(Length::Fixed(96.0))
                                .align_x(Alignment::End),
                        ]
                        .spacing(6)
                        .align_y(Alignment::Center),
                    );
                }
            }
        }

        let mut secondary = match self.metric {
            Metric::Percent => s.details.clone(),
            Metric::Cost => format!("total {} · {} calls", money(s.total_cost), s.today_calls),
            Metric::Tokens => format!("total {} · {} calls", mtok(s.total_tokens), s.today_calls),
        };
        if secondary.is_empty() {
            if let Some(note) = &s.note {
                secondary = note.clone();
            }
        }
        if !secondary.is_empty() {
            col = col.push(text(secondary).size(10));
        }

        col.into()
    }

    fn rolling_summary(&self, s: &Subscription) -> String {
        let mut parts = Vec::new();
        for label in WINDOW_ORDER {
            if let Some(w) = s.usage(label) {
                parts.push(format!("{label} {}", mtok(w.tokens)));
            }
        }
        parts.join(" · ")
    }

    fn unused_view(&self, unused: &[&Subscription]) -> Element<'_, Message> {
        let marker = if self.show_unused { "▾" } else { "▸" };
        let header = button::custom(
            row![
                text(marker).size(12),
                text(format!("{} unused subscriptions", unused.len())).size(11),
            ]
            .spacing(6)
            .align_y(Alignment::Center),
        )
        .padding([4, 6])
        .class(cosmic::theme::Button::Text)
        .on_press(Message::ToggleUnused);

        let mut col = column![].spacing(3).push(header);
        if self.show_unused {
            for s in unused {
                let mut label = s.plan.clone();
                if let Some(note) = &s.note {
                    label = format!("{} · {}", label, note);
                }
                col = col.push(
                    row![
                        container(text(s.provider.clone()).size(11)).width(Length::Fixed(96.0)),
                        container(text(label).size(10)).width(Length::Fill),
                    ]
                    .padding([0, 6]),
                );
            }
        }
        col.into()
    }

    fn popup_view(&self) -> Element<'_, Message> {
        let now = chrono::Utc::now().timestamp();
        let mut col = column![].spacing(8).padding([12, 14]).width(Length::Fill);

        let captured = self
            .data
            .captured_at
            .map(|ts| format!("updated {}", age(now, ts)))
            .unwrap_or_else(|| "no captures yet".to_string());
        col = col.push(
            row![
                text::title4("usage"),
                text("by aquilesfd").size(11),
                container(text(captured).size(10))
                    .width(Length::Fill)
                    .align_x(Alignment::End),
            ]
            .spacing(6)
            .align_y(Alignment::Center)
            .width(Length::Fill),
        );

        // Controls: metric + used/left direction.
        col = col.push(
            row![
                self.metric_button("%", Metric::Percent),
                self.metric_button("$", Metric::Cost),
                self.metric_button("Mtok", Metric::Tokens),
                container(text("show").size(10))
                    .width(Length::Fill)
                    .align_x(Alignment::End),
                self.direction_button("used", false),
                self.direction_button("left", true),
            ]
            .spacing(4)
            .align_y(Alignment::Center)
            .width(Length::Fill),
        );

        col = col.push(cosmic::widget::divider::horizontal::default());

        let mut used: Vec<&Subscription> = self
            .data
            .subscriptions
            .iter()
            .filter(|s| s.is_used())
            .collect();
        match self.metric {
            Metric::Percent => used.sort_by(|a, b| {
                b.worst_used()
                    .unwrap_or(0.0)
                    .partial_cmp(&a.worst_used().unwrap_or(0.0))
                    .unwrap_or(Ordering::Equal)
            }),
            Metric::Cost => used.sort_by(|a, b| {
                b.today_cost
                    .partial_cmp(&a.today_cost)
                    .unwrap_or(Ordering::Equal)
            }),
            Metric::Tokens => used.sort_by(|a, b| b.today_tokens.cmp(&a.today_tokens)),
        }
        let unused: Vec<&Subscription> = self
            .data
            .subscriptions
            .iter()
            .filter(|s| !s.is_used())
            .collect();

        let section = match self.metric {
            Metric::Percent => format!(
                "PLAN LIMITS · % {}",
                if self.remaining { "left" } else { "used" }
            ),
            Metric::Cost => "COST BY WINDOW".to_string(),
            Metric::Tokens => "TOKENS BY WINDOW".to_string(),
        };
        col = col.push(text(section).size(10));

        if used.is_empty() {
            col = col.push(text("No usage captured yet.").size(12));
        }
        for s in &used {
            col = col.push(self.subscription_view(s, &used));
        }

        if !unused.is_empty() {
            col = col.push(cosmic::widget::divider::horizontal::light());
            col = col.push(self.unused_view(&unused));
        }

        col = col.push(
            text(format!(
                "captured every 5 min · {} subscriptions",
                self.data.subscriptions.len()
            ))
            .size(10),
        );

        Element::from(self.core.applet.popup_container(col))
    }
}

impl cosmic::Application for Applet {
    type Executor = cosmic::SingleThreadExecutor;
    type Flags = ();
    type Message = Message;
    const APP_ID: &'static str = APP_ID;

    fn core(&self) -> &Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut Core {
        &mut self.core
    }

    fn init(core: Core, _flags: ()) -> (Self, Task<Message>) {
        let cfg = load_config();
        (
            Self {
                core,
                popup: None,
                data: load(),
                metric: Metric::from_str(&cfg.metric),
                remaining: cfg.remaining,
                show_unused: cfg.show_unused,
            },
            Task::none(),
        )
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    fn subscription(&self) -> cosmic::iced::Subscription<Message> {
        cosmic::iced::time::every(Duration::from_secs(REFRESH_SECS)).map(|_| Message::Refresh)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Refresh => {
                self.data = load();
            }
            Message::SetMetric(metric) => {
                self.metric = metric;
                save_config(self.metric, self.remaining, self.show_unused);
            }
            Message::SetRemaining(remaining) => {
                self.remaining = remaining;
                save_config(self.metric, self.remaining, self.show_unused);
            }
            Message::ToggleUnused => {
                self.show_unused = !self.show_unused;
                save_config(self.metric, self.remaining, self.show_unused);
            }
            Message::PopupClosed(id) => {
                if self.popup.as_ref() == Some(&id) {
                    self.popup = None;
                }
            }
            Message::Surface(a) => {
                return cosmic::task::message(cosmic::Action::Surface(a));
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, Message> {
        let have_popup = self.popup;
        let horizontal = matches!(
            self.core.applet.anchor,
            PanelAnchor::Top | PanelAnchor::Bottom
        );

        let size = self.core.applet.suggested_size(true).0;
        let handle = icon::from_name("speedometer-symbolic")
            .symbolic(true)
            .size(size)
            .into();
        let mut label = self.core.applet.text(self.panel_label());
        if self.metric == Metric::Percent {
            if let Some(used) = self.worst_used() {
                label = label.class(cosmic::theme::Text::Color(severity_color(used)));
            }
        }
        let icon_el = icon::icon(handle);

        // Stack vertically in left/right panels, side by side in top/bottom
        // panels, so the applet never overflows a narrow vertical panel.
        let content: Element<'_, Message> = if horizontal {
            row![icon_el, label]
                .spacing(4)
                .align_y(Alignment::Center)
                .into()
        } else {
            column![icon_el, label]
                .spacing(2)
                .align_x(Alignment::Center)
                .into()
        };

        let padding = self.core.applet.suggested_padding(true).0;
        let btn = button::custom(content)
            .padding(if horizontal {
                [0, padding]
            } else {
                [padding, 0]
            })
            .class(cosmic::theme::Button::AppletIcon)
            .on_press_with_rectangle(move |offset, bounds| {
                if let Some(id) = have_popup {
                    Message::Surface(destroy_popup(id))
                } else {
                    Message::Surface(app_popup::<Applet>(
                        |_| Default::default(),
                        move |state: &mut Applet| {
                            let new_id = Id::unique();
                            state.popup = Some(new_id);
                            let mut settings = state.core.applet.get_popup_settings(
                                state.core.main_window_id().unwrap(),
                                new_id,
                                None,
                                None,
                                None,
                            );
                            let (x, y) = match state.core.applet.anchor {
                                PanelAnchor::Top => (0, POPUP_GAP),
                                PanelAnchor::Bottom => (0, -POPUP_GAP),
                                PanelAnchor::Left => (POPUP_GAP, 0),
                                PanelAnchor::Right => (-POPUP_GAP, 0),
                            };
                            settings.positioner.offset = (x, y);
                            settings.positioner.anchor_rect = Rectangle {
                                x: (bounds.x - offset.x) as i32,
                                y: (bounds.y - offset.y) as i32,
                                width: bounds.width as i32,
                                height: bounds.height as i32,
                            };
                            settings
                        },
                        Some(Box::new(move |state: &Applet| {
                            Element::from(state.popup_view()).map(cosmic::Action::App)
                        })),
                    ))
                }
            });

        autosize::autosize(btn, AUTOSIZE_ID.clone()).into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Message> {
        "".into()
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}
