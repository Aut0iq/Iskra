//! Typed chat metadata, status aggregation and the emotion state machine.
//!
//! Semantics follow docs/ARCHITECTURE.md:
//!   awaitingInput > errored (anger 6s then sadness) > working (focus,
//!   alternating props) > completed (joy, unseen results only) > idle.
//! Stale reports are treated as unknown; unknown-only data is subdued.
//! Disconnection is its own state and never counts as completion or idle.

use serde_json::Value;
use std::collections::HashSet;
use std::time::{Duration, Instant};

/// Anger shown for this long after a newly observed error episode.
pub const ANGER_DURATION: Duration = Duration::from_millis(6_000);
/// focus_laptop / focus_solder alternate on this interval while working.
pub const FOCUS_ALTERNATE: Duration = Duration::from_millis(12_000);
/// A snapshot older than this is treated as a lost connection.
pub const FRESHNESS_LIMIT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeStatus {
    Working,
    Completed,
    Errored,
    AwaitingInput,
    Idle,
    Unknown,
}

impl NativeStatus {
    pub fn parse(raw: Option<&str>) -> Self {
        match raw {
            Some("working") => Self::Working,
            Some("completed") => Self::Completed,
            Some("errored") => Self::Errored,
            Some("awaitingInput") => Self::AwaitingInput,
            Some("idle") => Self::Idle,
            _ => Self::Unknown,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::Completed => "completed",
            Self::Errored => "errored",
            Self::AwaitingInput => "awaiting input",
            Self::Idle => "idle",
            Self::Unknown => "unknown",
        }
    }
}

/// Metadata kept per chat. Message previews and chat content are never kept.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatMeta {
    pub id: String,
    pub title: String,
    pub status: NativeStatus,
    pub stale: bool,
    pub last_message_at: Option<String>,
    pub parent_chat_id: Option<String>,
    pub device_id: Option<String>,
    pub last_turn_outcome: Option<String>,
}

impl ChatMeta {
    /// Effective status used for aggregation: a stale live report is unknown.
    pub fn effective_status(&self) -> NativeStatus {
        if self.stale {
            NativeStatus::Unknown
        } else {
            self.status
        }
    }

