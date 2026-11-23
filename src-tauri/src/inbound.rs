//! Tauri commands for inbound work sources (webhooks, Slack). The hub is created once the core
//! is up; its calls block briefly on the Keychain, so they run on a blocking thread.

use std::sync::Arc;

use codly_core::Core;
use codly_core::inbound::secrets::default_store;
use codly_core::inbound::{Deliver, InboundHub, InboundState, NewWebhook, SlackUpdate};
use tauri::State;
use tokio::sync::watch;

pub struct InboundHandle {
    pub hub: watch::Receiver<Option<Arc<InboundHub>>>,
}

/// Start the hub: whatever was left switched on listens again. `Core::send_inbound` is how an
/// inbound message reaches the Lead (or the agent the user chose): under its source's name,
/// never as the user's own message.
pub async fn start(core: Arc<Core>, data_dir: std::path::PathBuf) -> Option<Arc<InboundHub>> {
    let deliver: Deliver = Arc::new(move |thread, msg| {
        let core = core.clone();
        Box::pin(async move { core.send_inbound(&thread, &msg).await })
    });
    tokio::task::spawn_blocking(move || InboundHub::new(&data_dir, Arc::from(default_store()), deliver)).await.ok()
}

impl InboundHandle {
    async fn run<T: Send + 'static>(&self, f: impl FnOnce(&InboundHub) -> anyhow::Result<T> + Send + 'static) -> Result<T, String> {
        let mut rx = self.hub.clone();
        let hub = rx.wait_for(|h| h.is_some()).await.map_err(|e| e.to_string())?.clone().expect("checked is_some");
        tokio::task::spawn_blocking(move || f(&hub)).await.map_err(|e| e.to_string())?.map_err(|e| format!("{e:#}"))
    }
}

#[tauri::command]
pub async fn inbound_state(h: State<'_, InboundHandle>) -> Result<InboundState, String> {
    h.run(|hub| Ok(hub.state())).await
}

/// The new endpoint is switched off; its secret is in the result and nowhere else, ever again.
#[tauri::command]
pub async fn inbound_create_webhook(h: State<'_, InboundHandle>, name: String) -> Result<NewWebhook, String> {
    h.run(move |hub| hub.create_webhook(&name)).await
}

/// The two switches of one endpoint: on or off, and whether a plan made from its messages may
/// run without the user's Run plan click (`auto_run`).
#[tauri::command]
pub async fn inbound_set_webhook(h: State<'_, InboundHandle>, id: String, enabled: bool, auto_run: bool) -> Result<InboundState, String> {
    h.run(move |hub| {
        // First, so an endpoint that is switched on here already has the choice made for it.
        hub.set_webhook_auto_run(&id, auto_run)?;
        hub.set_webhook_enabled(&id, enabled).map(|()| hub.state())
    })
    .await
}

#[tauri::command]
pub async fn inbound_revoke_webhook(h: State<'_, InboundHandle>, id: String) -> Result<InboundState, String> {
    h.run(move |hub| hub.revoke_webhook(&id).map(|()| hub.state())).await
}

#[tauri::command]
pub async fn inbound_set_settings(h: State<'_, InboundHandle>, answerer: String, port: u16) -> Result<InboundState, String> {
    h.run(move |hub| hub.set_settings(&answerer, port).map(|()| hub.state())).await
}

#[tauri::command]
pub async fn inbound_set_slack(h: State<'_, InboundHandle>, update: SlackUpdate) -> Result<InboundState, String> {
    h.run(move |hub| hub.set_slack(update).map(|()| hub.state())).await
}
