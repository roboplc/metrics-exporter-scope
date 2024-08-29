use std::cmp;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use args::{
    Args, PlotConfig, ToPlotConfigMap as _, ToSmaMap as _, ToTriggerMap as _, TriggerConfig,
};
use atomic_float::AtomicF64;
use clap::Parser;
use egui::{Button, Color32, RichText, Ui};
use egui_plot::{Legend, Line, Plot, PlotPoint, PlotPoints};
use metrics_exporter_scope::Packet;
use once_cell::sync::Lazy;
use rtsc::data_policy::{DataDeliveryPolicy, DeliveryPolicy};

mod args;
mod client;

type EventSender = rtsc::policy_channel::Sender<Event, parking_lot::RawMutex, parking_lot::Condvar>;
type EventReceiver =
    rtsc::policy_channel::Receiver<Event, parking_lot::RawMutex, parking_lot::Condvar>;

const DATA_BUF_SIZE: usize = 100_000;
const UI_DELAY: Duration = Duration::from_millis(50);

const MAX_TIME_WINDOW: f32 = 600.0;

enum Event {
    Connect,
    Disconnect,
    Packet(Packet),
}

impl DataDeliveryPolicy for Event {
    fn delivery_policy(&self) -> DeliveryPolicy {
        match self {
            Event::Connect | Event::Disconnect => DeliveryPolicy::Always,
            Event::Packet(_) => DeliveryPolicy::Latest,
        }
    }
}

static COLORS: Lazy<BTreeMap<String, Color32>> = Lazy::new(|| {
    let mut colors = BTreeMap::new();
    colors.insert("red".to_owned(), Color32::RED);
    colors.insert("green".to_owned(), Color32::GREEN);
    colors.insert("blue".to_owned(), Color32::BLUE);
    colors.insert("yellow".to_owned(), Color32::YELLOW);
    colors.insert("cyan".to_owned(), Color32::from_rgb(0, 255, 255));
    colors.insert("magenta".to_owned(), Color32::from_rgb(255, 0, 255));
    colors.insert("orange".to_owned(), Color32::from_rgb(255, 105, 0));
    colors.insert("white".to_owned(), Color32::WHITE);
    colors.insert("black".to_owned(), Color32::from_rgb(0, 0, 0));
    colors
});

fn parse_color(color: &str) -> Option<Color32> {
    if let Some(color) = COLORS.get(color) {
        Some(*color)
    } else if let Some(c) = color.strip_prefix('#') {
        match c.len() {
            3 => {
                let r = u8::from_str_radix(&c[0..1].repeat(2), 16).ok()?;
                let g = u8::from_str_radix(&c[1..2].repeat(2), 16).ok()?;
                let b = u8::from_str_radix(&c[2..3].repeat(2), 16).ok()?;
                Some(Color32::from_rgb(r, g, b))
            }
            6 => {
                let r = u8::from_str_radix(&c[0..2], 16).ok()?;
                let g = u8::from_str_radix(&c[2..4], 16).ok()?;
                let b = u8::from_str_radix(&c[4..6], 16).ok()?;
                Some(Color32::from_rgb(r, g, b))
            }
            _ => None,
        }
    } else {
        None
    }
}

fn main() {
    let args = Args::parse();
    let mut source = args.source.clone();
    if !source.contains(':') {
        source = format!("{}:5001", source);
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([640.0, 480.0]),
        ..Default::default()
    };
    let (tx, rx) =
        rtsc::policy_channel::bounded::<Event, parking_lot::RawMutex, parking_lot::Condvar>(
            DATA_BUF_SIZE,
        );
    let source_c = source.clone();
    let timeout = Duration::from_secs(args.timeout);
    let sampling_interval = Duration::from_secs_f64(args.sampling_interval);
    thread::spawn(move || {
        client::reader(&source_c, tx, sampling_interval, timeout);
    });
    // make args static
    let args = Box::leak(Box::new(args));
    eframe::run_native(
        &format!("{} - metrics-scope", source),
        options,
        Box::new(|cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            if let Some(theme) = args.theme.as_ref() {
                match theme {
                    args::Theme::Dark => cc.egui_ctx.set_visuals(egui::Visuals::dark()),
                    args::Theme::Light => cc.egui_ctx.set_visuals(egui::Visuals::light()),
                }
            }
            Ok(Box::new(Scope {
                rx,
                data: <_>::default(),
                plots: <_>::default(),
                plot_settings: <_>::default(),
                colors: <_>::default(),
                paused: false,
                need_reset: false,
                show_legend: !args.hide_legend,
                time_window: args.time_window,
                chart_cols: args.chart_cols,
                aspect: args.chart_aspect,
                sma_selected_plot: None,
                sma_selected_metric: None,
                sma_selected_value: String::new(),
                trigger_selected_plot: None,
                trigger_selected_metric: None,
                trigger_selected_value_below: String::new(),
                trigger_selected_value_above: String::new(),
                range_selected_plot: None,
                range_selected_value_min: String::new(),
                range_selected_value_max: String::new(),
                triggered: None,
                sampling_interval_ns: Duration::from_secs_f64(args.sampling_interval)
                    .as_nanos()
                    .try_into()
                    .unwrap(),
                connected: false,
                source: args.source.clone(),
                predefined_smas: args.predefined_sma.to_sma_map(),
                predefined_triggers: args.predefined_trigger.to_trigger_map(),
                predefined_plots: args.predefined_y_range.to_plot_config_map(),
            }))
        }),
    )
    .expect("Failed to run UI");
}

