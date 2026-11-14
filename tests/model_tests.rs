use std::time::{Duration, Instant};
use zeron_pet::model::*;

fn chat(id: &str, status: &str) -> ChatMeta {
    ChatMeta {
        id: id.into(),
        title: format!("chat-{id}"),
        status: NativeStatus::parse(Some(status)),
        stale: false,
        last_message_at: None,
        parent_chat_id: None,
        device_id: Some("dev".into()),
        last_turn_outcome: None,
    }
}

fn snap(chats: Vec<ChatMeta>, now: Instant) -> Snapshot {
    let total = chats.len();
    Snapshot {
        chats,
        total,
        received: now,
    }
}

#[test]
fn single_statuses_map_to_emotions() {
    let now = Instant::now();
    let cases = [
        ("working", Emotion::Focus),
        ("completed", Emotion::Idle), // baseline history: no celebration
        ("errored", Emotion::Sadness),
        ("awaitingInput", Emotion::Question),
        ("idle", Emotion::Idle),
        ("bogus", Emotion::Subdued),
    ];
    for (status, want) in cases {
        let mut e = Engine::new();
        let d = e.update(&snap(vec![chat("a", status)], now), None, now);
        assert_eq!(d.emotion, want, "status {status}");
    }
}

#[test]
fn priority_order_is_exact() {
    let now = Instant::now();
    let mut e = Engine::new();
    // Baseline without triggers.
    e.update(&snap(vec![chat("a", "idle")], now), None, now);
    // working beats completed.
    let d = e.update(
        &snap(vec![chat("a", "working"), chat("b", "completed")], now),
        None,
        now,
    );
    assert_eq!(d.emotion, Emotion::Focus);
    // awaitingInput beats everything.
    let d = e.update(
        &snap(
            vec![
                chat("a", "awaitingInput"),
                chat("b", "working"),
                chat("c", "errored"),
            ],
            now,
        ),
        None,
        now,
    );
    assert_eq!(d.emotion, Emotion::Question);
    // errored beats working and completed.
    let d = e.update(
        &snap(vec![chat("a", "errored"), chat("b", "working")], now),
        None,
        now,
    );
    assert_eq!(d.emotion, Emotion::Anger); // new episode: anger window
}

#[test]
fn anger_decays_to_sadness_and_clears() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    e.update(&snap(vec![chat("a", "idle")], t0), None, t0);
    let d = e.update(&snap(vec![chat("a", "errored")], t0), None, t0);
    assert_eq!(d.emotion, Emotion::Anger);
    let t1 = t0 + Duration::from_millis(6_500);
    let d = e.update(&snap(vec![chat("a", "errored")], t1), None, t1);
    assert_eq!(d.emotion, Emotion::Sadness);
    let t2 = t1 + Duration::from_secs(1);
    let d = e.update(&snap(vec![chat("a", "idle")], t2), None, t2);
    assert_eq!(d.emotion, Emotion::Idle);
    // A fresh episode after clearing retriggers anger.
    let t3 = t2 + Duration::from_secs(1);
    let d = e.update(&snap(vec![chat("a", "errored")], t3), None, t3);
    assert_eq!(d.emotion, Emotion::Anger);
    // A second, different chat erroring also retriggers anger.
    let t4 = t3 + Duration::from_millis(6_500);
    let d = e.update(
        &snap(vec![chat("a", "errored"), chat("b", "errored")], t4),
        None,
        t4,
    );
    assert_eq!(d.emotion, Emotion::Anger);
}

#[test]
fn unseen_completion_celebrates_while_unopened() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    e.update(&snap(vec![chat("a", "working")], t0), None, t0);
    let t1 = t0 + Duration::from_secs(1);
    let d = e.update(&snap(vec![chat("a", "completed")], t1), None, t1);
    assert_eq!(d.emotion, Emotion::Joy);
    // Still completed (user has not opened it): still an unseen result.
    let t2 = t1 + Duration::from_secs(30);
    let d = e.update(&snap(vec![chat("a", "completed")], t2), None, t2);
    assert_eq!(d.emotion, Emotion::Joy);
    // Status moves on (opened/dismissed): celebration ends.
    let t3 = t2 + Duration::from_secs(1);
    let d = e.update(&snap(vec![chat("a", "idle")], t3), None, t3);
    assert_eq!(d.emotion, Emotion::Idle);
}

