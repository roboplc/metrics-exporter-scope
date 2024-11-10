#![ doc = include_str!( concat!( env!( "CARGO_MANIFEST_DIR" ), "/", "README.md" ) ) ]
#![deny(missing_docs)]
#[cfg(feature = "msrv")]
extern crate metrics_legacy as metrics;
#[cfg(feature = "msrv")]
extern crate metrics_util_legacy as metrics_util;

use std::{
    collections::BTreeMap,
    net::{SocketAddr, TcpListener, TcpStream},
    num::TryFromIntError,
    sync::{atomic::Ordering, Arc},
    thread,
    time::Duration,
};

use bma_ts::Monotonic;
use metrics::{Key, Recorder};
use metrics_util::registry::{AtomicStorage, GenerationalStorage, Registry};
use rtsc::time::interval;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

/// Crate error type
#[derive(thiserror::Error, Debug)]
pub enum Error {
    /// I/O errors
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Data serialization errors
    #[error("encode error: {0}")]
    Encode(#[from] rmp_serde::encode::Error),
    /// Data deserialization errors
    #[error("decode error: {0}")]
    Decode(#[from] rmp_serde::decode::Error),
    /// Recorder setup errors
    #[error("set recorder error: {0}")]
    SetRecorder(#[from] metrics::SetRecorderError<ScopeRecorder>),
    /// Other errors
    #[error("{0}")]
    Other(String),
}

impl From<TryFromIntError> for Error {
    fn from(error: TryFromIntError) -> Self {
        Self::Other(error.to_string())
    }
}

const CLIENT_CHAT_TIMEOUT: Duration = Duration::from_secs(60);

const SEND_INFO_INTERVAL: Duration = Duration::from_secs(5);

const SERVER_THREAD_NAME: &str = "MScopeSrv";

/// Communication protocol
pub mod protocol {

    /// Current protocol version
    pub const VERSION: u16 = 1;

    use std::io::{Read, Write};

    use crate::{ClientSettings, Error, Packet};
    use serde::{Deserialize, Serialize};

    /// Read protocol version from a stream
    pub fn read_version<R>(mut stream: R) -> Result<u16, Error>
    where
        R: Read,
    {
        let buf = &mut [0u8; 2];
        stream.read_exact(buf)?;
        Ok(u16::from_le_bytes(*buf))
    }

    /// Write protocol version to a stream
    pub fn write_version<W>(mut stream: W) -> Result<(), Error>
    where
        W: Write,
    {
        stream.write_all(&VERSION.to_le_bytes())?;
        Ok(())
    }

    /// Read a packet from a stream
    pub fn read_packet<R>(stream: R) -> Result<Packet, Error>
    where
        R: Read,
    {
        read(stream)
    }

    /// Write a packet to a stream
    pub fn write_packet<W>(stream: W, packet: &Packet) -> Result<(), Error>
    where
        W: Write,
    {
        write(stream, packet)
    }

    /// Read client settings from a stream
    pub fn read_client_settings<R>(stream: R) -> Result<ClientSettings, Error>
    where
        R: Read,
    {
        read(stream)
    }

    /// Write client settings to a stream
    pub fn write_client_settings<W>(stream: W, settings: &ClientSettings) -> Result<(), Error>
    where
        W: Write,
    {
        write(stream, settings)
    }

    fn write<D, W>(mut stream: W, data: D) -> Result<(), Error>
    where
        W: Write,
        D: Serialize,
    {
        let data = rmp_serde::to_vec_named(&data)?;
        stream.write_all(&u32::try_from(data.len())?.to_le_bytes())?;
        stream.write_all(&data)?;
        Ok(())
    }

    fn read<R, D>(mut stream: R) -> Result<D, Error>
    where
        R: Read,
        D: for<'de> Deserialize<'de>,
    {
        let buf = &mut [0u8; 4];
        stream.read_exact(buf)?;
        let len = usize::try_from(u32::from_le_bytes(*buf))?;
        let mut buf = vec![0u8; len];
        stream.read_exact(&mut buf)?;
        Ok(rmp_serde::from_slice(&buf)?)
    }
}

/// Communication packets
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(untagged)]
pub enum Packet {
    /// Information packet (metrics metadata)
    Info(Info),
    /// Snapshot packet (metrics data)
    Snapshot(Snapshot),
}

/// Client settings
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct ClientSettings {
    sampling_interval: u64,
}

impl ClientSettings {
    /// # Panics
    ///
    /// Panics if the duration is too large to fit into a u64.
    pub fn new(sampling_interval: Duration) -> Self {
        Self {
            sampling_interval: u64::try_from(sampling_interval.as_nanos()).unwrap(),
        }
    }
}

/// Information packet
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Info {
    metrics: BTreeMap<String, MetricInfo>,
}

impl Info {
    /// Get metrics metadata map
    pub fn metrics(&self) -> &BTreeMap<String, MetricInfo> {
        &self.metrics
    }
}

/// Metrics metadata
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct MetricInfo {
    labels: BTreeMap<String, String>,
}

impl MetricInfo {
    /// Metric labels map
    pub fn labels(&self) -> &BTreeMap<String, String> {
        &self.labels
    }
}

/// Snapshot packet
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Snapshot {
    t: Monotonic,
    d: BTreeMap<String, f64>,
}

impl Snapshot {
    /// Snapshot timestamp (monotonic, relative to the communication start)
    pub fn ts(&self) -> Monotonic {
        self.t
    }
    /// Snapshot data map (metric name -> value)
    pub fn data(&self) -> &BTreeMap<String, f64> {
        &self.d
    }
    /// Snapshot data map mutable (metric name -> value)
    pub fn data_mut(&mut self) -> &mut BTreeMap<String, f64> {
        &mut self.d
    }
    /// Take snapshot data map
    pub fn take_data(&mut self) -> BTreeMap<String, f64> {
        std::mem::take(&mut self.d)
    }
}

/// Exporter builder
pub struct ScopeBuilder {
    addr: SocketAddr,
    fallback: Option<Box<dyn Recorder + Send + Sync>>,
}

impl Default for ScopeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ScopeBuilder {
    /// Create a new exporter builder
    pub fn new() -> Self {
        Self {
            addr: (std::net::Ipv4Addr::UNSPECIFIED, 5001).into(),
            fallback: None,
        }
    }
    /// Set the server listening address and port
    pub fn with_addr<A: Into<SocketAddr>>(mut self, addr: A) -> Self {
        self.addr = addr.into();
        self
    }
    /// Set the fallback recorder
    pub fn with_fallback(mut self, fallback: Box<dyn Recorder + Send + Sync>) -> Self {
        self.fallback = Some(fallback);
        self
    }
    /// Build the exporter's recorder
    pub fn build(self) -> ScopeRecorder {
        ScopeRecorder::build(self.addr, self.fallback)
    }
    /// Build the exporter's recorder and install it as the global recorder
    pub fn install(self) -> Result<(), Error> {
        self.build().install()
    }
}

/// Scope recorder
#[derive(Clone)]
pub struct ScopeRecorder {
    inner: Arc<Inner>,
    fallback: Arc<Option<Box<dyn Recorder + Send + Sync>>>,
}

impl ScopeRecorder {
    fn build<A: Into<SocketAddr>>(
        addr: A,
        fallback: Option<Box<dyn Recorder + Send + Sync>>,
    ) -> Self {
        Self {
            inner: Arc::new(Inner::new(addr.into())),
            fallback: fallback.into(),
        }
    }
    /// # Panics
    ///
    /// Panics if the global recorder has already been set.
    pub fn install(self) -> Result<(), Error> {
        self.spawn_tasks()?;
        metrics::set_global_recorder(self).map_err(Into::into)
    }
    fn spawn_tasks(&self) -> Result<(), std::io::Error> {
        self.inner.spawn_server(self.inner.addr)?;
        Ok(())
    }
}

struct Inner {
    registry: Registry<Key, GenerationalStorage<AtomicStorage>>,
    addr: SocketAddr,
}

impl Inner {
    fn new(addr: SocketAddr) -> Self {
        let registry = Registry::new(GenerationalStorage::new(AtomicStorage));
        Self { registry, addr }
    }
    fn snapshot(&self, t: Monotonic) -> Snapshot {
        let handles = self.registry.get_gauge_handles();
        let mut map = BTreeMap::new();
        for (key, gauge) in handles {
            let name = key.name();
            let value = f64::from_bits(gauge.get_inner().load(Ordering::Acquire));
            map.insert(name[1..].to_string(), value);
        }
        Snapshot { t, d: map }
    }
    fn info(&self) -> Info {
        let info = self
            .registry
            .get_gauge_handles()
            .iter()
            .map(|(key, _)| {
                let labels = key
                    .labels()
                    .map(|label| (label.key().to_owned(), label.value().to_owned()));
                (
                    key.name()[1..].to_string(),
                    MetricInfo {
                        labels: labels.collect(),
                    },
                )
            })
            .collect();
        Info { metrics: info }
    }
    fn spawn_server(self: &Arc<Self>, addr: SocketAddr) -> Result<(), std::io::Error> {
        let listener = TcpListener::bind(addr)?;
        let metrics_scope = self.clone();
        thread::Builder::new()
            .name(SERVER_THREAD_NAME.to_owned())
            .spawn(move || {
                while let Ok((stream, addr)) = listener.accept() {
                    info!(?addr, "client connected");
                    let metrics_scope = metrics_scope.clone();
                    thread::spawn(move || {
                        if let Err(error) = handle_client(stream, metrics_scope) {
                            error!(?addr, ?error, "client error, disconnected");
                        } else {
                            info!(?addr, "client disconnected");
                        }
                    });
                }
            })?;
        Ok(())
    }
}
fn handle_client(mut stream: TcpStream, metrics_scope: Arc<Inner>) -> Result<(), Error> {
    stream.set_read_timeout(Some(CLIENT_CHAT_TIMEOUT))?;
    stream.set_write_timeout(Some(CLIENT_CHAT_TIMEOUT))?;
    stream.set_nodelay(true)?;
    protocol::write_version(&mut stream)?;
    let clients_settings = protocol::read_client_settings(&mut stream)?;
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(None)?;
    protocol::write_packet(&mut stream, &Packet::Info(metrics_scope.info()))?;
    let mut last_info_sent = Monotonic::now();
    let int_ns = u128::from(clients_settings.sampling_interval);
    let start = Monotonic::now();
    for _ in interval(Duration::from_nanos(clients_settings.sampling_interval)) {
        let ts = Monotonic::from_nanos(
            (start.elapsed().as_nanos() / int_ns * int_ns)
                .try_into()
                .unwrap(),
        );
        let packet = Packet::Snapshot(metrics_scope.snapshot(ts));
        if protocol::write_packet(&mut stream, &packet).is_err() {
            break;
        }
        if last_info_sent.elapsed() >= SEND_INFO_INTERVAL {
            let packet = Packet::Info(metrics_scope.info());
            if protocol::write_packet(&mut stream, &packet).is_err() {
                break;
            }
            last_info_sent = Monotonic::now();
        }
    }
    Ok(())
}

impl Recorder for ScopeRecorder {
    fn describe_counter(
        &self,
        key: metrics::KeyName,
        unit: Option<metrics::Unit>,
        description: metrics::SharedString,
    ) {
        if let Some(fallback) = self.fallback.as_ref() {
            fallback.describe_counter(key, unit, description);
        }
    }

    fn describe_gauge(
        &self,
        key: metrics::KeyName,
        unit: Option<metrics::Unit>,
        description: metrics::SharedString,
    ) {
        if let Some(fallback) = self.fallback.as_ref() {
            fallback.describe_gauge(key, unit, description);
        }
    }

    fn describe_histogram(
        &self,
        key: metrics::KeyName,
        unit: Option<metrics::Unit>,
        description: metrics::SharedString,
    ) {
        if let Some(fallback) = self.fallback.as_ref() {
            fallback.describe_histogram(key, unit, description);
        }
    }

    fn register_counter(
        &self,
        key: &metrics::Key,
        metadata: &metrics::Metadata<'_>,
    ) -> metrics::Counter {
        if let Some(fallback) = self.fallback.as_ref() {
            fallback.register_counter(key, metadata)
        } else {
            metrics::Counter::noop()
        }
    }

    fn register_gauge(
        &self,
        key: &metrics::Key,
        metadata: &metrics::Metadata<'_>,
    ) -> metrics::Gauge {
        if key.name().starts_with('~') {
            self.inner
                .registry
                .get_or_create_gauge(key, |c| c.clone().into())
        } else if let Some(fallback) = self.fallback.as_ref() {
            fallback.register_gauge(key, metadata)
        } else {
            metrics::Gauge::noop()
        }
    }

    fn register_histogram(
        &self,
        key: &metrics::Key,
        metadata: &metrics::Metadata<'_>,
    ) -> metrics::Histogram {
        if let Some(fallback) = self.fallback.as_ref() {
            fallback.register_histogram(key, metadata)
        } else {
            metrics::Histogram::noop()
        }
    }
}
