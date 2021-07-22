use core::hash::Hash;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::{collections::HashMap, fmt::Debug};

use crate::{kind::MetricKind, registry::Registry, CompositeKey};

use indexmap::IndexMap;
use metrics::{Counter, Gauge, Histogram, Key, Recorder, Unit};
use ordered_float::OrderedFloat;

pub struct Snapshot(HashMap<CompositeKey, (Option<Unit>, Option<&'static str>, DebugValue)>);

impl Snapshot {
    pub fn into_hashmap(
        self,
    ) -> HashMap<CompositeKey, (Option<Unit>, Option<&'static str>, DebugValue)> {
        self.0
    }

    pub fn into_vec(self) -> Vec<(CompositeKey, Option<Unit>, Option<&'static str>, DebugValue)> {
        self.0
            .into_iter()
            .map(|(k, (unit, desc, value))| (k, unit, desc, value))
            .collect::<Vec<_>>()
    }
}

/// A point-in-time value for a metric exposing raw values.
#[derive(Debug, PartialEq, Eq, Hash)]
pub enum DebugValue {
    /// Counter.
    Counter(u64),
    /// Gauge.
    Gauge(OrderedFloat<f64>),
    /// Histogram.
    Histogram(Vec<OrderedFloat<f64>>),
}

/// Captures point-in-time snapshots of `DebuggingRecorder`.
pub struct Snapshotter {
    registry: Arc<Registry>,
    metrics: Arc<Mutex<IndexMap<CompositeKey, (Option<Unit>, Option<&'static str>)>>>,
}

impl Snapshotter {
    /// Takes a snapshot of the recorder.
    pub fn snapshot(&self) -> Snapshot {
        let mut snapshot = HashMap::new();

        let counters = self.registry.get_counter_handles();
        let gauges = self.registry.get_gauge_handles();
        let histograms = self.registry.get_histogram_handles();

        let metrics = self.metrics.lock().expect("metrics lock poisoned").clone();

        for (ck, (unit, desc)) in metrics.into_iter() {
            let value = match ck.kind() {
                MetricKind::Counter => counters
                    .get(ck.key())
                    .map(|c| DebugValue::Counter(c.load(Ordering::SeqCst))),
                MetricKind::Gauge => gauges.get(ck.key()).map(|g| {
                    let value = f64::from_bits(g.load(Ordering::SeqCst));
                    DebugValue::Gauge(value.into())
                }),
                MetricKind::Histogram => histograms.get(ck.key()).map(|h| {
                    let mut values = Vec::new();
                    h.clear_with(|xs| values.extend(xs.iter().map(|f| OrderedFloat::from(*f))));
                    DebugValue::Histogram(values)
                }),
            };
            let value = value.expect("debug value should always be present");

            snapshot.insert(ck, (unit, desc, value));
        }

        Snapshot(snapshot)
    }
}

/// A simplistic recorder that can be installed and used for debugging or testing.
///
/// Callers can easily take snapshots of the metrics at any given time and get access
/// to the raw values.
pub struct DebuggingRecorder {
    registry: Arc<Registry>,
    metrics: Arc<Mutex<IndexMap<CompositeKey, (Option<Unit>, Option<&'static str>)>>>,
}

impl DebuggingRecorder {
    /// Creates a new `DebuggingRecorder`.
    pub fn new() -> DebuggingRecorder {
        DebuggingRecorder {
            registry: Arc::new(Registry::new()),
            metrics: Arc::new(Mutex::new(IndexMap::new())),
        }
    }

    /// Gets a `Snapshotter` attached to this recorder.
    pub fn snapshotter(&self) -> Snapshotter {
        Snapshotter {
            registry: self.registry.clone(),
            metrics: self.metrics.clone(),
        }
    }

    fn register_metric(&self, rkey: CompositeKey, unit: Option<Unit>, desc: Option<&'static str>) {
        let mut metrics = self.metrics.lock().expect("metrics lock poisoned");
        let (uentry, dentry) = metrics.entry(rkey).or_insert((None, None));
        *uentry = unit;
        *dentry = desc;
    }

    /// Installs this recorder as the global recorder.
    pub fn install(self) -> Result<(), metrics::SetRecorderError> {
        metrics::set_boxed_recorder(Box::new(self))
    }
}

impl Recorder for DebuggingRecorder {
    fn describe_counter(&self, key: &Key, unit: Option<Unit>, description: Option<&'static str>) {
        let ckey = CompositeKey::new(MetricKind::Counter, key.clone());
        self.register_metric(ckey, unit, description);
    }

    fn describe_gauge(&self, key: &Key, unit: Option<Unit>, description: Option<&'static str>) {
        let ckey = CompositeKey::new(MetricKind::Gauge, key.clone());
        self.register_metric(ckey, unit, description);
    }

    fn describe_histogram(&self, key: &Key, unit: Option<Unit>, description: Option<&'static str>) {
        let ckey = CompositeKey::new(MetricKind::Histogram, key.clone());
        self.register_metric(ckey, unit, description);
    }

    fn register_counter(&self, key: &Key) -> Counter {
        self.registry
            .get_or_create_counter(key, |c| Counter::from_arc(c.clone()))
    }

    fn register_gauge(&self, key: &Key) -> Gauge {
        self.registry
            .get_or_create_gauge(key, |g| Gauge::from_arc(g.clone()))
    }

    fn register_histogram(&self, key: &Key) -> Histogram {
        self.registry
            .get_or_create_histogram(key, |h| Histogram::from_arc(h.clone()))
    }
}

impl Default for DebuggingRecorder {
    fn default() -> Self {
        DebuggingRecorder::new()
    }
}