#[test]
fn old_history_never_celebrates() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    // completed + lastTurnOutcome=completed at startup is history, not joy.
    let mut c = chat("a", "idle");
    c.last_turn_outcome = Some("completed".into());
    let d = e.update(&snap(vec![c, chat("b", "completed")], t0), None, t0);
    assert_eq!(d.emotion, Emotion::Idle);
}

#[test]
fn stale_and_unknown_data() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    let mut c = chat("a", "working");
    c.stale = true; // stale live report is unknown, not inferred working
    let d = e.update(&snap(vec![c], t0), None, t0);
    assert_eq!(d.emotion, Emotion::Subdued);
    assert!(d.badge.contains("unknown"));
    assert_eq!(d.counts.unknown, 1);
    // unknown cannot create completion celebrations even when it clears stale
    let mut e2 = Engine::new();
    let mut c1 = chat("a", "completed");
    c1.stale = true;
    let d = e2.update(&snap(vec![c1], t0), None, t0);
    assert_eq!(d.emotion, Emotion::Subdued);
}

#[test]
fn disconnect_and_reconnect_rebaselines() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    e.update(&snap(vec![chat("a", "working")], t0), None, t0);
    let d = e.on_disconnect();
    assert_eq!(d.emotion, Emotion::Disconnected);
    assert!(!d.connected);
    // Pre-existing error after reconnect is sad, not a fresh anger.
    let t1 = t0 + Duration::from_secs(12);
    let d = e.update(&snap(vec![chat("a", "errored")], t1), None, t1);
    assert_eq!(d.emotion, Emotion::Sadness);
}

#[test]
fn truncation_is_explicit() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    let mut s = snap(vec![chat("a", "working")], t0);
    s.total = 812;
    assert!(s.truncated());
    let d = e.update(&s, None, t0);
    assert!(d.counts.truncated);
    assert!(d.badge.contains("+"), "badge: {}", d.badge);
}

#[test]
fn selected_chat_narrows_aggregation() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    let mut child = chat("kid", "working");
    child.parent_chat_id = Some("a".into());
    let s = snap(vec![chat("a", "idle"), child, chat("b", "errored")], t0);
    // Selecting "a" also covers its child "kid".
    let d = e.update(&s, Some("a"), t0);
    assert_eq!(d.emotion, Emotion::Focus);
    assert_eq!(d.counts.shown, 2);
    // Selecting a missing chat shows an honest not-found state.
    let d = e.update(&s, Some("nope"), t0);
    assert_eq!(d.emotion, Emotion::Subdued);
    assert!(d.badge.contains("not found"));
}

#[test]
fn counts_and_summary() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    let s = snap(
        vec![
            chat("a", "working"),
            chat("b", "completed"),
            chat("c", "awaitingInput"),
            chat("d", "unknown-x"),
        ],
        t0,
    );
    let d = e.update(&s, None, t0);
    assert_eq!(d.counts.working, 1);
    assert_eq!(d.counts.completed, 1);
    assert_eq!(d.counts.awaiting_input, 1);
    assert_eq!(d.counts.unknown, 1);
    assert!(d.badge.contains("awaiting input"));
}

#[test]
fn observe_transport_down_is_immediate_disconnect() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    let s = snap(vec![chat("a", "working")], t0);
    // Fresh snapshot but transport Down: disconnected NOW, not in 10s.
    let d = e.observe(Some(&s), false, None, t0);
    assert_eq!(d.emotion, Emotion::Disconnected);
    assert!(!d.connected);
    // Connected transport but expired snapshot: same.
    let old = snap(vec![chat("a", "working")], t0);
    let t1 = t0 + FRESHNESS_LIMIT + Duration::from_secs(1);
    let d = e.observe(Some(&old), true, None, t1);
    assert_eq!(d.emotion, Emotion::Disconnected);
    // Reconnect rebaselines: a pre-existing completion is not celebrated.
    let d = e.observe(
        Some(&snap(vec![chat("a", "completed")], t1)),
        true,
        None,
        t1,
    );
    assert_eq!(d.emotion, Emotion::Idle);
}