#[allow(clippy::struct_excessive_bools)]
struct Scope {
    rx: EventReceiver,
    data: BTreeMap<String, Vec<f64>>,
    plots: BTreeMap<String, BTreeSet<Arc<Metric>>>,
    plot_settings: BTreeMap<String, PlotSettings>,
    colors: BTreeMap<String, Color32>,
    paused: bool,
    need_reset: bool,
    show_legend: bool,
    time_window: f32,
    chart_cols: f32,
    aspect: f32,
    sma_selected_plot: Option<String>,
    sma_selected_metric: Option<Arc<Metric>>,
    sma_selected_value: String,
    trigger_selected_plot: Option<String>,
    trigger_selected_metric: Option<Arc<Metric>>,
    trigger_selected_value_below: String,
    trigger_selected_value_above: String,
    range_selected_plot: Option<String>,
    range_selected_value_min: String,
    range_selected_value_max: String,
    triggered: Option<Triggered>,
    sampling_interval_ns: u64,
    connected: bool,
    source: String,
    predefined_smas: BTreeMap<String, usize>,
    predefined_triggers: BTreeMap<String, TriggerConfig>,
    predefined_plots: BTreeMap<String, PlotConfig>,
}

struct PlotSettings {
    min_y: AtomicF64,
    max_y: AtomicF64,
}

impl PlotSettings {
    fn new() -> Self {
        Self {
            min_y: AtomicF64::new(f64::NAN),
            max_y: AtomicF64::new(f64::NAN),
        }
    }
    fn get_min_y(&self) -> Option<f64> {
        let val = self.min_y.load(Ordering::Relaxed);
        if val.is_nan() {
            None
        } else {
            Some(val)
        }
    }
    fn get_max_y(&self) -> Option<f64> {
        let val = self.max_y.load(Ordering::Relaxed);
        if val.is_nan() {
            None
        } else {
            Some(val)
        }
    }
    fn set_min_y(&self, value: Option<f64>) {
        if let Some(value) = value {
            self.min_y.store(value, Ordering::Relaxed);
        } else {
            self.min_y.store(f64::NAN, Ordering::Relaxed);
        }
    }
    fn set_max_y(&self, value: Option<f64>) {
        if let Some(value) = value {
            self.max_y.store(value, Ordering::Relaxed);
        } else {
            self.max_y.store(f64::NAN, Ordering::Relaxed);
        }
    }
}

struct Triggered {
    at: f64,
    by: String,
    below_above: TriggeredKind,
}

impl Triggered {
    fn below(at: f64, by: impl AsRef<str>) -> Self {
        Self {
            at,
            by: by.as_ref().to_owned(),
            below_above: TriggeredKind::Below,
        }
    }
    fn above(at: f64, by: impl AsRef<str>) -> Self {
        Self {
            at,
            by: by.as_ref().to_owned(),
            below_above: TriggeredKind::Above,
        }
    }
}

#[derive(Eq, PartialEq, Clone, Copy)]
enum TriggeredKind {
    Below,
    Above,
}

struct Metric {
    name: String,
    sma_window: AtomicUsize,
    trigger_below: AtomicF64,
    trigger_above: AtomicF64,
}

