use std::collections::BTreeMap;

use clap::{
    builder::{TypedValueParser, ValueParserFactory},
    Parser, ValueEnum,
};

#[derive(Parser)]
pub struct Args {
    #[clap(help = "HOST[:PORT], the default port is 5001")]
    pub source: String,
    #[clap(
        short = 's',
        long,
        help = "Sampling interval in seconds",
        default_value = "0.1"
    )]
    pub sampling_interval: f64,
    #[clap(
        short = 't',
        long,
        help = "Network timeout in seconds",
        default_value = "10"
    )]
    pub timeout: u64,
    #[clap(long, help = "Hide legend")]
    pub hide_legend: bool,
    #[clap(
        short = 'w',
        long,
        help = "Time window in seconds",
        default_value = "10"
    )]
    pub time_window: f32,
    #[clap(long, help = "Chart columns", default_value = "2")]
    pub chart_cols: f32,
    #[clap(long, help = "Chart aspect ratio", default_value = "2")]
    pub chart_aspect: f32,
    #[clap(long, help = "Override system colors")]
    pub theme: Option<Theme>,
    #[clap(
        long = "y-range",
        value_name = "RANGE",
        help = "Predefined Y-range (plot=[min],[max])"
    )]
    pub predefined_y_range: Vec<PredefinedYRange>,
    #[clap(
        long = "sma",
        value_name = "WINDOW",
        help = "Predefined SMA (plot/metric=window or metric=window)"
    )]
    pub predefined_sma: Vec<PredefinedSma>,
    #[clap(
        long = "trigger",
        value_name = "TRIGGER",
        help = "Predefined Trigger (plot/metric=[below],[above] or metric=[below],[above])"
    )]
    pub predefined_trigger: Vec<PredefinedTrigger>,
}

pub trait ToPlotConfigMap {
    fn to_plot_config_map(&self) -> BTreeMap<String, PlotConfig>;
}

impl ToPlotConfigMap for Vec<PredefinedYRange> {
    fn to_plot_config_map(&self) -> BTreeMap<String, PlotConfig> {
        let mut map = BTreeMap::new();
        for PredefinedYRange { key, min, max } in self {
            map.insert(
                key.to_owned(),
                PlotConfig {
                    min: *min,
                    max: *max,
                },
            );
        }
        map
    }
}

pub trait ToSmaMap {
    fn to_sma_map(&self) -> BTreeMap<String, usize>;
}

impl ToSmaMap for Vec<PredefinedSma> {
    fn to_sma_map(&self) -> BTreeMap<String, usize> {
        let mut map = BTreeMap::new();
        for PredefinedSma { key, value } in self {
            map.insert(key.to_owned(), *value);
        }
        map
    }
}

pub trait ToTriggerMap {
    fn to_trigger_map(&self) -> BTreeMap<String, TriggerConfig>;
}

impl ToTriggerMap for Vec<PredefinedTrigger> {
    fn to_trigger_map(&self) -> BTreeMap<String, TriggerConfig> {
        let mut map = BTreeMap::new();
        for PredefinedTrigger { key, below, above } in self {
            map.insert(
                key.to_owned(),
                TriggerConfig {
                    below: *below,
                    above: *above,
                },
            );
        }
        map
    }
}

#[derive(Clone)]
pub struct PredefinedYRange {
    key: String,
    min: Option<f64>,
    max: Option<f64>,
}

impl ValueParserFactory for PredefinedYRange {
    type Parser = PredefinedYRangeParser;
    fn value_parser() -> Self::Parser {
        PredefinedYRangeParser
    }
}

#[derive(Clone)]
pub struct PredefinedYRangeParser;

impl TypedValueParser for PredefinedYRangeParser {
    type Value = PredefinedYRange;

    fn parse_ref(
        &self,
        _cmd: &clap::Command,
        _arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let v = value.to_str().ok_or_else(|| {
            clap::error::Error::raw(
                clap::error::ErrorKind::ValueValidation,
                "Invalid Y-range string",
            )
        })?;
        let mut sp = v.splitn(2, '=');
        let key = sp.next().unwrap();
        let value_str = sp.next().ok_or_else(|| {
            clap::error::Error::raw(
                clap::error::ErrorKind::ValueValidation,
                "Invalid Y-range - no value",
            )
        })?;
        let mut value_sp = value_str.splitn(2, ',');
        let min_str = value_sp.next().unwrap();
        let max_str = value_sp.next().ok_or_else(|| {
            clap::error::Error::raw(
                clap::error::ErrorKind::ValueValidation,
                "Invalid Y-range - no max value",
            )
        })?;
        let min = if min_str.is_empty() {
            None
        } else {
            Some(min_str.parse().map_err(|_| {
                clap::error::Error::raw(
                    clap::error::ErrorKind::ValueValidation,
                    "Invalid Y-range - min must be a float",
                )
            })?)
        };
        let max = if max_str.is_empty() {
            None
        } else {
            Some(max_str.parse().map_err(|_| {
                clap::error::Error::raw(
                    clap::error::ErrorKind::ValueValidation,
                    "Invalid Y-range - max must be a float",
                )
            })?)
        };
        Ok(PredefinedYRange {
            key: key.to_owned(),
            min,
            max,
        })
    }
}