    pub fn from_json(v: &Value) -> Option<Self> {
        let id = v.get("id").and_then(Value::as_str)?.to_string();
        Some(ChatMeta {
            id,
            title: v
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            status: NativeStatus::parse(v.get("status").and_then(Value::as_str)),
            stale: v
                .get("statusStale")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            last_message_at: v
                .get("lastMessageAt")
                .and_then(Value::as_str)
                .map(str::to_string),
            parent_chat_id: v
                .get("parentChatId")
                .and_then(Value::as_str)
                .map(str::to_string),
            device_id: v
                .get("deviceId")
                .and_then(Value::as_str)
                .map(str::to_string),
            last_turn_outcome: v
                .get("lastTurnOutcome")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub chats: Vec<ChatMeta>,
    /// Total chats reported by the engine; may exceed `chats.len()` when the
    /// list was truncated by the request limit.
    pub total: usize,
    pub received: Instant,
}

impl Snapshot {
    pub fn truncated(&self) -> bool {
        self.total > self.chats.len()
    }

    pub fn fresh(&self, now: Instant) -> bool {
        now.duration_since(self.received) <= FRESHNESS_LIMIT
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Counts {
    pub working: usize,
    pub completed: usize,
    pub errored: usize,
    pub awaiting_input: usize,
    pub idle: usize,
    pub unknown: usize,
    pub shown: usize,
    pub total: usize,
    pub truncated: bool,
}

impl Counts {
    fn bump(&mut self, s: NativeStatus) {
        match s {
            NativeStatus::Working => self.working += 1,
            NativeStatus::Completed => self.completed += 1,
            NativeStatus::Errored => self.errored += 1,
            NativeStatus::AwaitingInput => self.awaiting_input += 1,
            NativeStatus::Idle => self.idle += 1,
            NativeStatus::Unknown => self.unknown += 1,
        }
    }

    /// Compact readable badge text, e.g. "2 working · 1 awaiting input".
    pub fn summary(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        let mut add = |n: usize, label: &str| {
            if n > 0 {
                parts.push(format!("{n} {label}"));
            }
        };
        add(self.awaiting_input, "awaiting input");
        add(self.errored, "errored");
        add(self.working, "working");
        add(self.completed, "completed");
        add(self.unknown, "unknown");
        if parts.is_empty() {
            parts.push(format!("{} idle", self.idle));
        }
        let mut s = parts.join(" · ");
        if self.truncated {
            s.push_str(&format!(" (+{}/{})", self.shown, self.total));
        }
        s
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emotion {
    Idle,
    Joy,
    Focus,
    Anger,
    Sadness,
    Question,
    Subdued,
    Disconnected,
}

impl Emotion {
    /// Roaming is stopped for states that must stay legible.
    pub fn stationary(self) -> bool {
        !matches!(self, Emotion::Idle | Emotion::Focus)
    }

    /// The eight-animation asset name; Focus alternates on FOCUS_ALTERNATE.
    pub fn animation(self, focus_phase: bool) -> &'static str {
        match self {
            Emotion::Idle => "idle",
            Emotion::Joy => "joy",
            Emotion::Focus => {
                if focus_phase {
                    "focus_solder"
                } else {
                    "focus_laptop"
                }
            }
            Emotion::Anger => "anger",
            Emotion::Sadness | Emotion::Subdued | Emotion::Disconnected => "sadness",
            Emotion::Question => "question",
        }
    }
}

/// Semantic status independent of the visual emotion/animation. Derived
/// only from truthful engine state; demos, poses and reactions cannot
/// overwrite it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StatusKind {
    Working,
    AwaitingInput,
    Success,
    Error,
    Idle,
    Unknown,
    Disconnected,
}

impl StatusKind {
    /// Single-token machine name for --status / bar.
    pub fn token(self) -> &'static str {
        match self {
            StatusKind::Working => "working",
            StatusKind::AwaitingInput => "awaiting_input",
            StatusKind::Success => "success",
            StatusKind::Error => "error",
            StatusKind::Idle => "idle",
            StatusKind::Unknown => "unknown",
            StatusKind::Disconnected => "disconnected",
        }
    }

    /// Primary persistent badge label (Russian, full word).
    pub fn label(self) -> &'static str {
        match self {
            StatusKind::Working => "Работает",
            StatusKind::AwaitingInput => "Нужен ответ",
            StatusKind::Success => "Готово",
            StatusKind::Error => "Ошибка",
            StatusKind::Idle => "Отдыхает",
            StatusKind::Unknown => "Статус неизвестен",
            StatusKind::Disconnected => "Нет связи",
        }
    }

    /// Compact secondary detail suffix counts (Russian, only nonzero).
    pub fn detail(&self, c: &Counts) -> String {
        let mut parts: Vec<String> = Vec::new();
        if c.working > 0 {
            parts.push(format!("{} работают", c.working));
        }
        if c.awaiting_input > 0 {
            parts.push(format!("{} ждут ответа", c.awaiting_input));
        }
        if c.errored > 0 {
            parts.push(format!("{} ошибки", c.errored));
        }
        if c.completed > 0 {
            parts.push(format!("{} готовы", c.completed));
        }
        if c.idle > 0 && parts.is_empty() {
            parts.push(format!("{} отдыхают", c.idle));
        }
        parts.join(" · ")
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayState {
    pub emotion: Emotion,
    pub badge: String,
    pub connected: bool,
    pub counts: Counts,
    /// Semantic status kind — never affected by demo/pose/reaction.
    pub kind: StatusKind,
    /// Title of the explicitly selected chat, if narrowing is active.
    pub selected_title: Option<String>,
    /// True on the update where a NEW awaiting-input or errored chat
    /// appeared after the baseline — used to interrupt a blush reaction.
    pub attention_episode: bool,
    /// True on the update where a NEW chat entered `completed` after the
    /// baseline — starts a fresh 6s joy budget. Never true for startup,
    /// reconnect or scope-rebaseline history.
    pub completion_episode: bool,
}

#[derive(Debug, Default)]
pub struct Engine {
    baselined: bool,
    prev_errored: HashSet<String>,
    prev_awaiting: HashSet<String>,
    prev_completed: HashSet<String>,
    anger_until: Option<Instant>,
    /// Chat ids whose completed status is an unseen result observed after
    /// the baseline; membership is removed when the chat leaves `completed`.
    celebratable: HashSet<String>,
    /// Aggregation scope the episode tracking was baselined against.
    /// A change (all <-> selected chat) rebaselines so old states in the
    /// new scope cannot fabricate error/completion events.
    scope: Option<String>,
    scope_set: bool,
}

impl Engine {
    pub fn new() -> Self {
        Self::default()
    }

    fn reset_episodes(&mut self) {
        self.baselined = false;
        self.prev_errored.clear();
        self.prev_awaiting.clear();
        self.prev_completed.clear();
        self.celebratable.clear();
        self.anger_until = None;
    }

    /// Called when the transport is not delivering a fresh snapshot.
    /// Re-baselines so a reconnect does not celebrate pre-existing states.
    pub fn on_disconnect(&mut self) -> DisplayState {
        self.reset_episodes();
        DisplayState {
            emotion: Emotion::Disconnected,
            badge: "disconnected".to_string(),
            connected: false,
            counts: Counts::default(),
            kind: StatusKind::Disconnected,
            selected_title: None,
            attention_episode: false,
            completion_episode: false,
        }
    }

    /// Single entry point honoring transport truth: a Down transport or a
    /// missing/expired snapshot is an immediate disconnect, never a
    /// lingering successful state.
    pub fn observe(
        &mut self,
        snap: Option<&Snapshot>,
        transport_connected: bool,
        selected: Option<&str>,
        now: Instant,
    ) -> DisplayState {
        match snap {
            Some(s) if transport_connected && s.fresh(now) => self.update(s, selected, now),
            _ => self.on_disconnect(),
        }
    }

    /// Chats the aggregation covers: everything, or the selected chat plus
    /// its direct children when a selection narrows the view.
    fn filtered<'a>(
        snap: &'a Snapshot,
        selected: Option<&str>,
    ) -> (Vec<&'a ChatMeta>, Option<String>) {
        match selected {
            None => (snap.chats.iter().collect(), None),
            Some(id) => {
                let title = snap
                    .chats
                    .iter()
                    .find(|c| c.id == id)
                    .map(|c| c.title.clone());
                let v: Vec<&ChatMeta> = snap
                    .chats
                    .iter()
                    .filter(|c| c.id == id || c.parent_chat_id.as_deref() == Some(id))
                    .collect();
                (v, title)
            }
        }
    }

    pub fn update(
        &mut self,
        snap: &Snapshot,
        selected: Option<&str>,
        now: Instant,
    ) -> DisplayState {
        // Scope change: re-baseline episodes so pre-existing states in the
        // new scope are established state, not fresh events.
        let scope = selected.map(str::to_string);
        if self.scope_set && self.scope != scope {
            self.reset_episodes();
        }
        self.scope = scope;
        self.scope_set = true;

        let (chats, selected_title) = Self::filtered(snap, selected);

        let mut counts = Counts {
            shown: chats.len(),
            total: snap.total,
            truncated: snap.truncated(),
            ..Counts::default()
        };
        let mut errored_now = HashSet::new();
        let mut awaiting_now = HashSet::new();
        let mut completed_now = HashSet::new();
        for c in &chats {
            let st = c.effective_status();
            counts.bump(st);
            if st == NativeStatus::Errored {
                errored_now.insert(c.id.clone());
            } else if st == NativeStatus::AwaitingInput {
                awaiting_now.insert(c.id.clone());
            } else if st == NativeStatus::Completed {
                completed_now.insert(c.id.clone());
            }
        }

        let new_error = errored_now.iter().any(|id| !self.prev_errored.contains(id));
        let new_awaiting = awaiting_now
            .iter()
            .any(|id| !self.prev_awaiting.contains(id));
        // A completion entering `completed` after the baseline is a fresh
        // episode; startup/reconnect/scope history is baselined instead.
        let completion_episode = self.baselined
            && completed_now
                .iter()
                .any(|id| !self.prev_completed.contains(id));
        let mut attention_episode = false;
        if self.baselined {
            // New error episodes retrigger the anger interval.
            if new_error {
                self.anger_until = Some(now + ANGER_DURATION);
            }
            // New awaiting-input/error episodes interrupt a blush reaction.
            attention_episode = new_error || new_awaiting;
            // Completions entering `completed` after the baseline are unseen
            // results worth celebrating while they remain unopened.
            for id in completed_now.difference(&self.prev_completed) {
                self.celebratable.insert(id.clone());
            }
        } else {
            // Startup/reconnect baseline: existing errors and completed
            // history are established state, never celebrations.
            self.baselined = true;
        }
        self.prev_errored = errored_now;
        self.prev_awaiting = awaiting_now;
        self.celebratable.retain(|id| completed_now.contains(id));
        self.prev_completed = completed_now;

        let has_errors = !self.prev_errored.is_empty();
        let celebrating = !self.celebratable.is_empty();
        let all_unknown = !chats.is_empty() && counts.unknown == chats.len();

        // Semantic kind: same precedence as the visual emotion but purely
        // status-derived — completed history at startup is Success without
        // a fresh joy episode; unknown/empty scope is Unknown, not Idle.
        let kind = if counts.awaiting_input > 0 {
            StatusKind::AwaitingInput
        } else if counts.errored > 0 {
            StatusKind::Error
        } else if counts.working > 0 {
            StatusKind::Working
        } else if counts.completed > 0 {
            StatusKind::Success
        } else if chats.is_empty() || all_unknown {
            StatusKind::Unknown
        } else {
            StatusKind::Idle
        };

        let (emotion, note) = if counts.awaiting_input > 0 {
            (Emotion::Question, None)
        } else if has_errors {
            let angry = self.anger_until.map(|t| now < t).unwrap_or(false);
            (
                if angry {
                    Emotion::Anger
                } else {
                    Emotion::Sadness
                },
                None,
            )
        } else if counts.working > 0 {
            (Emotion::Focus, None)
        } else if celebrating {
            (Emotion::Joy, None)
        } else if chats.is_empty() {
            (
                Emotion::Subdued,
                Some(if selected.is_some() {
                    "chat not found".to_string()
                } else {
                    "no chats".to_string()
                }),
            )
        } else if all_unknown {
            (Emotion::Subdued, Some("unknown status".to_string()))
        } else {
            (Emotion::Idle, None)
        };

        let mut badge = match emotion {
            Emotion::Joy => format!("done · {}", counts.summary()),
            _ => counts.summary(),
        };
        if let Some(n) = note {
            badge = format!("{n} · {badge}");
        }
        if let Some(t) = &selected_title {
            badge = format!("{t}: {badge}");
        }

        DisplayState {
            emotion,
            badge,
            connected: true,
            counts,
            kind,
            selected_title,
            attention_episode,
            completion_episode,
        }
    }
}
