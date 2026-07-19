//! In-mode key handler for `Mode::AgentFocus`.
//!
//! `focus_agents` (default `prefix+a`) enters this submode. While inside it,
//! `j`/`Down` and `k`/`Up` move a preview cursor through the sidebar Agents
//! panel without changing focus, `Enter` focuses the selected agent's pane and
//! exits, and `Esc` exits without changing focus. The cursor lives entirely in
//! `AppState::agent_panel_focus`; the server/socket/wire protocol is unaware.

use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{state::AppState, App};

impl App {
    pub(crate) fn handle_agent_focus_key(&mut self, key: KeyEvent) {
        // Plain modifier keys (shift alone, etc.) and bare modifier combos are
        // ignored so the cursor only moves on real j/k/arrow presses.
        match key.code {
            KeyCode::Esc => {
                self.state.leave_agent_focus_mode();
            }
            KeyCode::Enter => {
                self.state.focus_selected_agent();
                self.state.leave_agent_focus_mode();
            }
            KeyCode::Char('j') | KeyCode::Down if key.modifiers.is_empty() => {
                move_agent_focus_cursor(&mut self.state, 1);
            }
            KeyCode::Char('k') | KeyCode::Up if key.modifiers.is_empty() => {
                move_agent_focus_cursor(&mut self.state, -1);
            }
            _ => {}
        }
    }
}