#[derive(Clone)]
pub struct PredefinedSma {
    key: String,
    value: usize,
}

impl ValueParserFactory for PredefinedSma {
    type Parser = PredefinedSmaParser;
    fn value_parser() -> Self::Parser {
        PredefinedSmaParser
    }
}

#[derive(Clone)]
pub struct PredefinedSmaParser;

impl TypedValueParser for PredefinedSmaParser {
    type Value = PredefinedSma;

    fn parse_ref(
        &self,
        _cmd: &clap::Command,
        _arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let v = value.to_str().ok_or_else(|| {
            clap::error::Error::raw(
                clap::error::ErrorKind::ValueValidation,
                "Invalid SMA string",
            )
        })?;
        let mut sp = v.splitn(2, '=');
        let key = sp.next().unwrap();
        let value_str = sp.next().ok_or_else(|| {
            clap::error::Error::raw(
                clap::error::ErrorKind::ValueValidation,
                "Invalid SMA - no value",
            )
        })?;
        let value: usize = value_str.parse().map_err(|_| {
            clap::error::Error::raw(
                clap::error::ErrorKind::ValueValidation,
                "Invalid SMA - window must be an unsigned integer",
            )
        })?;
        Ok(PredefinedSma {
            key: key.to_owned(),
            value,
        })
    }
}

#[derive(Clone)]
pub struct PredefinedTrigger {
    key: String,
    below: Option<f64>,
    above: Option<f64>,
}

impl ValueParserFactory for PredefinedTrigger {
    type Parser = PredefinedTriggerParser;
    fn value_parser() -> Self::Parser {
        PredefinedTriggerParser
    }
}

#[derive(Clone)]
pub struct PredefinedTriggerParser;

impl TypedValueParser for PredefinedTriggerParser {
    type Value = PredefinedTrigger;

    fn parse_ref(
        &self,
        _cmd: &clap::Command,
        _arg: Option<&clap::Arg>,
        value: &std::ffi::OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let v = value.to_str().ok_or_else(|| {
            clap::error::Error::raw(
                clap::error::ErrorKind::ValueValidation,
                "Invalid Trigger string",
            )
        })?;
        let mut sp = v.splitn(2, '=');
        let key = sp.next().unwrap();
        let value_str = sp.next().ok_or_else(|| {
            clap::error::Error::raw(
                clap::error::ErrorKind::ValueValidation,
                "Invalid Trigger - no value",
            )
        })?;
        let mut value_sp = value_str.splitn(2, ',');
        let below_str = value_sp.next().unwrap();
        let above_str = value_sp.next().ok_or_else(|| {
            clap::error::Error::raw(
                clap::error::ErrorKind::ValueValidation,
                "Invalid Trigger - no above value",
            )
        })?;
        let below = if below_str.is_empty() {
            None
        } else {
            Some(below_str.parse().map_err(|_| {
                clap::error::Error::raw(
                    clap::error::ErrorKind::ValueValidation,
                    "Invalid Trigger - below must be a float",
                )
            })?)
        };
        let above = if above_str.is_empty() {
            None
        } else {
            Some(above_str.parse().map_err(|_| {
                clap::error::Error::raw(
                    clap::error::ErrorKind::ValueValidation,
                    "Invalid Trigger - above must be a float",
                )
            })?)
        };
        Ok(PredefinedTrigger {
            key: key.to_owned(),
            below,
            above,
        })
    }
}

#[derive(ValueEnum, Clone)]
pub enum Theme {
    #[clap(name = "dark")]
    Dark,
    #[clap(name = "light")]
    Light,
}

pub struct TriggerConfig {
    pub below: Option<f64>,
    pub above: Option<f64>,
}

pub struct PlotConfig {
    pub min: Option<f64>,
    pub max: Option<f64>,
}