impl Metric {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            sma_window: AtomicUsize::new(0),
            trigger_below: AtomicF64::new(f64::NAN),
            trigger_above: AtomicF64::new(f64::NAN),
        }
    }
    fn get_sma(&self) -> usize {
        self.sma_window.load(Ordering::Relaxed)
    }
    fn set_sma(&self, value: usize) {
        self.sma_window.store(value, Ordering::Relaxed);
    }
    fn get_trigger_below(&self) -> Option<f64> {
        let val = self.trigger_below.load(Ordering::Relaxed);
        if val.is_nan() {
            None
        } else {
            Some(val)
        }
    }
    fn get_trigger_above(&self) -> Option<f64> {
        let val = self.trigger_above.load(Ordering::Relaxed);
        if val.is_nan() {
            None
        } else {
            Some(val)
        }
    }
    fn set_trigger_below(&self, value: Option<f64>) {
        if let Some(value) = value {
            self.trigger_below.store(value, Ordering::Relaxed);
        } else {
            self.trigger_below.store(f64::NAN, Ordering::Relaxed);
        }
    }
    fn set_trigger_above(&self, value: Option<f64>) {
        if let Some(value) = value {
            self.trigger_above.store(value, Ordering::Relaxed);
        } else {
            self.trigger_above.store(f64::NAN, Ordering::Relaxed);
        }
    }
}

impl PartialOrd for Metric {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        Some(self.name.cmp(&other.name))
    }
}

impl Ord for Metric {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.name.cmp(&other.name)
    }
}

impl Eq for Metric {}

