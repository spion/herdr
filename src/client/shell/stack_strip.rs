//! Per-tile tab strips.
//!
//! The server reserves the top border row of every tile whose stack holds
//! several panes and leaves its title off. This module draws the tile's tabs
//! there, plus a `+` that opens another tab. The focused single-tab tile offers
//! only the `+`, at the right end of its top border.
//!
//! The pane surface is blitted over the chrome buffer, so the strip is drawn
//! into the chrome buffer and its cells are copied back after the blit. Only
//! the cells of tab entries and buttons are copied, so the server's border
//! stays visible between them.
//!
//! A dragged tab lands on a strip at the gap under the mouse, on the middle of
//! a tile at the end of its strip, and on the outer quarter of a tile in a new
//! tile on that side. The drop target is marked the same way as the strip, so
//! marks are lines rather than a wash over the pane's content.

use super::*;
use crate::api::schema::{Method, PaneDirection, PaneMoveTabParams, PaneSplitTabParams};
use crate::protocol::{CellData, ClientShellPane};

const MAX_LABEL_WIDTH: usize = 24;
const MIN_ENTRY_WIDTH: u16 = 5;
const NEW_TAB_LABEL: &str = " + ";
const NEW_TAB_WIDTH: u16 = 3;

#[derive(Default)]
pub(in crate::client::shell) struct StackStrips {
    /// Tab entries, each with the pane it shows.
    pub(in crate::client::shell) tabs: Vec<(Rect, String)>,
    /// `+` buttons, each with a pane of the tile it adds to.
    pub(in crate::client::shell) new_tabs: Vec<(Rect, String)>,
    /// One-row pieces of the drop marker.
    drop_marks: Vec<Rect>,
}

impl StackStrips {
    fn drawn(&self) -> impl Iterator<Item = Rect> + '_ {
        self.tabs
            .iter()
            .chain(&self.new_tabs)
            .map(|(rect, _)| *rect)
            .chain(self.drop_marks.iter().copied())
    }

    /// Copy the drawn cells out of `frame` before the pane surface covers them.
    pub(in crate::client::shell) fn save(&self, frame: &FrameData) -> Vec<(Rect, Vec<CellData>)> {
        self.drawn()
            .map(|rect| (rect, frame.cells[row_cells(frame, rect)].to_vec()))
            .collect()
    }

    pub(in crate::client::shell) fn restore(
        frame: &mut FrameData,
        saved: &[(Rect, Vec<CellData>)],
    ) {
        for (rect, cells) in saved {
            let range = row_cells(frame, *rect);
            frame.cells[range].clone_from_slice(cells);
        }
    }
}

fn row_cells(frame: &FrameData, rect: Rect) -> std::ops::Range<usize> {
    let start = usize::from(rect.y) * usize::from(frame.width) + usize::from(rect.x);
    start..start + usize::from(rect.width)
}

pub(in crate::client::shell) fn render_stack_strips(
    buffer: &mut Buffer,
    panes: &[PaneHit],
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    drop: Option<&StackTabDrop>,
) -> StackStrips {
    let mut strips = StackStrips::default();
    for hit in panes.iter().filter(|hit| !hit.popup) {
        // Tiles without a top border have no row for a strip.
        if hit.inner_rect.y <= hit.rect.y || hit.rect.width < 2 + NEW_TAB_WIDTH {
            continue;
        }
        let Some(shown) = snapshot
            .panes
            .iter()
            .find(|pane| pane.pane_id == hit.pane_id)
        else {
            continue;
        };
        let tabs = tabs_of_tile(snapshot, &hit.pane_id);
        if tabs.is_empty() {
            continue;
        }
        let row = Rect::new(hit.rect.x + 1, hit.rect.y, hit.rect.width - 2, 1);
        if tabs.len() > 1 {
            render_strip(
                buffer,
                row,
                &tabs,
                shown.focused,
                snapshot,
                config,
                &mut strips,
            );
        } else if shown.focused && config.mouse_capture {
            let rect = Rect::new(row.right() - NEW_TAB_WIDTH, row.y, NEW_TAB_WIDTH, 1);
            put_new_tab_button(buffer, rect, config);
            strips.new_tabs.push((rect, shown.pane_id.clone()));
        }
    }
    if let Some(drop) = drop {
        strips.drop_marks = drop_marks(drop, panes, snapshot, &strips.tabs);
        let accent = Style::default().bg(config.palette.accent);
        for mark in &strips.drop_marks {
            buffer.set_style(*mark, accent);
        }
    }
    strips
}

/// The panes of the tile holding `pane_id`, in tab order. Empty when the
/// server does not report tiles.
pub(in crate::client::shell) fn tabs_of_tile<'a>(
    snapshot: &'a ClientShellSnapshot,
    pane_id: &str,
) -> Vec<&'a ClientShellPane> {
    let stack_id = snapshot
        .panes
        .iter()
        .find(|pane| pane.pane_id == pane_id)
        .and_then(|pane| pane.stack_id.as_deref());
    snapshot
        .panes
        .iter()
        .filter(|pane| stack_id.is_some() && pane.stack_id.as_deref() == stack_id)
        .collect()
}

