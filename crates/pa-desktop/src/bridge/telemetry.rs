//! Adoption telemetry for explicit project list changes.

use std::path::Path;

use pa_core::session_engine::telemetry::{TelemetrySwitch, build_client, telemetry_switch};
use pa_core::settings::SettingsManager;
use pa_telemetry::{DesktopProjectAction, TelemetryClient};
use tokio::runtime::Handle;

use super::BridgeHandle;

impl BridgeHandle {
    /// Project paths are used only to honor local settings, never as event properties.
    pub(crate) fn track_project_action(
        &self,
        agent_dir: &Path,
        project: &Path,
        action: DesktopProjectAction,
    ) {
        let agent_dir = agent_dir.to_path_buf();
        let project = project.to_path_buf();
        enqueue_project_action(self.runtime.handle(), action, move || {
            let settings = SettingsManager::create(project, &agent_dir);
            let global = settings
                .global_settings()
                .telemetry
                .as_ref()
                .and_then(|telemetry| telemetry.enabled);
            // A project can opt out but cannot opt the user in. Requiring a settings
            // decision also prevents an environment override from bypassing a later opt-out.
            (global == Some(true) && telemetry_switch(&settings) == TelemetrySwitch::Settings(true))
                .then(|| build_client(&settings, &agent_dir))
        });
    }
}

fn enqueue_project_action(
    runtime: &Handle,
    action: DesktopProjectAction,
    client: impl FnOnce() -> Option<TelemetryClient> + Send + 'static,
) {
    drop(runtime.spawn(async move {
        if let Some(client) = client() {
            // The telemetry worker drains the one-shot client on drop.
            action.track(&client);
        }
    }));
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use futures::future::BoxFuture;
    use pa_telemetry::{SinkOutcome, TelemetryClientConfig, TelemetryEvent, TelemetrySink};
    use tokio::sync::mpsc;

    use super::*;

    struct HangingSink(mpsc::UnboundedSender<Vec<TelemetryEvent>>);

    impl TelemetrySink for HangingSink {
        fn send_batch<'a>(
            &'a self,
            _install_id: &'a str,
            events: Vec<TelemetryEvent>,
        ) -> BoxFuture<'a, SinkOutcome> {
            Box::pin(async move {
                self.0.send(events).unwrap();
                std::future::pending().await
            })
        }
    }

    #[tokio::test]
    async fn project_navigation_does_not_wait_for_delivery() {
        let (entered, mut batches) = mpsc::unbounded_channel();
        let handoff = tokio::spawn(async move {
            enqueue_project_action(&Handle::current(), DesktopProjectAction::Added, || {
                let mut config = TelemetryClientConfig::new("test");
                config.sinks = vec![Arc::new(HangingSink(entered))];
                Some(TelemetryClient::spawn(config).unwrap())
            });
        });
        let (handoff, batch) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(handoff, batches.recv())
        })
        .await
        .expect("the handoff must return while delivery hangs");
        handoff.unwrap();
        let [event]: [TelemetryEvent; 1] = batch.unwrap().try_into().unwrap();
        let mut expected = pa_telemetry::base_properties("interactive");
        expected.set("action", serde_json::json!("added"));
        assert_eq!(
            (event.name.as_str(), event.properties),
            ("desktop project action", expected)
        );
    }
}
