use crate::plugins::telemetry::consts::OTEL_ORIGINAL_NAME;
use crate::plugins::telemetry::tracing::datadog_exporter::DatadogTraceState;
use futures::future::BoxFuture;
use opentelemetry::trace::{SpanContext, SpanKind};
use opentelemetry::{KeyValue, Value};
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::export::trace::{ExportResult, SpanData, SpanExporter};
use std::fmt::{Debug, Formatter};

pub(crate) struct MeasuringExporter<T: SpanExporter> {
    pub(crate) delegate: T,
    pub(crate) span_metrics: ahash::HashMap<String, bool>,
}

impl<T: SpanExporter> Debug for MeasuringExporter<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.delegate.fmt(f)
    }
}

impl<T: SpanExporter> SpanExporter for MeasuringExporter<T> {
    fn export(&mut self, mut batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        // Here we do some special processing of the spans before passing them to the delegate
        // In particular we default the span.kind to the span kind, and also override the trace measure status if we need to.
        for span in &mut batch {
            // If the span metrics are enabled for this span, set the trace state to measuring.
            // We do all this dancing to avoid allocating.
            let original_span_name = span
                .attributes
                .iter()
                .find(|kv| kv.key.as_str() == OTEL_ORIGINAL_NAME)
                .map(|kv| kv.value.as_str());
            let final_span_name = if let Some(span_name) = &original_span_name {
                span_name.as_ref()
            } else {
                span.name.as_ref()
            };

            // If enabled via config
            let metrics_configured = self.span_metrics.get(final_span_name).map(|v| *v);

            if metrics_configured.unwrap_or_default() {
                let new_trace_state = span.span_context.trace_state().with_measuring(true);
                span.span_context = SpanContext::new(
                    span.span_context.trace_id(),
                    span.span_context.span_id(),
                    span.span_context.trace_flags(),
                    span.span_context.is_remote(),
                    new_trace_state,
                );
                span.attributes.push(KeyValue::new("_dd.measured", "1"));
            }

            // Set the span kind https://github.com/DataDog/dd-trace-go/blob/main/ddtrace/ext/span_kind.go
            let span_kind = match &span.span_kind {
                SpanKind::Client => "client",
                SpanKind::Server => "server",
                SpanKind::Producer => "producer",
                SpanKind::Consumer => "consumer",
                SpanKind::Internal => "internal",
            };
            span.attributes.push(KeyValue::new("span.kind", span_kind));

            // Note we do NOT set span.type as it isn't a good fit for otel.
        }
        self.delegate.export(batch)
    }
    fn shutdown(&mut self) {
        self.delegate.shutdown()
    }
    fn force_flush(&mut self) -> BoxFuture<'static, ExportResult> {
        self.delegate.force_flush()
    }
    fn set_resource(&mut self, resource: &Resource) {
        self.delegate.set_resource(resource);
    }
}