pub(in crate::client::shell) fn drop_at(
    stack_tabs: &[(Rect, String)],
    panes: &[PaneHit],
    snapshot: &ClientShellSnapshot,
    dragged: &str,
    point: (u16, u16),
) -> Option<StackTabDrop> {
    if let Some((rect, pane_id)) = stack_tabs.iter().find(|(rect, _)| contains(*rect, point)) {
        let tabs = tabs_of_tile(snapshot, pane_id);
        let position = tabs.iter().position(|tab| tab.pane_id == *pane_id)?;
        let past_middle = point.0 >= rect.x + rect.width / 2;
        return Some(StackTabDrop::Tile {
            pane_id: pane_id.clone(),
            index: position + usize::from(past_middle),
        });
    }
    let hit = panes
        .iter()
        .find(|hit| !hit.popup && contains(hit.rect, point))?;
    let tabs = tabs_of_tile(snapshot, &hit.pane_id);
    if tabs.iter().all(|tab| tab.pane_id == dragged) {
        return None;
    }
    let pane_id = hit.pane_id.clone();
    match edge_side(hit.rect, point).filter(|_| point.1 != hit.rect.y) {
        Some(side) => Some(StackTabDrop::Edge { pane_id, side }),
        None => Some(StackTabDrop::Tile {
            pane_id,
            index: tabs.len(),
        }),
    }
}

/// The side of `rect` whose outer quarter holds `point`, the nearest one in a
/// corner.
fn edge_side(rect: Rect, (x, y): (u16, u16)) -> Option<PaneDirection> {
    let fraction = |distance: u16, size: u16| f32::from(distance) / f32::from(size.max(1));
    [
        (PaneDirection::Left, fraction(x - rect.x, rect.width)),
        (
            PaneDirection::Right,
            fraction(rect.right() - 1 - x, rect.width),
        ),
        (PaneDirection::Up, fraction(y - rect.y, rect.height)),
        (
            PaneDirection::Down,
            fraction(rect.bottom() - 1 - y, rect.height),
        ),
    ]
    .into_iter()
    .filter(|(_, fraction)| *fraction < 0.25)
    .min_by(|a, b| a.1.total_cmp(&b.1))
    .map(|(side, _)| side)
}

/// The request that carries out `drop`, or None when it would change nothing.
pub(in crate::client::shell) fn drop_request(
    snapshot: &ClientShellSnapshot,
    dragged: String,
    drop: StackTabDrop,
) -> Option<Method> {
    match drop {
        StackTabDrop::Edge { pane_id, side } => Some(Method::PaneSplitTab(PaneSplitTabParams {
            pane_id: dragged,
            target_pane_id: Some(pane_id),
            side,
        })),
        StackTabDrop::Tile { pane_id, index } => {
            let from = tabs_of_tile(snapshot, &pane_id)
                .iter()
                .position(|tab| tab.pane_id == dragged);
            // `index` counts the dragged tab, which leaves its slot first.
            let index = match from {
                Some(from) if from == index || from + 1 == index => return None,
                Some(from) if from < index => index - 1,
                _ => index,
            };
            Some(Method::PaneMoveTab(PaneMoveTabParams {
                pane_id: dragged,
                target_pane_id: Some(pane_id),
                index,
            }))
        }
    }
}

fn drop_marks(
    drop: &StackTabDrop,
    panes: &[PaneHit],
    snapshot: &ClientShellSnapshot,
    entries: &[(Rect, String)],
) -> Vec<Rect> {
    let tile = |pane_id: &str| {
        panes
            .iter()
            .find(|hit| hit.pane_id == pane_id)
            .map(|hit| hit.rect)
    };
    match drop {
        StackTabDrop::Edge { pane_id, side } => tile(pane_id)
            .map(|rect| outline(half_toward(rect, *side)))
            .unwrap_or_default(),
        StackTabDrop::Tile { pane_id, index } => {
            let tabs = tabs_of_tile(snapshot, pane_id);
            let entry = |position: usize| {
                let tab = tabs.get(position)?;
                let (rect, _) = entries.iter().find(|(_, id)| *id == tab.pane_id)?;
                Some(*rect)
            };
            let gap = match index.checked_sub(1).and_then(entry) {
                Some(before) => Some(before.right()),
                None => entry(0).map(|first| first.x.saturating_sub(1)),
            };
            match (gap, entry(0)) {
                (Some(x), Some(first)) => vec![Rect::new(x, first.y, 1, 1)],
                _ => tile(pane_id).map(outline).unwrap_or_default(),
            }
        }
    }
}

fn half_toward(rect: Rect, side: PaneDirection) -> Rect {
    let (half_width, half_height) = (rect.width / 2, rect.height / 2);
    match side {
        PaneDirection::Left => Rect {
            width: half_width,
            ..rect
        },
        PaneDirection::Right => Rect {
            x: rect.right() - half_width,
            width: half_width,
            ..rect
        },
        PaneDirection::Up => Rect {
            height: half_height,
            ..rect
        },
        PaneDirection::Down => Rect {
            y: rect.bottom() - half_height,
            height: half_height,
            ..rect
        },
    }
}