#[test]
fn scope_change_rebaselines_episodes() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    let s = snap(vec![chat("a", "idle"), chat("b", "completed")], t0);
    // Baseline in All scope: b's completed is history.
    let d = e.update(&s, None, t0);
    assert_eq!(d.emotion, Emotion::Idle);
    // Narrowing to a then to b must not fabricate Joy from old state.
    let d = e.update(&s, Some("a"), t0 + Duration::from_secs(1));
    assert_eq!(d.emotion, Emotion::Idle);
    let d = e.update(&s, Some("b"), t0 + Duration::from_secs(2));
    assert_eq!(d.emotion, Emotion::Idle, "old completed in new scope");
    // ...but a real completion after the new baseline still celebrates.
    let s2 = snap(vec![chat("a", "idle"), chat("b", "working")], t0);
    let t3 = t0 + Duration::from_secs(3);
    e.update(&s2, Some("b"), t3);
    let d = e.update(
        &snap(vec![chat("a", "idle"), chat("b", "completed")], t3),
        Some("b"),
        t3 + Duration::from_secs(1),
    );
    assert_eq!(d.emotion, Emotion::Joy);
}

#[test]
fn scope_change_old_error_is_sad_not_angry() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    let s = snap(vec![chat("a", "idle"), chat("b", "errored")], t0);
    e.update(&s, None, t0);
    // Selecting the long-errored chat shows sadness, not a fresh anger.
    let d = e.update(&s, Some("b"), t0 + Duration::from_secs(1));
    assert_eq!(d.emotion, Emotion::Sadness);
    // Deselect back to All: prior history is still not celebrated.
    let s2 = snap(vec![chat("a", "idle"), chat("b", "completed")], t0);
    let d = e.update(&s2, None, t0 + Duration::from_secs(2));
    assert_ne!(d.emotion, Emotion::Joy);
}

#[test]
fn fresh_snapshot_required() {
    let t0 = Instant::now();
    let s = snap(vec![chat("a", "idle")], t0);
    assert!(s.fresh(t0));
    assert!(!s.fresh(t0 + FRESHNESS_LIMIT + Duration::from_millis(1)));
}

#[test]
fn status_kind_semantics() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    // Working.
    let d = e.update(&snap(vec![chat("a", "working")], t0), None, t0);
    assert_eq!(d.kind, StatusKind::Working);
    // Awaiting input beats working.
    let d = e.update(
        &snap(vec![chat("a", "working"), chat("b", "awaitingInput")], t0),
        None,
        t0,
    );
    assert_eq!(d.kind, StatusKind::AwaitingInput);
    // Errors beat working/completed.
    let d = e.update(
        &snap(
            vec![
                chat("a", "working"),
                chat("b", "errored"),
                chat("c", "completed"),
            ],
            t0,
        ),
        None,
        t0,
    );
    assert_eq!(d.kind, StatusKind::Error);
    // Completed history at startup: Success kind, Idle emotion, no joy.
    let mut e2 = Engine::new();
    let d = e2.update(&snap(vec![chat("a", "completed")], t0), None, t0);
    assert_eq!(d.kind, StatusKind::Success);
    assert_eq!(d.emotion, Emotion::Idle);
    // Fresh completion: Joy emotion AND Success kind.
    let mut e3 = Engine::new();
    e3.update(&snap(vec![chat("a", "working")], t0), None, t0);
    let d = e3.update(
        &snap(vec![chat("a", "completed")], t0 + Duration::from_secs(1)),
        None,
        t0 + Duration::from_secs(1),
    );
    assert_eq!(d.emotion, Emotion::Joy);
    assert_eq!(d.kind, StatusKind::Success);
    // All unknown -> Unknown, not Idle.
    let mut e4 = Engine::new();
    let d = e4.update(&snap(vec![chat("a", "bogus")], t0), None, t0);
    assert_eq!(d.kind, StatusKind::Unknown);
    // No chats -> Unknown.
    let mut e5 = Engine::new();
    let d = e5.update(&snap(vec![], t0), None, t0);
    assert_eq!(d.kind, StatusKind::Unknown);
    // Plain idle -> Idle.
    let mut e6 = Engine::new();
    let d = e6.update(&snap(vec![chat("a", "idle")], t0), None, t0);
    assert_eq!(d.kind, StatusKind::Idle);
    // Disconnect -> Disconnected regardless of prior state.
    let d = e6.observe(None, false, None, t0);
    assert_eq!(d.kind, StatusKind::Disconnected);
    // Working beats completed (no error/awaiting).
    let mut e7 = Engine::new();
    let d = e7.update(
        &snap(vec![chat("a", "working"), chat("b", "completed")], t0),
        None,
        t0,
    );
    assert_eq!(d.kind, StatusKind::Working);
}

