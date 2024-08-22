use metrics::gauge;
use metrics_exporter_prometheus::PrometheusBuilder;
use metrics_exporter_scope::ScopeBuilder;
use rtsc::time::interval;
use std::{thread, time::Duration};

#[allow(clippy::cast_precision_loss)]
fn main() {
    // build runtime for exporter prometheus
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (prometheus_exporter, prometheus_exporter_fut) = {
        let _g = runtime.enter();
        PrometheusBuilder::new().build().unwrap()
    };
    // build scope recorder with fallback to prometheus exporter
    ScopeBuilder::new()
        .with_fallback(Box::new(prometheus_exporter))
        .install()
        .unwrap();
    // start prometheus exporter
    thread::spawn(move || runtime.block_on(prometheus_exporter_fut));
    // generate some metrics
    for (i, _) in interval(Duration::from_millis(10)).enumerate() {
        gauge!("~test", "plot" => "1", "color" => "blue").set((i % 1000) as f64); // to scope
        gauge!("~test2", "plot" => "2", "color" => "red").set((i % 50) as f64); // to scope
        gauge!("test3").set(i as f64); // to fallback
    }
}
