//! MCP progress reporting: turns `aether_render` encode progress into `notifications/progress`
//! for tool calls whose request carries `_meta.progressToken`.
use crate::encode::{ProgressObserver, ProgressUpdate};
use serde_json::{json, Value};
use std::io::Write;

/// Sends `notifications/progress` for a tool call whose request carried `_meta.progressToken`
/// (a string or an integer), written to stdout between the request and its response.
pub(super) struct McpProgress {
    token: Value,
}

impl McpProgress {
    pub(super) fn from_params(params: &Value) -> Option<Self> {
        let token = params.get("_meta")?.get("progressToken")?;
        (token.is_string() || token.is_i64() || token.is_u64()).then(|| Self {
            token: token.clone(),
        })
    }

    fn notification(&self, update: &ProgressUpdate) -> Value {
        json!({
            "jsonrpc": "2.0",
            "method": "notifications/progress",
            "params": {
                "progressToken": self.token,
                "progress": update.done,
                "total": update.total,
                "message": update.summary().trim_start(),
            }
        })
    }
}

impl ProgressObserver for McpProgress {
    fn update(&self, update: ProgressUpdate) {
        let mut stdout = std::io::stdout().lock();
        let _ = writeln!(stdout, "{}", self.notification(&update));
        let _ = stdout.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_tokens_become_progress_notifications() {
        assert!(McpProgress::from_params(&json!({"name": "aether_render"})).is_none());
        assert!(McpProgress::from_params(&json!({"_meta": {"progressToken": 1.5}})).is_none());
        let reporter =
            McpProgress::from_params(&json!({"_meta": {"progressToken": "r1"}})).unwrap();
        let note = reporter.notification(&ProgressUpdate {
            done: 30,
            total: 60,
            elapsed: std::time::Duration::from_secs(1),
        });
        assert_eq!(note["method"], "notifications/progress");
        assert_eq!(note["params"]["progressToken"], "r1");
        assert_eq!(note["params"]["progress"], 30);
        assert_eq!(note["params"]["total"], 60);
        assert_eq!(
            note["params"]["message"],
            "50% 30/60 frames, 30.0 fps, ETA 1.0 s"
        );
    }
}