/// Move the `Mode::AgentFocus` cursor by `delta` (negative = up) with wrap
/// around. Re-fetches `agent_panel_entries` on every call because the panel
/// contents (and ordering, when `agent_panel_sort` toggles) can change between
/// keypresses. The cursor tracks the neighbor's `pane_id`, not its index, so
/// a later re-sort does not move the highlight to the wrong row. No-op when
/// the panel is empty or the stored `pane_id` is no longer present.
fn move_agent_focus_cursor(state: &mut AppState, delta: isize) {
    let focus = match state.agent_panel_focus.as_ref() {
        Some(focus) => *focus,
        None => return,
    };
    let entries = crate::ui::agent_panel_entries(state);
    let len = entries.len();
    if len == 0 {
        return;
    }
    let current = entries
        .iter()
        .position(|entry| entry.pane_id == focus.pane_id);
    let next_idx = match current {
        Some(idx) => {
            if delta >= 0 {
                (idx + delta as usize) % len
            } else {
                (idx as isize + delta).rem_euclid(len as isize) as usize
            }
        }
        None => 0,
    };
    let Some(target) = entries.get(next_idx) else {
        return;
    };
    if let Some(focus) = state.agent_panel_focus.as_mut() {
        focus.pane_id = target.pane_id;
    }
    state.ensure_agent_panel_entry_visible(next_idx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{AgentFocusState, Mode};
    use crate::detect::{Agent, AgentState};
    use crate::layout::PaneId;
    use crate::workspace::Workspace;

    fn mark_agent(state: &mut AppState, ws_idx: usize, tab_idx: usize, pane_id: PaneId) {
        state.ensure_test_terminals();
        let terminal_id = state.workspaces[ws_idx].tabs[tab_idx]
            .panes
            .get(&pane_id)
            .expect("pane should exist")
            .attached_terminal_id
            .clone();
        if let Some(terminal) = state.terminals.get_mut(&terminal_id) {
            terminal.set_detected_state(Some(Agent::Pi), AgentState::Idle);
        }
    }

    fn state_with_three_agents() -> (AppState, PaneId, PaneId, PaneId) {
        let mut first = Workspace::test_new("one");
        let first_root = first.tabs[0].root_pane;
        let first_second = first.test_split(ratatui::layout::Direction::Horizontal);
        first.tabs[0].layout.focus_pane(first_root);
        let second = Workspace::test_new("two");
        let second_root = second.tabs[0].root_pane;

        let mut state = AppState::test_new();
        state.workspaces = vec![first, second];
        state.ensure_test_terminals();
        state.active = Some(0);
        state.selected = 0;
        state.mode = Mode::Terminal;
        mark_agent(&mut state, 0, 0, first_root);
        mark_agent(&mut state, 0, 0, first_second);
        mark_agent(&mut state, 1, 0, second_root);
        (state, first_root, first_second, second_root)
    }

    #[test]
    fn j_moves_cursor_down_with_wrap() {
        let (mut state, first_root, first_second, second_root) = state_with_three_agents();
        state.enter_agent_focus_mode();
        assert_eq!(state.agent_panel_focus.unwrap().pane_id, first_root);

        move_agent_focus_cursor(&mut state, 1);
        assert_eq!(state.agent_panel_focus.unwrap().pane_id, first_second);

        move_agent_focus_cursor(&mut state, 1);
        assert_eq!(state.agent_panel_focus.unwrap().pane_id, second_root);

        // Wrap back to the first entry from the last.
        move_agent_focus_cursor(&mut state, 1);
        assert_eq!(state.agent_panel_focus.unwrap().pane_id, first_root);
        state.assert_invariants_for_test();
    }

    #[test]
    fn k_moves_cursor_up_with_wrap() {
        let (mut state, first_root, first_second, second_root) = state_with_three_agents();
        state.enter_agent_focus_mode();
        // Cursor starts on first_root; going up should wrap to the last entry.
        move_agent_focus_cursor(&mut state, -1);
        assert_eq!(state.agent_panel_focus.unwrap().pane_id, second_root);

        move_agent_focus_cursor(&mut state, -1);
        assert_eq!(state.agent_panel_focus.unwrap().pane_id, first_second);

        move_agent_focus_cursor(&mut state, -1);
        assert_eq!(state.agent_panel_focus.unwrap().pane_id, first_root);
        state.assert_invariants_for_test();
    }

    #[test]
    fn move_is_noop_when_not_in_mode() {
        let (mut state, _, _, _) = state_with_three_agents();
        // Never entered AgentFocus, so cursor move must do nothing.
        move_agent_focus_cursor(&mut state, 1);
        assert!(state.agent_panel_focus.is_none());
    }

    #[test]
    fn move_recovers_when_stored_pane_id_no_longer_in_panel() {
        let (mut state, first_root, _, _) = state_with_three_agents();
        state.enter_agent_focus_mode();
        // Inject a stale pane_id that is not in the panel. The cursor move
        // should fall back to index 0 (the first entry) instead of panicking
        // or computing a wrong neighbor from a non-existent index.
        state.agent_panel_focus = Some(AgentFocusState {
            pane_id: PaneId::from_raw(u32::MAX),
        });
        move_agent_focus_cursor(&mut state, 1);
        assert_eq!(state.agent_panel_focus.unwrap().pane_id, first_root);
        state.assert_invariants_for_test();
    }

    #[test]
    fn shift_up_does_not_move_cursor() {
        // Regression: previously `k`/Up accepted Shift (unlike `j`/Down),
        // so Shift+Up moved the cursor while Shift+Down was ignored. After
        // the fix, both arms require empty modifiers, so a synthetic
        // Shift+Up key event must NOT move the cursor.
        let (state, first_root, _, _) = state_with_three_agents();
        let mut app = App::new(
            &crate::config::Config::default(),
            true,
            None,
            tokio::sync::mpsc::unbounded_channel().1,
            crate::api::EventHub::default(),
        );
        app.state = state;
        app.state.enter_agent_focus_mode();
        assert_eq!(app.state.agent_panel_focus.unwrap().pane_id, first_root);

        app.handle_agent_focus_key(KeyEvent::new(
            KeyCode::Up,
            crossterm::event::KeyModifiers::SHIFT,
        ));
        assert_eq!(
            app.state.agent_panel_focus.unwrap().pane_id,
            first_root,
            "Shift+Up must not move the AgentFocus cursor"
        );
    }
}
