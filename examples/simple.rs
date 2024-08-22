use metrics::gauge;
use metrics_exporter_scope::ScopeBuilder;
use rtsc::time::interval;
use std::time::Duration;

/// Trait to round a float to a given number of digits (just for the demo)
trait RoundTo {
    fn round_to(self, digits: u32) -> Self;
}

impl RoundTo for f64 {
    #[allow(clippy::cast_precision_loss)]
    fn round_to(self, digits: u32) -> Self {
        let factor = 10u64.pow(digits) as f64;
        (self * factor).round() / factor
    }
}

#[allow(clippy::cast_precision_loss)]
fn main() {
    // build scope recorder
    ScopeBuilder::new().install().unwrap();
    // generate some metrics
    for (i, _) in interval(Duration::from_millis(10)).enumerate() {
        gauge!("~i%1000").set((i % 1000) as f64); // to scope, default plot, default color
        gauge!("~i_sin", "plot" => "trig", "color" => "orange")
            .set((i as f64 / 90.0).sin().round_to(3)); // to scope
        gauge!("~i_cos", "plot" => "trig", "color" => "#9cf")
            .set((i as f64 / 90.0).cos().round_to(3)); // to scope
        gauge!("~i_sin2", "plot" => "trig2", "color" => "yellow")
            .set((i as f64 / 180.0).sin().round_to(3)); // to scope
        gauge!("~i_cos2", "plot" => "trig2", "color" => "cyan")
            .set((i as f64 / 180.0).cos().round_to(3)); // to scope
        gauge!("~i%100", "plot" => "counts", "color" => "#336699").set((i % 100) as f64); // to scope
        gauge!("iteration").set(i as f64); // ignored
    }
}
