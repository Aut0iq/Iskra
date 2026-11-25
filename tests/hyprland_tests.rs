use zeron_pet::hyprland::*;

fn monitors_json() -> String {
    r#"[
      {"id":0,"name":"eDP-1","width":1920,"height":1080,"scale":1.0,"transform":0,
       "reserved":[0,40,0,0],"focused":true,
       "activeWorkspace":{"id":3,"name":"3"},"specialWorkspace":{"id":0,"name":""}},
      {"id":1,"name":"HDMI-A-1","width":2560,"height":1440,"scale":1.5,"transform":0,
       "reserved":[0,0,0,0],"focused":false,
       "activeWorkspace":{"id":1,"name":"1"},"specialWorkspace":{"id":0,"name":""}}
    ]"#
    .to_string()
}

fn client(mapped: bool, fs: i64, fsc: i64, ws: i64) -> String {
    format!(
        r#"{{"mapped":{mapped},"fullscreen":{fs},"fullscreenClient":{fsc},
           "workspace":{{"id":{ws},"name":"{ws}"}},"monitor":0}}"#
    )
}

fn clients_json(items: &[String]) -> String {
    format!("[{}]", items.join(","))
}

#[test]
fn parses_monitors_with_scale_and_reserved() {
    let m = parse_monitors(&monitors_json()).unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].name, "eDP-1");
    assert_eq!((m[0].logical_w, m[0].logical_h), (1920, 1080));
    // Fractional scale: 2560/1.5 = 1706.67 -> 1707.
    assert_eq!((m[1].logical_w, m[1].logical_h), (1707, 960));
    assert_eq!(m[0].reserved, [0, 40, 0, 0]);
    assert!(m[0].focused);
}

#[test]
fn rotation_swaps_logical_size() {
    let j = r#"[{"id":0,"name":"X","width":1920,"height":1080,"scale":1.0,
        "transform":1,"reserved":[0,0,0,0],"focused":true,
        "activeWorkspace":{"id":1},"specialWorkspace":{"id":0}}]"#;
    let m = parse_monitors(j).unwrap();
    assert_eq!((m[0].logical_w, m[0].logical_h), (1080, 1920));
}

#[test]
fn no_fullscreen_is_visible() {
    let m = parse_monitors(&monitors_json()).unwrap();
    let c = parse_clients(&clients_json(&[client(true, 0, 0, 3)])).unwrap();
    assert_eq!(decide_visibility(&m, &c, "eDP-1"), Visibility::Visible);
}

#[test]
fn fullscreen_on_active_workspace_hides() {
    let m = parse_monitors(&monitors_json()).unwrap();
    // eFullscreenState >= 2: 2 = fullscreen, 3 = maximized+fullscreen.
    for (fs, fsc) in [(2, 0), (0, 2), (3, 0), (0, 3)] {
        let c = parse_clients(&clients_json(&[client(true, fs, fsc, 3)])).unwrap();
        assert!(matches!(
            decide_visibility(&m, &c, "eDP-1"),
            Visibility::HiddenFullscreen { .. }
        ));
    }
}

#[test]
fn maximized_on_active_workspace_does_not_hide() {
    let m = parse_monitors(&monitors_json()).unwrap();
    // eFullscreenState 1 = maximized: a maximized window does not cover
    // a layer-shell surface.
    for (fs, fsc) in [(1, 0), (0, 1), (1, 1)] {
        let c = parse_clients(&clients_json(&[client(true, fs, fsc, 3)])).unwrap();
        assert_eq!(decide_visibility(&m, &c, "eDP-1"), Visibility::Visible);
    }
    // Legacy payload without fullscreenClient: nonzero is fullscreen.
    let c = parse_clients(&clients_json(&[
        r#"{"mapped":true,"fullscreen":1,"workspace":{"id":3,"name":"3"},"monitor":0}"#.to_string(),
    ]))
    .unwrap();
    assert!(matches!(
        decide_visibility(&m, &c, "eDP-1"),
        Visibility::HiddenFullscreen { .. }
    ));
}

#[test]
fn fullscreen_elsewhere_does_not_hide() {
    let m = parse_monitors(&monitors_json()).unwrap();
    // Other workspace, other monitor's workspace, unmapped client.
    for cj in [
        client(true, 2, 0, 7),
        client(true, 2, 0, 1), // ws 1 is active on HDMI, not eDP
        client(false, 2, 0, 3),
    ] {
        let c = parse_clients(&clients_json(&[cj])).unwrap();
        assert_eq!(decide_visibility(&m, &c, "eDP-1"), Visibility::Visible);
    }
}

