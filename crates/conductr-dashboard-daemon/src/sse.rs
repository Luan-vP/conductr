use conductr_dashboard_core::SseEvent;
use tokio::sync::broadcast;

pub const SSE_CHANNEL_CAPACITY: usize = 128;

/// Create the broadcast channel used to fan out SSE events to all connected
/// outlet clients.
pub fn new_channel() -> (broadcast::Sender<SseEvent>, broadcast::Receiver<SseEvent>) {
    broadcast::channel(SSE_CHANNEL_CAPACITY)
}

/// Format a single `SseEvent` as an SSE frame (RFC 8895 §9).
///
/// The `event:` field carries the event name; the `data:` field carries only
/// the inner payload JSON (no event-type tag in the body).
pub fn format_sse_frame(event: &SseEvent) -> String {
    let name = event.event_name();
    let data = event.to_data_json();
    format!("event: {name}\ndata: {data}\n\n")
}