/// The edge of `rect`, as one-row pieces.
fn outline(rect: Rect) -> Vec<Rect> {
    if rect.is_empty() {
        return Vec::new();
    }
    let mut pieces = vec![Rect { height: 1, ..rect }];
    for y in rect.y + 1..rect.bottom().saturating_sub(1) {
        pieces.push(Rect::new(rect.x, y, 1, 1));
        pieces.push(Rect::new(rect.right() - 1, y, 1, 1));
    }
    if rect.height > 1 {
        pieces.push(Rect {
            y: rect.bottom() - 1,
            height: 1,
            ..rect
        });
    }
    pieces
}

fn render_strip(
    buffer: &mut Buffer,
    row: Rect,
    tabs: &[&ClientShellPane],
    tile_focused: bool,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    strips: &mut StackStrips,
) {
    let palette = &config.palette;
    let button_width = if config.mouse_capture {
        NEW_TAB_WIDTH
    } else {
        0
    };
    let entries_right = row.right() - button_width;
    let entries: Vec<(Option<crate::api::schema::AgentStatus>, String)> =
        tabs.iter().map(|pane| tab_entry(pane, snapshot)).collect();
    let widths = entry_widths(&entries, entries_right - row.x);

    let mut x = row.x;
    for ((pane, (status, label)), width) in tabs.iter().zip(&entries).zip(widths) {
        if x + width > entries_right {
            break;
        }
        let style = match (!pane.hidden, tile_focused) {
            (true, true) => Style::default()
                .fg(panel_contrast_fg(palette))
                .bg(palette.accent)
                .add_modifier(Modifier::BOLD),
            (true, false) => Style::default()
                .fg(palette.text)
                .bg(palette.surface0)
                .add_modifier(Modifier::BOLD),
            (false, _) => Style::default().fg(palette.overlay1).bg(palette.surface0),
        };
        let rect = Rect::new(x, row.y, width, 1);
        buffer.set_style(rect, style);
        let mut text_x = x + 1;
        let mut text_width = width - 2;
        if let Some(status) = status {
            let icon = status_icon(*status, config.status_indicators);
            put_text(
                buffer,
                text_x,
                row.y,
                1,
                icon,
                style.fg(status_color(*status, palette)),
            );
            text_x += 2;
            text_width = text_width.saturating_sub(2);
        }
        let label = crate::ui::truncate_end(label, usize::from(text_width));
        put_text(buffer, text_x, row.y, text_width, &label, style);
        if config.mouse_capture {
            strips.tabs.push((rect, pane.pane_id.clone()));
        }
        x += width + 1;
    }
    if config.mouse_capture {
        let rect = Rect::new(x.min(entries_right), row.y, NEW_TAB_WIDTH, 1);
        put_new_tab_button(buffer, rect, config);
        strips.new_tabs.push((rect, tabs[0].pane_id.clone()));
    }
}

/// Entry widths: each tab's natural width, shrunk evenly when the row is short.
fn entry_widths(
    entries: &[(Option<crate::api::schema::AgentStatus>, String)],
    available: u16,
) -> Vec<u16> {
    let natural: Vec<u16> = entries
        .iter()
        .map(|(status, label)| {
            let label = display_width(label).min(MAX_LABEL_WIDTH as u16);
            label + 2 + if status.is_some() { 2 } else { 0 }
        })
        .collect();
    let gaps = natural.len().saturating_sub(1) as u16;
    let total = natural.iter().sum::<u16>() + gaps;
    if total <= available {
        return natural;
    }
    let share = (available.saturating_sub(gaps) / natural.len().max(1) as u16).max(MIN_ENTRY_WIDTH);
    natural.into_iter().map(|width| width.min(share)).collect()
}

/// A tab's agent status, when it runs an agent, and its label.
fn tab_entry(
    pane: &ClientShellPane,
    snapshot: &ClientShellSnapshot,
) -> (Option<crate::api::schema::AgentStatus>, String) {
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane.pane_id);
    let label = pane
        .label
        .clone()
        .or_else(|| agent.and_then(|agent| agent.name.clone()))
        .or_else(|| agent.and_then(|agent| agent.display_agent.clone()))
        .or_else(|| pane.terminal_title.clone())
        .or_else(|| {
            let cwd = pane.foreground_cwd.as_deref().or(pane.cwd.as_deref())?;
            std::path::Path::new(cwd)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "shell".to_owned());
    let status = agent
        .filter(|agent| agent.agent.is_some())
        .map(|agent| agent.agent_status);
    (status, label)
}

fn put_new_tab_button(buffer: &mut Buffer, rect: Rect, config: &ClientShellConfig) {
    put_text(
        buffer,
        rect.x,
        rect.y,
        rect.width,
        NEW_TAB_LABEL,
        Style::default()
            .fg(config.palette.overlay1)
            .bg(config.palette.panel_bg),
    );
}