impl PartialEq for Metric {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Scope {
    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Connect => {
                self.data.clear();
                //self.plots.clear();
                self.colors.clear();
                self.connected = true;
            }
            Event::Disconnect => {
                self.connected = false;
            }
            Event::Packet(Packet::Snapshot(mut snapshot)) => {
                let max_time_window = Duration::from_secs_f32(MAX_TIME_WINDOW);
                let max_data_ponts = usize::try_from(
                    u64::try_from(max_time_window.as_nanos()).unwrap() / self.sampling_interval_ns,
                )
                .unwrap();
                let ts_vec = self.data.entry(String::new()).or_default();
                ts_vec.push(snapshot.ts().as_secs_f64());
                if ts_vec.len() > max_data_ponts {
                    ts_vec.drain(0..(ts_vec.len() - max_data_ponts));
                }
                for (n, v) in snapshot.take_data() {
                    let data_vec = self.data.entry(n).or_default();
                    data_vec.push(v);
                    if data_vec.len() > max_data_ponts {
                        data_vec.drain(0..(data_vec.len() - max_data_ponts));
                    }
                }
            }
            Event::Packet(Packet::Info(info)) => {
                for (name, m) in info.metrics() {
                    let metric = Arc::new(Metric::new(name));
                    let (plot, tag) = if let Some(plot) = m.labels().get("plot") {
                        if self
                            .plots
                            .entry(plot.to_owned())
                            .or_default()
                            .insert(metric.clone())
                        {
                            (Some(plot.to_owned()), Some(format!("{}/{}", plot, name)))
                        } else {
                            (None, None)
                        }
                    } else if self
                        .plots
                        .entry(name.to_owned())
                        .or_default()
                        .insert(metric.clone())
                    {
                        (Some(name.to_owned()), Some(name.to_owned()))
                    } else {
                        (None, None)
                    };
                    if let Some(plot) = plot {
                        let plot_settings =
                            if let Some(plot_config) = self.predefined_plots.get(&plot) {
                                let settings = PlotSettings::new();
                                settings.set_min_y(plot_config.min);
                                settings.set_max_y(plot_config.max);
                                settings
                            } else {
                                PlotSettings::new()
                            };
                        self.plot_settings.insert(plot, plot_settings);
                    }
                    if let Some(tag) = tag {
                        if let Some(sma) = self.predefined_smas.get(&tag) {
                            metric.set_sma(*sma);
                        }
                        if let Some(triggers) = self.predefined_triggers.get(&tag) {
                            if let Some(below) = triggers.below {
                                metric.set_trigger_below(Some(below));
                            }
                            if let Some(above) = triggers.above {
                                metric.set_trigger_above(Some(above));
                            }
                        }
                    }
                    if let Some(color) = m.labels().get("color") {
                        if let Some(color) = parse_color(color) {
                            self.colors.insert(name.to_owned(), color);
                        } else {
                            eprintln!("Invalid color: {}", color);
                        }
                    }
                }
            }
        }
    }

    fn process_global_keys(&mut self, ui: &mut Ui) {
        if ui.input(|i| i.key_pressed(egui::Key::L)) {
            self.show_legend = !self.show_legend;
        }
        if ui.input(|i| i.key_pressed(egui::Key::F5)) {
            self.need_reset = true;
            self.triggered = None;
        }
        if ui.input(|i| i.key_pressed(egui::Key::P)) {
            self.paused = !self.paused;
            self.triggered = None;
        }
    }

    fn show_sma_toolbar(&mut self, ui: &mut Ui) {
        egui::ComboBox::from_label("SMA")
            .selected_text(self.sma_selected_plot.as_deref().unwrap_or("-"))
            .show_ui(ui, |ui| {
                if self.sma_selected_plot.is_some() && ui.selectable_label(false, "-").clicked() {
                    self.sma_selected_plot = None;
                    self.sma_selected_metric = None;
                }
                for plot in self.plots.keys() {
                    if ui.selectable_label(false, plot).clicked() {
                        self.sma_selected_plot = Some(plot.clone());
                        self.sma_selected_metric = None;
                    }
                }
            });
        if let Some(plot) = self.sma_selected_plot.as_ref() {
            if let Some(metrics) = self.plots.get(plot).as_mut() {
                egui::ComboBox::from_label("SMA for metric")
                    .selected_text(
                        self.sma_selected_metric
                            .as_ref()
                            .map(|m| m.name.clone())
                            .unwrap_or_default(),
                    )
                    .show_ui(ui, |ui| {
                        for metric in *metrics {
                            if ui.selectable_label(false, &metric.name).clicked() {
                                self.sma_selected_metric = Some(metric.clone());
                                self.sma_selected_value = metric.get_sma().to_string();
                            }
                        }
                    });
            }
            if let Some(metric) = self.sma_selected_metric.as_ref() {
                if ui
                    .add(egui::widgets::TextEdit::singleline(
                        &mut self.sma_selected_value,
                    ))
                    .changed()
                {
                    metric.set_sma(self.sma_selected_value.parse().unwrap_or_default());
                }
            }
        }
        ui.end_row();
    }

    fn show_trigger_toolbar(&mut self, ui: &mut Ui) {
        egui::ComboBox::from_label("Trigger")
            .selected_text(self.trigger_selected_plot.as_deref().unwrap_or("-"))
            .show_ui(ui, |ui| {
                if self.trigger_selected_plot.is_some() && ui.selectable_label(false, "-").clicked()
                {
                    self.trigger_selected_plot = None;
                    self.trigger_selected_metric = None;
                }
                for plot in self.plots.keys() {
                    if ui.selectable_label(false, plot).clicked() {
                        self.trigger_selected_plot = Some(plot.clone());
                        self.trigger_selected_metric = None;
                    }
                }
            });
        if let Some(plot) = self.trigger_selected_plot.as_ref() {
            if let Some(metrics) = self.plots.get(plot).as_mut() {
                egui::ComboBox::from_label("Trigger for metric")
                    .selected_text(
                        self.trigger_selected_metric
                            .as_ref()
                            .map(|m| m.name.clone())
                            .unwrap_or_default(),
                    )
                    .show_ui(ui, |ui| {
                        for metric in *metrics {
                            if ui.selectable_label(false, &metric.name).clicked() {
                                self.trigger_selected_metric = Some(metric.clone());
                                self.trigger_selected_value_below = metric
                                    .get_trigger_below()
                                    .map(|v| v.to_string())
                                    .unwrap_or_default();
                                self.trigger_selected_value_above = metric
                                    .get_trigger_above()
                                    .map(|v| v.to_string())
                                    .unwrap_or_default();
                            }
                        }
                    });
            }
            if let Some(metric) = self.trigger_selected_metric.as_ref() {
                ui.label("below");
                if ui
                    .add(egui::widgets::TextEdit::singleline(
                        &mut self.trigger_selected_value_below,
                    ))
                    .changed()
                {
                    metric.set_trigger_below(self.trigger_selected_value_below.parse().ok());
                    if let Some(ref tr) = self.triggered {
                        if tr.by == metric.name && tr.below_above == TriggeredKind::Below {
                            self.triggered = None;
                        }
                    }
                }
                ui.label("above");
                if ui
                    .add(egui::widgets::TextEdit::singleline(
                        &mut self.trigger_selected_value_above,
                    ))
                    .changed()
                {
                    metric.set_trigger_above(self.trigger_selected_value_above.parse().ok());
                    if let Some(ref tr) = self.triggered {
                        if tr.by == metric.name && tr.below_above == TriggeredKind::Above {
                            self.triggered = None;
                        }
                    }
                }
            }
        }
        if let Some(ref tr) = self.triggered {
            let mut text = RichText::new(format!("TRIG {}", tr.by)).color(Color32::BLACK);
            if self.paused {
                text = text.background_color(Color32::GRAY);
            } else {
                text = text.background_color(Color32::YELLOW);
            }
            ui.label(text);
        }
        ui.end_row();
    }

    fn show_range_toolbar(&mut self, ui: &mut Ui) {
        egui::ComboBox::from_label("Y-Range")
            .selected_text(self.range_selected_plot.as_deref().unwrap_or("-"))
            .show_ui(ui, |ui| {
                if self.range_selected_plot.is_some() && ui.selectable_label(false, "-").clicked() {
                    self.range_selected_plot = None;
                }
                for plot in self.plots.keys() {
                    if ui.selectable_label(false, plot).clicked() {
                        let plot_settings = self.plot_settings.get(plot).unwrap();
                        self.range_selected_plot = Some(plot.clone());
                        self.range_selected_value_min = plot_settings
                            .get_min_y()
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                        self.range_selected_value_max = plot_settings
                            .get_max_y()
                            .map(|v| v.to_string())
                            .unwrap_or_default();
                    }
                }
            });
        if let Some(plot) = self.range_selected_plot.as_ref() {
            ui.label("min");
            if ui
                .add(egui::widgets::TextEdit::singleline(
                    &mut self.range_selected_value_min,
                ))
                .changed()
            {
                self.plot_settings
                    .get(plot)
                    .unwrap()
                    .set_min_y(self.range_selected_value_min.parse().ok());
            }
            ui.label("max");
            if ui
                .add(egui::widgets::TextEdit::singleline(
                    &mut self.range_selected_value_max,
                ))
                .changed()
            {
                self.plot_settings
                    .get(plot)
                    .unwrap()
                    .set_max_y(self.range_selected_value_max.parse().ok());
            }
        }
        ui.end_row();
    }

    fn show_common_controls(&mut self, ui: &mut Ui) {
        ui.add(
            egui::Slider::new(&mut self.time_window, 1.0..=MAX_TIME_WINDOW)
                .text("Time window")
                .step_by(1.0)
                .integer()
                .logarithmic(true),
        );
        ui.checkbox(&mut self.show_legend, "Legend (L)");
        if ui.add(Button::new("Reset (F5)")).clicked() {
            self.need_reset = true;
            self.triggered = None;
        }
        if self.paused {
            if ui.add(Button::new("Resume (P)")).clicked() {
                self.paused = false;
                self.triggered = None;
            }
        } else if ui.add(Button::new("Pause (P)")).clicked() {
            self.paused = true;
            self.triggered = None;
        }
        ui.end_row();
        ui.add(
            egui::Slider::new(&mut self.chart_cols, 1.0..=10.0)
                .text("Cols")
                .integer(),
        );
        ui.add(
            egui::Slider::new(&mut self.aspect, 1.0..=5.0)
                .text("Aspect")
                .step_by(0.1),
        );
        ui.end_row();
    }

    #[allow(clippy::too_many_lines, clippy::cast_precision_loss)]
    fn show_charts(&mut self, ui: &mut Ui, ts_vec: Vec<f64>, data_points: usize) {
        let chart_width = ui.available_width() / self.chart_cols - 10.0;
        let plots: Vec<_> = self.plots.iter().filter(|(_, v)| !v.is_empty()).collect();
        let mut ts_vec_axis = vec![];
        for i in (0..data_points).rev() {
            ts_vec_axis.push(-(i as f64 * self.sampling_interval_ns as f64 / 1_000_000_000.0));
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        for plot_chunk in plots.chunks(self.chart_cols as usize) {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                for (plot, metrics) in plot_chunk {
                    let mut plot_name = String::new();
                    for metric in *metrics {
                        if plot_name.is_empty() && metric.name != **plot {
                            plot_name.push_str(&format!("{} ", plot));
                        }
                        if let Some(data) = self.data.get(&metric.name) {
                            if let Some(last) = data.last() {
                                plot_name.push_str(&format!("{}={} ", metric.name, last));
                            }
                        }
                    }
                    let mut chart_plot = Plot::new(plot)
                        .view_aspect(self.aspect)
                        .x_axis_label(plot_name)
                        .label_formatter(|name, value| {
                            if name.is_empty() {
                                format!("t={}\n{}", value.x, value.y)
                            } else {
                                format!("t={}\n{}={}", value.x, name, value.y)
                            }
                        })
                        .width(chart_width)
                        .link_axis("scope", true, false)
                        .link_cursor("scope", true, false);
                    if self.need_reset {
                        chart_plot = chart_plot.reset();
                    }
                    if self.show_legend {
                        let legend = Legend::default();
                        chart_plot = chart_plot.legend(legend);
                    };
                    let plot_settings = self.plot_settings.get(*plot).unwrap();
                    if let Some(min_y) = plot_settings.get_min_y() {
                        chart_plot = chart_plot.include_y(min_y);
                    }
                    if let Some(max_y) = plot_settings.get_max_y() {
                        chart_plot = chart_plot.include_y(max_y);
                    }
                    chart_plot.show(ui, |plot_ui| {
                        for metric in *metrics {
                            let mut data = if let Some(d) = self.data.get(&metric.name) {
                                if self.triggered.is_none() {
                                    if let Some(last) = d.last() {
                                        if let Some(min) = metric.get_trigger_below() {
                                            if *last <= min {
                                                self.triggered = Some(Triggered::below(
                                                    *ts_vec.last().unwrap(),
                                                    &metric.name,
                                                ));
                                            }
                                        }
                                        if let Some(max) = metric.get_trigger_above() {
                                            if *last >= max {
                                                self.triggered = Some(Triggered::above(
                                                    *ts_vec.last().unwrap(),
                                                    &metric.name,
                                                ));
                                            }
                                        }
                                    }
                                }
                                match d.len().cmp(&data_points) {
                                    cmp::Ordering::Less => {
                                        let to_insert = data_points - d.len();
                                        let mut data = Vec::with_capacity(data_points);
                                        data.resize(to_insert, f64::NAN);
                                        data.extend(d);
                                        data
                                    }
                                    cmp::Ordering::Equal => d.clone(),
                                    cmp::Ordering::Greater => d[d.len() - data_points..].to_vec(),
                                }
                            } else {
                                vec![f64::NAN; data_points]
                            };
                            if let Some(min_y) = plot_settings.get_min_y() {
                                for entry in &mut data {
                                    if *entry < min_y {
                                        *entry = f64::NAN;
                                    }
                                }
                            }
                            if let Some(max_y) = plot_settings.get_max_y() {
                                for entry in &mut data {
                                    if *entry > max_y {
                                        *entry = f64::NAN;
                                    }
                                }
                            }
                            let sma_window = metric.get_sma();
                            if sma_window > 0 {
                                let sma = data
                                    .windows(sma_window)
                                    .map(|w| w.iter().sum::<f64>() / w.len() as f64)
                                    .collect::<Vec<_>>();
                                let pp = PlotPoints::Owned(
                                    sma.into_iter()
                                        .zip(ts_vec_axis.clone())
                                        .skip(sma_window - 1)
                                        .map(|(d, ts)| PlotPoint::new(ts, d))
                                        .collect(),
                                );
                                plot_ui.line(
                                    Line::new(pp)
                                        .name(format!("SMA {}", metric.name))
                                        .style(egui_plot::LineStyle::Dotted { spacing: 5.0 }),
                                );
                            }
                            let pp = PlotPoints::Owned(
                                data.into_iter()
                                    .zip(ts_vec_axis.clone())
                                    .map(|(d, ts)| PlotPoint::new(ts, d))
                                    .collect(),
                            );
                            let mut line = Line::new(pp).name(&metric.name);
                            if let Some(color) = self.colors.get(&metric.name) {
                                line = line.color(*color);
                            }
                            plot_ui.line(line);
                            if let Some(trigger_min) = metric.get_trigger_below() {
                                plot_ui.line(
                                    Line::new(PlotPoints::Owned(vec![
                                        PlotPoint::new(
                                            ts_vec_axis.first().copied().unwrap_or_default(),
                                            trigger_min,
                                        ),
                                        PlotPoint::new(
                                            ts_vec_axis.last().copied().unwrap_or_default(),
                                            trigger_min,
                                        ),
                                    ]))
                                    .color(Color32::from_rgba_premultiplied(149, 80, 45, 20))
                                    .style(egui_plot::LineStyle::Dashed { length: 10.0 })
                                    .name(format!("TrB {}", metric.name)),
                                );
                            }
                            if let Some(trigger_max) = metric.get_trigger_above() {
                                plot_ui.line(
                                    Line::new(PlotPoints::Owned(vec![
                                        PlotPoint::new(
                                            ts_vec_axis.first().copied().unwrap_or_default(),
                                            trigger_max,
                                        ),
                                        PlotPoint::new(
                                            ts_vec_axis.last().copied().unwrap_or_default(),
                                            trigger_max,
                                        ),
                                    ]))
                                    .color(Color32::from_rgba_premultiplied(149, 40, 45, 20))
                                    .style(egui_plot::LineStyle::Dashed { length: 10.0 })
                                    .name(format!("TrA {}", metric.name)),
                                );
                            }
                        }
                    });
                }
            });
        }
    }
}