#[test]
fn attention_episode_flags_only_new_input_or_error() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    // Baseline awaiting chat: established state, no episode.
    let d = e.update(&snap(vec![chat("a", "awaitingInput")], t0), None, t0);
    assert!(!d.attention_episode);
    // Same chat still awaiting: no new episode.
    let d = e.update(
        &snap(
            vec![chat("a", "awaitingInput")],
            t0 + Duration::from_secs(1),
        ),
        None,
        t0 + Duration::from_secs(1),
    );
    assert!(!d.attention_episode);
    // A second chat newly awaiting -> episode.
    let d = e.update(
        &snap(
            vec![chat("a", "awaitingInput"), chat("b", "awaitingInput")],
            t0 + Duration::from_secs(2),
        ),
        None,
        t0 + Duration::from_secs(2),
    );
    assert!(d.attention_episode);
    // Next update with the same set: episode consumed.
    let d = e.update(
        &snap(
            vec![chat("a", "awaitingInput"), chat("b", "awaitingInput")],
            t0 + Duration::from_secs(3),
        ),
        None,
        t0 + Duration::from_secs(3),
    );
    assert!(!d.attention_episode);
    // New errored chat -> episode (also retriggers anger window).
    let d = e.update(
        &snap(
            vec![
                chat("a", "awaitingInput"),
                chat("b", "awaitingInput"),
                chat("c", "errored"),
            ],
            t0 + Duration::from_secs(4),
        ),
        None,
        t0 + Duration::from_secs(4),
    );
    assert!(d.attention_episode);
    assert_eq!(d.emotion, Emotion::Question); // awaiting still wins
}

#[test]
fn completion_episode_only_on_fresh_completion() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    // Baseline working: no episode.
    let d = e.update(&snap(vec![chat("a", "working")], t0), None, t0);
    assert!(!d.completion_episode);
    // Fresh completion -> episode + Joy.
    let t1 = t0 + Duration::from_secs(1);
    let d = e.update(&snap(vec![chat("a", "completed")], t1), None, t1);
    assert!(d.completion_episode);
    assert_eq!(d.emotion, Emotion::Joy);
    // Same completed chat next update: episode consumed, no re-fire.
    let t2 = t1 + Duration::from_secs(1);
    let d = e.update(&snap(vec![chat("a", "completed")], t2), None, t2);
    assert!(!d.completion_episode);
    // A SECOND fresh completion while already Joy: new episode again.
    let t3 = t2 + Duration::from_secs(1);
    let d = e.update(
        &snap(vec![chat("a", "completed"), chat("b", "completed")], t3),
        None,
        t3,
    );
    assert!(d.completion_episode);
}

#[test]
fn completion_episode_never_from_baseline_or_reconnect() {
    let t0 = Instant::now();
    // Startup history: completed already present -> no episode.
    let mut e = Engine::new();
    let d = e.update(&snap(vec![chat("a", "completed")], t0), None, t0);
    assert!(!d.completion_episode);
    assert_eq!(d.emotion, Emotion::Idle);
    // Reconnect baselines again: pre-existing completed is not fresh.
    e.on_disconnect();
    let d = e.update(
        &snap(
            vec![chat("a", "completed"), chat("b", "completed")],
            t0 + Duration::from_secs(5),
        ),
        None,
        t0 + Duration::from_secs(5),
    );
    assert!(!d.completion_episode);
}

#[test]
fn completion_episode_survives_interruption_without_refiring() {
    let t0 = Instant::now();
    let mut e = Engine::new();
    e.update(&snap(vec![chat("a", "working")], t0), None, t0);
    let t1 = t0 + Duration::from_secs(1);
    let d = e.update(&snap(vec![chat("a", "completed")], t1), None, t1);
    assert!(d.completion_episode);
    // Higher-priority interruption (working appears elsewhere): episode
    // flag is consumed once and does NOT refire when Joy resumes.
    let t2 = t1 + Duration::from_secs(1);
    let d = e.update(
        &snap(vec![chat("a", "completed"), chat("b", "working")], t2),
        None,
        t2,
    );
    assert!(!d.completion_episode);
    assert_eq!(d.emotion, Emotion::Focus);
    let t3 = t2 + Duration::from_secs(1);
    let d = e.update(&snap(vec![chat("a", "completed")], t3), None, t3);
    assert!(!d.completion_episode);
    assert_eq!(d.emotion, Emotion::Joy); // back to Joy, spent budget kept
}
