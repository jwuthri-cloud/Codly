//! Settings → Notifications: which events raise a notification of your system. The app decides
//! what each event says and shows it (src-tauri/src/notify.rs); this is only what you chose.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One switch per kind of notification, and one for when they show. All on by default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(default)]
#[ts(export)]
pub struct NotificationSettings {
    /// An agent waits for your approval, or a plan waits for your Run.
    pub approval: bool,
    /// An agent asks you a question.
    pub question: bool,
    /// A plan finished.
    pub plan_done: bool,
    /// A plan failed, or cannot go on by itself.
    pub plan_failed: bool,
    /// A task is blocked, or failed.
    pub task_blocked: bool,
    /// An agent gives no output or sits idle on its task, or Codly held it back or stopped it.
    pub agent_stalled: bool,
    /// A budget is used up.
    pub budget: bool,
    /// A solo agent ended a turn with changes: its branch is ready to merge.
    pub merge_ready: bool,
    /// Only while Codly's window is not in front.
    pub only_in_background: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        NotificationSettings {
            approval: true,
            question: true,
            plan_done: true,
            plan_failed: true,
            task_blocked: true,
            agent_stalled: true,
            budget: true,
            merge_ready: true,
            only_in_background: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::Settings;

    /// A settings file written before these switches existed still loads, with everything on.
    #[test]
    fn settings_saved_before_the_switches_load_with_the_defaults() {
        let mut saved = serde_json::to_value(Settings::default()).unwrap();
        saved.as_object_mut().unwrap().remove("notifications");
        let loaded: Settings = serde_json::from_value(saved).unwrap();
        assert_eq!(loaded.notifications, NotificationSettings::default());
        assert!(loaded.notifications.approval && loaded.notifications.only_in_background);
        // A part of it only: the rest keeps its default.
        let partial: NotificationSettings = serde_json::from_value(json!({ "plan_done": false })).unwrap();
        assert_eq!(partial, NotificationSettings { plan_done: false, ..Default::default() });
    }

    #[test]
    fn a_switch_you_turned_off_stays_off_when_saved_and_read_again() {
        let settings = Settings { notifications: NotificationSettings { budget: false, only_in_background: false, ..Default::default() }, ..Default::default() };
        let read: Settings = serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
        assert_eq!(read.notifications, settings.notifications);
    }
}