impl eframe::App for Scope {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let time_window = Duration::from_secs_f32(self.time_window);
        if self.paused {
            thread::sleep(UI_DELAY);
        } else {
            let mut received = false;
            while let Ok(event) = self.rx.try_recv() {
                received = true;
                self.handle_event(event);
            }
            if !received {
                thread::sleep(UI_DELAY);
            }
        }
        let Some(full_ts_vec) = self.data.get("") else {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.label("Connecting...");
            });
            thread::sleep(UI_DELAY);
            ctx.request_repaint();
            return;
        };
        let data_points = usize::try_from(
            u64::try_from(time_window.as_nanos()).unwrap() / self.sampling_interval_ns,
        )
        .unwrap();
        let mut ts_vec;
        match full_ts_vec.len().cmp(&data_points) {
            cmp::Ordering::Less => {
                let ts = full_ts_vec.first().copied().unwrap_or_default();
                let to_insert = data_points - full_ts_vec.len();
                ts_vec = Vec::with_capacity(data_points);
                #[allow(clippy::cast_precision_loss)]
                for i in (0..to_insert).rev() {
                    ts_vec.push(
                        ts - (i as f64) * (self.sampling_interval_ns as f64) / 1_000_000_000.0,
                    );
                }
                ts_vec.extend(full_ts_vec);
            }
            cmp::Ordering::Equal => {
                ts_vec = full_ts_vec.clone();
            }
            cmp::Ordering::Greater => {
                ts_vec = full_ts_vec[full_ts_vec.len() - data_points..].to_vec();
            }
        }
        if let Some(ref tr) = self.triggered {
            let ts_half = ts_vec.len() / 2;
            if let Some(ts) = ts_vec.get(ts_half) {
                if tr.at <= *ts {
                    self.paused = true;
                }
            }
        }
        egui::CentralPanel::default().show(ctx, |ui| {
            self.process_global_keys(ui);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::TOP), |ui| {
                    ui.add(
                        egui::Image::new(egui::include_image!("../assets/bma.svg"))
                            .rounding(5.0)
                            .max_width(48.0)
                            .shrink_to_fit(),
                    );
                    egui::Grid::new("status").show(ui, |ui| {
                        ui.label(&self.source);
                        ui.end_row();
                        let text = if self.connected {
                            RichText::new("ONLINE")
                                .color(Color32::WHITE)
                                .background_color(Color32::DARK_GREEN)
                        } else {
                            RichText::new("OFFLINE")
                                .color(Color32::WHITE)
                                .background_color(Color32::DARK_RED)
                        };
                        ui.label(text);
                    });
                });
                egui::Grid::new("toolbar").show(ui, |ui| {
                    self.show_sma_toolbar(ui);
                    self.show_trigger_toolbar(ui);
                    self.show_range_toolbar(ui);
                });
            });
            egui::Grid::new("common_controls").show(ui, |ui| {
                self.show_common_controls(ui);
            });
            egui::ScrollArea::both().show(ui, |ui| {
                self.show_charts(ui, ts_vec, data_points);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::RIGHT), |ui| {
                    let text = RichText::new("RoboPLC Metrics Scope © Bohemia Automation")
                        .color(Color32::DARK_GRAY);
                    ui.label(text);
                });
            });
        });
        self.need_reset = false;
        ctx.request_repaint();
    }
}
