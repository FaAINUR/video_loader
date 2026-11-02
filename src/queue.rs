//! Планирование очереди без запуска внешних процессов.
use crate::jobs::DownloadItem;
use crate::models::Status;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct QueueState {
    pub paused: bool,
}

#[derive(Clone, Copy, Debug)]
pub enum Move {
    Up,
    Down,
    Next,
}

pub fn next_waiting(items: &[DownloadItem], state: &QueueState) -> Option<usize> {
    if state.paused
        || items
            .iter()
            .any(|i| i.running.is_some() || i.status == Status::Downloading)
    {
        return None;
    }
    items.iter().position(|i| i.status == Status::Waiting)
}

pub fn neighbors(items: &[DownloadItem], id: u64) -> (bool, bool) {
    let waiting: Vec<u64> = items
        .iter()
        .filter(|i| i.status == Status::Waiting)
        .map(|i| i.id)
        .collect();
    match waiting.iter().position(|value| *value == id) {
        Some(index) => (index > 0, index + 1 < waiting.len()),
        None => (false, false),
    }
}

/// Меняем только ожидающие слоты. История и запущенные задачи не перемещаются.
pub fn move_waiting(items: &mut [DownloadItem], id: u64, movement: Move) -> bool {
    let slots: Vec<usize> = items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.status == Status::Waiting)
        .map(|(index, _)| index)
        .collect();
    let Some(position) = slots.iter().position(|index| items[*index].id == id) else {
        return false;
    };
    match movement {
        Move::Up if position > 0 => items.swap(slots[position], slots[position - 1]),
        Move::Down if position + 1 < slots.len() => {
            items.swap(slots[position], slots[position + 1])
        }
        Move::Next if position > 0 => {
            for p in (1..=position).rev() {
                items.swap(slots[p], slots[p - 1]);
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{restore_items, saved_jobs};
    use crate::models::sample_spec;

    fn item(id: u64, status: Status) -> DownloadItem {
        let mut i = DownloadItem::new(id, sample_spec(), "auto".into());
        i.status = status;
        i
    }
    fn queue() -> Vec<DownloadItem> {
        vec![
            item(1, Status::Completed),
            item(2, Status::Waiting),
            item(3, Status::Downloading),
            item(4, Status::Waiting),
            item(5, Status::Failed),
            item(6, Status::Waiting),
        ]
    }
    fn ids(items: &[DownloadItem]) -> Vec<u64> {
        items.iter().map(|i| i.id).collect()
    }

    #[test]
    fn up_skips_nonwaiting() {
        let mut q = queue();
        assert!(move_waiting(&mut q, 4, Move::Up));
        assert_eq!(ids(&q), vec![1, 4, 3, 2, 5, 6]);
    }
    #[test]
    fn down_skips_nonwaiting() {
        let mut q = queue();
        assert!(move_waiting(&mut q, 2, Move::Down));
        assert_eq!(ids(&q), vec![1, 4, 3, 2, 5, 6]);
    }
    #[test]
    fn next_preserves_other_order() {
        let mut q = queue();
        assert!(move_waiting(&mut q, 6, Move::Next));
        assert_eq!(ids(&q), vec![1, 6, 3, 2, 5, 4]);
    }
    #[test]
    fn boundaries_do_nothing() {
        let mut q = queue();
        let before = ids(&q);
        assert!(!move_waiting(&mut q, 2, Move::Up));
        assert!(!move_waiting(&mut q, 6, Move::Down));
        assert!(!move_waiting(&mut q, 2, Move::Next));
        assert_eq!(ids(&q), before);
    }
    #[test]
    fn nonwaiting_and_unknown_are_ignored() {
        let mut q = queue();
        let before = ids(&q);
        for id in [1, 3, 5, 999] {
            assert!(!move_waiting(&mut q, id, Move::Next));
        }
        assert_eq!(ids(&q), before);
    }
    #[test]
    fn neighbor_flags() {
        let q = queue();
        assert_eq!(neighbors(&q, 2), (false, true));
        assert_eq!(neighbors(&q, 4), (true, true));
        assert_eq!(neighbors(&q, 6), (true, false));
        assert_eq!(neighbors(&q, 3), (false, false));
    }
    #[test]
    fn paused_does_not_schedule() {
        let q = vec![item(1, Status::Waiting)];
        assert_eq!(next_waiting(&q, &QueueState { paused: true }), None);
        assert_eq!(next_waiting(&q, &QueueState::default()), Some(0));
    }
    #[test]
    fn active_download_blocks_next() {
        assert_eq!(next_waiting(&queue(), &QueueState::default()), None);
    }
    #[test]
    fn empty_and_single_queues() {
        assert_eq!(next_waiting(&[], &QueueState::default()), None);
        let mut q = vec![item(1, Status::Waiting)];
        assert!(!move_waiting(&mut q, 1, Move::Next));
        assert_eq!(neighbors(&q, 1), (false, false));
    }
    #[test]
    fn order_and_pause_round_trip() {
        let mut q = queue();
        move_waiting(&mut q, 6, Move::Next);
        let jobs_json = serde_json::to_string(&saved_jobs(&q, 200, 10)).unwrap();
        let (mut restored, _) = restore_items(serde_json::from_str(&jobs_json).unwrap());
        assert_eq!(ids(&restored), ids(&q));
        let state = QueueState { paused: true };
        let state: QueueState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        for i in &mut restored {
            if i.status == Status::Interrupted {
                i.resume();
            }
        }
        assert_eq!(next_waiting(&restored, &state), None);
        assert_eq!(next_waiting(&restored, &QueueState::default()), Some(1));
    }
    #[test]
    fn missing_pause_defaults_to_false() {
        let state: QueueState = serde_json::from_str("{}").unwrap();
        assert!(!state.paused);
    }
}
