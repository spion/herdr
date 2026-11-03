use super::*;

/// One tile showing `pane_1`, with `pane_2` as its hidden second tab.
fn stacked_tile() -> (ClientShellSnapshot, PaneSurfaceFrame) {
    let mut snapshot = snapshot();
    snapshot.panes[0].stack_id = Some("s1".into());
    snapshot.panes[0].label = Some("agent".into());
    let mut logs = snapshot.panes[0].clone();
    logs.pane_id = "pane_2".into();
    logs.label = Some("logs".into());
    logs.focused = false;
    logs.hidden = true;
    snapshot.panes.push(logs);

    let mut surface = surface();
    surface.frame = FrameData::from_ratatui_buffer_with_hyperlinks(
        &Buffer::empty(Rect::new(0, 0, 40, 6)),
        None,
        &[],
    );
    surface.panes[0].rect = SurfaceRect {
        x: 0,
        y: 0,
        width: 40,
        height: 6,
    };
    surface.panes[0].inner_rect = SurfaceRect {
        x: 1,
        y: 1,
        width: 38,
        height: 4,
    };
    (snapshot, surface)
}

fn click(state: &mut ClientShellState, rect: Rect) -> ClientShellInput {
    state.handle_raw_events(vec![RawInputEvent::Mouse(crossterm::event::MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: rect.x + 1,
        row: rect.y,
        modifiers: KeyModifiers::empty(),
    })])
}

#[test]
fn tile_strip_lists_its_tabs_and_routes_clicks_to_the_pane_api() {
    let (snapshot, surface) = stacked_tile();
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface);

    let frame = state.compose(106, 20).expect("composed frame");
    let tabs: Vec<_> = state
        .hits
        .stack_tabs
        .iter()
        .map(|(_, pane_id)| pane_id.as_str())
        .collect();
    assert_eq!(tabs, ["pane_1", "pane_2"]);
    let strip_row = state.hits.stack_tabs[0].0.y as usize;
    let row: String = frame.cells[strip_row * frame.width as usize..][..frame.width as usize]
        .iter()
        .map(|cell| cell.symbol.as_str())
        .collect();
    assert!(
        row.contains("agent") && row.contains("logs"),
        "row: {row:?}"
    );

    let logs = state.hits.stack_tabs[1].0;
    let select = click(&mut state, logs);
    let [ClientShellAction::Endpoint { request, .. }] = &select.actions[..] else {
        panic!("expected one endpoint action: {:?}", select.actions);
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneFocus(target) if target.pane_id == "pane_2"
    ));

    let new_tab = state.hits.new_stack_tabs[0].0;
    let add = click(&mut state, new_tab);
    let [ClientShellAction::Endpoint { request, .. }] = &add.actions[..] else {
        panic!("expected one endpoint action: {:?}", add.actions);
    };
    assert!(matches!(
        &request.method,
        crate::api::schema::Method::PaneAddTab(params) if params.pane_id == "pane_1" && params.focus
    ));
}

#[test]
fn single_tab_tiles_and_old_servers_show_no_strip() {
    let (mut snapshot, surface) = stacked_tile();
    snapshot.panes.truncate(1);
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&Config::default()));
    state.set_snapshot(Box::new(snapshot.clone()));
    state.set_pane_surface(surface.clone());
    state.compose(106, 20).expect("composed frame");
    assert!(state.hits.stack_tabs.is_empty());
    assert_eq!(state.hits.new_stack_tabs.len(), 1, "focused tile offers +");

    snapshot.panes[0].stack_id = None;
    state.set_snapshot(Box::new(snapshot));
    state.set_pane_surface(surface);
    state.compose(106, 20).expect("composed frame");
    assert!(state.hits.stack_tabs.is_empty());
    assert!(state.hits.new_stack_tabs.is_empty());
}
