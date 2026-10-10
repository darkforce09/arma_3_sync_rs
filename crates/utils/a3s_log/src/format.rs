use std::fmt::{self, Write as _};

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

/// Java-like line format, used for stderr and the debug file:
/// `YYYY-MM-DD HH:MM:SS.mmm [<thread name>] [<LEVEL>] <message> key=value…`.
///
/// Local time; unnamed threads print `unnamed`; no ANSI colours.
// Mirrors utils/DebugLogger.java:140-153
#[derive(Clone, Copy, Debug, Default)]
pub struct JavaLikeFormat;

impl<S, N> FormatEvent<S, N> for JavaLikeFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let now = chrono::Local::now();
        let thread = std::thread::current();
        let mut fields = FieldCollector::default();
        event.record(&mut fields);
        writeln!(
            writer,
            "{} [{}] [{}] {}{}",
            now.format("%Y-%m-%d %H:%M:%S%.3f"),
            thread.name().unwrap_or("unnamed"),
            event.metadata().level(),
            fields.message,
            fields.extra
        )
    }
}

/// Collects the `message` field and ` key=value` pairs of the other fields.
#[derive(Default)]
struct FieldCollector {
    message: String,
    extra: String,
}

impl Visit for FieldCollector {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            write!(self.extra, " {}={value}", field.name()).ok();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            write!(self.message, "{value:?}").ok();
        } else {
            write!(self.extra, " {}={value:?}", field.name()).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::Arc;

    use parking_lot::Mutex;
    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Buffer {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for Buffer {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    fn capture(thread_name: Option<&str>, emit: fn()) -> String {
        let buffer = Buffer::default();
        let subscriber = tracing_subscriber::fmt()
            .event_format(JavaLikeFormat)
            .with_writer(buffer.clone())
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let mut builder = std::thread::Builder::new();
        if let Some(name) = thread_name {
            builder = builder.name(name.to_owned());
        }
        builder
            .spawn(move || tracing::subscriber::with_default(subscriber, emit))
            .expect("spawn")
            .join()
            .expect("join");
        String::from_utf8(buffer.0.lock().clone()).expect("utf-8")
    }

    /// Checks `YYYY-MM-DD HH:MM:SS.mmm ` and returns the rest of the line.
    fn strip_timestamp(line: &str) -> &str {
        let (stamp, rest) = line.split_at(24);
        let pattern = "dddd-dd-dd dd:dd:dd.ddd ";
        for (c, p) in stamp.chars().zip(pattern.chars()) {
            if p == 'd' {
                assert!(c.is_ascii_digit(), "{line:?}");
            } else {
                assert_eq!(c, p, "{line:?}");
            }
        }
        rest
    }

    #[test]
    fn a_l4_line_format() {
        let out = capture(Some("worker"), || tracing::info!(n = 1, "hello"));
        assert!(out.ends_with('\n'));
        assert_eq!(out.matches('\n').count(), 1, "{out:?}");
        assert_eq!(strip_timestamp(&out), "[worker] [INFO] hello n=1\n");
    }

    #[test]
    fn a_l4_unnamed_thread_and_levels() {
        let out = capture(None, || {
            tracing::warn!(path = "a b", "careful");
            tracing::error!("broken");
        });
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2, "{out:?}");
        assert_eq!(
            strip_timestamp(lines[0]),
            "[unnamed] [WARN] careful path=a b"
        );
        assert_eq!(strip_timestamp(lines[1]), "[unnamed] [ERROR] broken");
    }
}
