//! Queue edits: moving songs, playing one next, clearing. Each keeps
//! `current` (the playing song's index) pointing at the same song.

use crate::model::Track;

/// Moves the song at `from` one place up (`delta` -1) or down (+1).
/// Returns where it ended up.
pub fn move_song(queue: &mut [Track], current: &mut Option<usize>, from: usize, delta: isize) -> usize {
    let Some(to) = from.checked_add_signed(delta).filter(|&to| to < queue.len()) else {
        return from;
    };
    queue.swap(from, to);
    // The playing song moves with the swap, wherever it was.
    *current = current.map(|c| {
        if c == from {
            to
        } else if c == to {
            from
        } else {
            c
        }
    });
    to
}

/// Puts `track` right after the playing song (or at the end when nothing
/// is playing). Returns its index.
pub fn insert_next(queue: &mut Vec<Track>, current: Option<usize>, track: Track) -> usize {
    let at = current.map_or(queue.len(), |c| c + 1);
    queue.insert(at, track);
    at
}

/// Drops everything but the playing song.
pub fn keep_only_current(queue: &mut Vec<Track>, current: &mut Option<usize>) {
    match current.and_then(|c| queue.get(c).cloned()) {
        Some(playing) => {
            *queue = vec![playing];
            *current = Some(0);
        }
        None => queue.clear(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(ids: &[&str]) -> Vec<Track> {
        ids.iter()
            .map(|id| Track {
                video_id: (*id).into(),
                ..Track::default()
            })
            .collect()
    }

    fn ids(queue: &[Track]) -> Vec<&str> {
        queue.iter().map(|t| t.video_id.as_str()).collect()
    }

    #[test]
    fn moving_keeps_the_playing_song_tracked() {
        let mut q = queue(&["a", "b", "c"]);
        let mut current = Some(1);
        assert_eq!(move_song(&mut q, &mut current, 1, 1), 2, "move the playing song down");
        assert_eq!((ids(&q), current), (vec!["a", "c", "b"], Some(2)));
        assert_eq!(
            move_song(&mut q, &mut current, 0, 1),
            1,
            "move another song past nothing playing"
        );
        assert_eq!((ids(&q), current), (vec!["c", "a", "b"], Some(2)));
        assert_eq!(move_song(&mut q, &mut current, 1, 1), 2, "swap with the playing song");
        assert_eq!((ids(&q), current), (vec!["c", "b", "a"], Some(1)));
    }

    #[test]
    fn moving_past_the_ends_does_nothing() {
        let mut q = queue(&["a", "b"]);
        let mut current = None;
        assert_eq!(move_song(&mut q, &mut current, 0, -1), 0);
        assert_eq!(move_song(&mut q, &mut current, 1, 1), 1);
        assert_eq!(ids(&q), vec!["a", "b"]);
    }

    #[test]
    fn play_next_goes_right_after_the_current_song() {
        let mut q = queue(&["a", "b", "c"]);
        let new = |id: &str| Track {
            video_id: id.into(),
            ..Track::default()
        };
        assert_eq!(insert_next(&mut q, Some(0), new("x")), 1);
        assert_eq!(ids(&q), vec!["a", "x", "b", "c"]);
        assert_eq!(insert_next(&mut q, None, new("y")), 4, "nothing playing: the end");
    }

    #[test]
    fn clearing_keeps_only_the_playing_song() {
        let mut q = queue(&["a", "b", "c"]);
        let mut current = Some(2);
        keep_only_current(&mut q, &mut current);
        assert_eq!((ids(&q), current), (vec!["c"], Some(0)));
        let mut current = None;
        keep_only_current(&mut q, &mut current);
        assert!(q.is_empty());
    }
}