#[test]
fn special_workspace_fullscreen_hides_when_shown() {
    let j = r#"[{"id":0,"name":"eDP-1","width":1920,"height":1080,"scale":1.0,
        "transform":0,"reserved":[0,0,0,0],"focused":true,
        "activeWorkspace":{"id":3},"specialWorkspace":{"id":-99,"name":"special:magic"}}]"#;
    let m = parse_monitors(j).unwrap();
    assert_eq!(m[0].special_workspace, Some(-99));
    let c = parse_clients(&clients_json(&[client(true, 2, 0, -99)])).unwrap();
    assert!(matches!(
        decide_visibility(&m, &c, "eDP-1"),
        Visibility::HiddenFullscreen { .. }
    ));
    // Same client, special workspace not shown -> visible.
    let m2 = parse_monitors(&monitors_json()).unwrap();
    assert_eq!(decide_visibility(&m2, &c, "eDP-1"), Visibility::Visible);
}

#[test]
fn unknown_output_hides_conservatively() {
    let m = parse_monitors(&monitors_json()).unwrap();
    let c = parse_clients(&clients_json(&[])).unwrap();
    assert_eq!(decide_visibility(&m, &c, "DP-9"), Visibility::HiddenUnknown);
    // Fullscreen exists somewhere we cannot map: still hidden.
    let c = parse_clients(&clients_json(&[client(true, 2, 0, 42)])).unwrap();
    assert!(matches!(
        decide_visibility(&m, &c, "DP-9"),
        Visibility::HiddenFullscreen { .. }
    ));
}

#[test]
fn unplug_chooses_remaining_monitor() {
    let one = parse_monitors(
        r#"[{"id":1,"name":"HDMI-A-1","width":100,"height":100,"scale":1.0,
            "transform":0,"reserved":[0,0,0,0],"focused":false,
            "activeWorkspace":{"id":1},"specialWorkspace":{"id":0}}]"#,
    )
    .unwrap();
    assert_eq!(
        choose_monitor(&one, Some("eDP-1")).unwrap().name,
        "HDMI-A-1"
    );
}

#[test]
fn usable_bounds_respect_reserved() {
    let m = parse_monitors(&monitors_json()).unwrap();
    // Margins are relative to the reserved-adjusted anchor rect.
    let b = usable_bounds(&m[0], 192, 256);
    assert_eq!((b.min_x, b.min_y), (0, 0));
    assert_eq!(b.max_x, 1920 - 192);
    assert_eq!(b.max_y, 1080 - 40 - 256);
    // Tiny output: clamps to min.
    let j = r#"[{"id":0,"name":"X","width":100,"height":80,"scale":1.0,
        "transform":0,"reserved":[0,0,0,0],"focused":true,
        "activeWorkspace":{"id":1},"specialWorkspace":{"id":0}}]"#;
    let m = parse_monitors(j).unwrap();
    let b = usable_bounds(&m[0], 192, 256);
    assert_eq!((b.max_x, b.max_y), (0, 0));
}

#[test]
fn monitor_apply_initial_stable_and_fallback() {
    let m = parse_monitors(&monitors_json()).unwrap();
    // Explicit preferred connector: first state must apply it.
    assert_eq!(
        monitor_to_apply(&m, Some("eDP-1"), None),
        Some("eDP-1".to_string())
    );
    // Repeated identical state: no redundant switch.
    assert_eq!(monitor_to_apply(&m, Some("eDP-1"), Some("eDP-1")), None);
    // No preference: focused monitor is chosen once.
    assert_eq!(monitor_to_apply(&m, None, None), Some("eDP-1".to_string()));
    // Focus changing later does not move the pet.
    assert_eq!(monitor_to_apply(&m, Some("eDP-1"), Some("eDP-1")), None);
}

#[test]
fn monitor_apply_unplug_fallback() {
    let only_hdmi = parse_monitors(
        r#"[{"id":1,"name":"HDMI-A-1","width":100,"height":100,"scale":1.0,
            "transform":0,"reserved":[0,0,0,0],"focused":true,
            "activeWorkspace":{"id":1},"specialWorkspace":{"id":0}}]"#,
    )
    .unwrap();
    // Preferred eDP-1 vanished: fallback applies, then sticks.
    assert_eq!(
        monitor_to_apply(&only_hdmi, Some("eDP-1"), Some("eDP-1")),
        Some("HDMI-A-1".to_string())
    );
    assert_eq!(
        monitor_to_apply(&only_hdmi, Some("eDP-1"), Some("HDMI-A-1")),
        None
    );
}

#[test]
fn malformed_json_is_an_error() {
    assert!(parse_monitors("not json").is_err());
    assert!(parse_monitors("{}").is_err());
    assert!(parse_clients("[").is_err());
}
